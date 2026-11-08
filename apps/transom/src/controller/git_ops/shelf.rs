//! Git 离线搁置项（Shelf）命令处理。
//!
//! 设计原理（Why）：
//! - 搁置项的补丁文本保存在应用数据目录的 JSON 中，读写都是阻塞文件 IO，
//!   统一交给 `spawn_blocking`，控制循环与界面线程都不被磁盘拖住；
//! - 导出补丁与丢弃工作区改动是两次 git 子进程，必须严格按序；补丁为空时
//!   既不创建空条目、也不丢弃工作区，避免用户勾选的改动被静默清掉；
//! - 失败只经 [`GitAppEvent::ShelfFailed`] 上报，由界面统一记录与提示，
//!   控制器不重复打印同一条错误日志。
//!
//! 代价与局限：同一项目的多条搁置操作并发时，存储层的「读取-修改-写回」
//! 没有跨任务锁；两次创建/删除请求若恰好交错，后写者可能覆盖前者的结果。
//! 界面按点击节奏发起操作，正常使用下不会触发；需要强一致时再引入按项目串行化。

use std::path::PathBuf;
use std::sync::Arc;

use super::super::state::{ControllerCtx, ControllerState};
use super::runner::{send_event, to_app_error};
use transom_core::{
    AppError, ErrorCode, GitAppEvent, GitCommand, GitProjectId, GitRequestId, GitShelfOperation,
    ShelfEntry,
};
use transom_git::{GitService, Repo};

/// 创建搁置项所需的业务输入。
///
/// 设计原理（Why）：名称、分支与文件清单共同表达一次「搁置意图」，
/// 收敛成结构体避免落盘步骤与事件构造各自摊开一长串位置参数。
struct ShelveRequest {
    /// 存储侧的项目 ID（搁置项 JSON 以它命名）。
    project_id: String,
    /// 仓库工作区根路径。
    repo_path: PathBuf,
    /// 搁置项标题（含界面本地化文案）。
    name: String,
    /// 搁置时所处的分支名（或 `HEAD`）。
    branch: String,
    /// 要抽离的工作区文件清单。
    files: Vec<String>,
}

/// 应用搁置项所需的业务输入。
struct ApplyRequest {
    /// 仓库工作区根路径。
    repo_path: PathBuf,
    /// 存储侧的项目 ID，用于 `pop` 时删除条目。
    project_id: String,
    /// 搁置项 ID。
    shelf_id: String,
    /// unified diff 补丁文本。
    patch: String,
    /// 应用成功后是否删除该条目。
    pop: bool,
}

/// 处理搁置项命令；命中搁置项命令返回 `true`，否则返回 `false`。
pub(super) fn handle_shelf_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: &GitCommand,
) -> bool {
    match command {
        GitCommand::LoadShelves {
            project,
            request_id,
        } => {
            spawn_load(state, ctx, project.id, *request_id);
            true
        }
        GitCommand::Shelve {
            project,
            request_id,
            name,
            branch,
            files,
        } => {
            spawn_shelve(
                state,
                ctx,
                project.id,
                *request_id,
                ShelveRequest {
                    project_id: project.id.to_string(),
                    repo_path: project.path.clone(),
                    name: name.clone(),
                    branch: branch.clone(),
                    files: files.clone(),
                },
            );
            true
        }
        GitCommand::ApplyShelf {
            project,
            request_id,
            shelf_id,
            patch,
            pop,
        } => {
            spawn_apply(
                state,
                ctx,
                project.id,
                *request_id,
                ApplyRequest {
                    repo_path: project.path.clone(),
                    project_id: project.id.to_string(),
                    shelf_id: shelf_id.clone(),
                    patch: patch.clone(),
                    pop: *pop,
                },
            );
            true
        }
        GitCommand::DropShelf {
            project,
            request_id,
            shelf_id,
        } => {
            spawn_drop(state, ctx, project.id, *request_id, shelf_id.clone());
            true
        }
        _ => false,
    }
}

/// 派发一次搁置项列表读取：登记取消标志，结果按登记口径丢弃。
fn spawn_load(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project_id: GitProjectId,
    request_id: GitRequestId,
) {
    let service = Arc::clone(&ctx.git);
    let event_tx = ctx.event_tx.clone();
    let flag = service.register(request_id);
    state.git_set.spawn(async move {
        let event = match load_shelves(project_id.to_string()).await {
            Ok(shelves) => GitAppEvent::ShelvesLoaded {
                project_id,
                request_id,
                shelves,
            },
            Err(error) => shelf_failed(project_id, request_id, GitShelfOperation::Load, error),
        };
        service.finish(request_id);
        if GitService::is_cancelled(&flag) {
            log::debug!("Git 搁置项读取已取消，结果被丢弃，项目: {project_id}");
            return;
        }
        send_event(&event_tx, event);
    });
}

/// 派发一次搁置项创建。
fn spawn_shelve(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project_id: GitProjectId,
    request_id: GitRequestId,
    request: ShelveRequest,
) {
    let event_tx = ctx.event_tx.clone();
    state.git_set.spawn(async move {
        let event = match shelve(request).await {
            Ok(entry) => GitAppEvent::Shelved {
                project_id,
                request_id,
                entry,
            },
            Err(error) => shelf_failed(project_id, request_id, GitShelfOperation::Shelve, error),
        };
        send_event(&event_tx, event);
    });
}

/// 派发一次搁置项应用。
fn spawn_apply(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project_id: GitProjectId,
    request_id: GitRequestId,
    request: ApplyRequest,
) {
    let event_tx = ctx.event_tx.clone();
    let shelf_id = request.shelf_id.clone();
    let pop = request.pop;
    state.git_set.spawn(async move {
        let event = match apply_shelf(request).await {
            Ok(cleanup_error) => GitAppEvent::ShelfApplied {
                project_id,
                request_id,
                shelf_id,
                pop,
                cleanup_error,
            },
            Err(error) => shelf_failed(project_id, request_id, GitShelfOperation::Apply, error),
        };
        send_event(&event_tx, event);
    });
}

/// 派发一次搁置项删除。
fn spawn_drop(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    project_id: GitProjectId,
    request_id: GitRequestId,
    shelf_id: String,
) {
    let event_tx = ctx.event_tx.clone();
    state.git_set.spawn(async move {
        let event = match remove_shelf(project_id.to_string(), shelf_id.clone()).await {
            Ok(()) => GitAppEvent::ShelfDropped {
                project_id,
                request_id,
                shelf_id,
            },
            Err(error) => shelf_failed(project_id, request_id, GitShelfOperation::Drop, error),
        };
        send_event(&event_tx, event);
    });
}

/// 读取本地搁置项列表（阻塞 IO 走专用线程池）。
async fn load_shelves(project_id: String) -> Result<Vec<ShelfEntry>, AppError> {
    tokio::task::spawn_blocking(move || transom_store::load_project_shelves(&project_id))
        .await
        .map_err(|error| blocking_error("读取搁置项", error))?
}

/// 导出补丁、落盘搁置项，再抽离工作区改动。
///
/// 顺序是数据安全的硬约束：先保存补丁后执行 `discard`。若反过来，
/// 一旦保存失败，工作区改动已被丢弃而补丁没落盘，用户没有任何恢复路径。
/// 当前顺序的最坏情况是「补丁在、改动也还在」，可以再次搁置或手动丢弃。
async fn shelve(request: ShelveRequest) -> Result<ShelfEntry, AppError> {
    let repo = Repo::at(request.repo_path);
    let patch = repo
        .export_patch(&request.files)
        .await
        .map_err(to_app_error)?;
    if patch.trim().is_empty() {
        // 未跟踪文件不进 `git diff`，勾选它们时补丁为空；此时若继续 discard
        // 只会删掉用户改动却不留下任何可恢复内容，必须在此终止并明确上报。
        return Err(AppError::with_detail(
            ErrorCode::GitCommandFailed,
            "所选取的改动没有可导出的补丁内容，未创建搁置项",
        ));
    }

    let now = chrono::Local::now();
    let entry = ShelfEntry {
        id: format!("shelf_{}", now.timestamp_millis()),
        name: request.name,
        branch: request.branch,
        created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        files: request.files.clone(),
        patch,
    };
    let project_id = request.project_id;
    let stored = entry.clone();
    tokio::task::spawn_blocking(move || {
        let mut shelves = transom_store::load_project_shelves(&project_id)?;
        shelves.insert(0, stored);
        transom_store::save_project_shelves(&project_id, &shelves)?;
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|error| blocking_error("保存搁置项", error))??;

    // 落盘成功后再抽离工作区改动。抽离失败不撤销已创建的搁置项：
    // 补丁仍可恢复，改动留在工作区是安全降级，记录告警即可。
    if let Err(error) = repo.discard(&request.files).await {
        log::warn!(
            "搁置项已落盘，但丢弃工作区改动失败，项目: {}，文件数: {}，原因: {error}",
            repo.root().display(),
            request.files.len()
        );
    }
    Ok(entry)
}

/// 应用补丁；`pop` 为真时再尝试删除条目。
///
/// 返回值区分两段结果：外层 `Err` 表示补丁应用失败；`Ok(Some(error))` 表示
/// 补丁已成功应用、但条目删除失败（条目仍在存储中，界面不能按整次失败处理）。
async fn apply_shelf(request: ApplyRequest) -> Result<Option<AppError>, AppError> {
    let repo = Repo::at(request.repo_path);
    repo.apply_patch(&request.patch)
        .await
        .map_err(to_app_error)?;
    if !request.pop {
        return Ok(None);
    }
    match remove_shelf(request.project_id, request.shelf_id).await {
        Ok(()) => Ok(None),
        Err(error) => {
            let detail = format!("补丁已应用，但删除搁置项失败，原因: {error}");
            Ok(Some(AppError::with_detail(error.code, detail)))
        }
    }
}

/// 从存储中删除一条搁置项。
///
/// 条目本就不存在时视为成功：删除的目标状态是「不存在」，重复删除不应报错；
/// 只有读写存储本身失败才上报，避免把幂等操作做成一次假故障。
async fn remove_shelf(project_id: String, shelf_id: String) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || {
        let mut shelves = transom_store::load_project_shelves(&project_id)?;
        let before = shelves.len();
        shelves.retain(|entry| entry.id != shelf_id);
        if shelves.len() != before {
            transom_store::save_project_shelves(&project_id, &shelves)?;
        }
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|error| blocking_error("删除搁置项", error))?
}

/// 构造搁置项失败事件。
fn shelf_failed(
    project_id: GitProjectId,
    request_id: GitRequestId,
    operation: GitShelfOperation,
    error: AppError,
) -> GitAppEvent {
    GitAppEvent::ShelfFailed {
        project_id,
        request_id,
        operation,
        error,
    }
}

/// 阻塞线程池任务异常结束（panic 或运行时关闭）统一收敛为可上报错误。
fn blocking_error(operation: &str, error: tokio::task::JoinError) -> AppError {
    AppError::with_detail(
        ErrorCode::Io,
        format!("{operation}的阻塞任务异常结束，原因: {error}"),
    )
}
