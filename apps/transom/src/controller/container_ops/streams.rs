//! 日志、统计、进程与 exec 长任务。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::Cancel { tab_id, request_id } => {
            cancel_task(state, ctx, tab_id, request_id)
        }
        ContainerCommand::ExecTerminal {
            tab_id,
            request_id,
            options,
            size,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let outcome = transom_container::open_exec(&engine, &options, size).await;
                match outcome {
                    Ok((io, exit)) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::ExecReady {
                                tab_id,
                                request_id,
                                io: Box::new(io),
                                exit,
                            },
                        )
                        .await;
                    }
                    Err(error) => send_failed(&event_tx, request_id, error).await,
                }
            });
        }
        ContainerCommand::Logs {
            tab_id,
            request_id,
            container,
            tail,
            follow,
            timestamps,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(
                state,
                ctx,
                tab_id,
                request_id,
                Arc::clone(&cancel),
                async move {
                    let mut sink = |bytes: Vec<u8>| {
                        let _ = event_tx
                            .try_send(ContainerAppEvent::LogsChunk { request_id, bytes }.into());
                    };
                    let result = transom_container::stream_logs(
                        &engine, &container, tail, follow, timestamps, &cancel, &mut sink,
                    )
                    .await;
                    match result {
                        Ok(()) | Err(transom_container::ContainerError::Cancelled) => {}
                        Err(error) => send_failed(&event_tx, request_id, error).await,
                    }
                    send_event(&event_tx, ContainerAppEvent::LogsClosed { request_id }).await;
                },
            );
        }
        ContainerCommand::ProjectLogs {
            tab_id,
            request_id,
            project: _,
            containers,
            tail,
            follow,
            timestamps,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(
                state,
                ctx,
                tab_id,
                request_id,
                Arc::clone(&cancel),
                async move {
                    let mut sink = |bytes: Vec<u8>| {
                        let _ = event_tx
                            .try_send(ContainerAppEvent::LogsChunk { request_id, bytes }.into());
                    };
                    let result = transom_container::stream_project_logs(
                        &engine, containers, tail, follow, timestamps, &cancel, &mut sink,
                    )
                    .await;
                    match result {
                        Ok(()) | Err(transom_container::ContainerError::Cancelled) => {}
                        Err(error) => send_failed(&event_tx, request_id, error).await,
                    }
                    send_event(&event_tx, ContainerAppEvent::LogsClosed { request_id }).await;
                },
            );
        }
        ContainerCommand::Stats {
            tab_id,
            request_id,
            container,
            enabled,
            interval_ms: _,
        } => {
            if !enabled {
                cancel_task(state, ctx, tab_id, request_id);
                return;
            }
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                log::warn!("容器统计未能启动，容器: {container}，标签: {tab_id}");
                return;
            };
            log::info!("容器统计流已启动，容器: {container}");
            let event_tx = ctx.event_tx.clone();
            spawn_long(
                state,
                ctx,
                tab_id,
                request_id,
                Arc::clone(&cancel),
                async move {
                    stream_stats_with_reconnect(engine, container, cancel, event_tx, request_id)
                        .await;
                },
            );
        }
        ContainerCommand::ListProcesses {
            tab_id,
            request_id,
            container,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.list_processes(&container).await },
            |request_id, processes| ContainerAppEvent::ProcessesListed {
                request_id,
                processes,
            },
        ),
        _ => {}
    }
}

/// 维护一条统计流：流断开后指数退避重连，短暂失败保留 UI 上一帧。
async fn stream_stats_with_reconnect(
    engine: Engine,
    container: String,
    cancel: Arc<AtomicBool>,
    event_tx: flume::Sender<transom_core::AppEvent>,
    request_id: ContainerRequestId,
) {
    let local_engine = is_local_engine(&engine);
    let mut failures = 0u32;
    let mut samples = 0u64;
    let mut cached_host_load: Option<(f32, f32, f32)> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let (stats_tx, mut stats_rx) = tokio::sync::watch::channel::<Option<ContainerStats>>(None);
        let stream_engine = engine.clone();
        let stream_container = container.clone();
        let stream_cancel = Arc::clone(&cancel);
        let mut stream_task = tokio::spawn(async move {
            let mut sink = |stats: ContainerStats| {
                let _ = stats_tx.send(Some(stats));
            };
            stream_engine
                .stream_container_stats(&stream_container, &stream_cancel, &mut sink)
                .await
        });

        let outcome = loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if cancel.load(Ordering::Relaxed) {
                        stream_task.abort();
                        let _ = stream_task.await;
                        return;
                    }
                }
                changed = stats_rx.changed() => {
                    if changed.is_err() {
                        continue;
                    }
                    let Some(mut stats) = stats_rx.borrow_and_update().clone() else {
                        continue;
                    };
                    failures = 0;
                    samples = samples.saturating_add(1);
                    if (samples == 1 || samples.is_multiple_of(5))
                        && local_engine
                        && let Ok(Ok(text)) = tokio::time::timeout(
                            Duration::from_secs(1),
                            tokio::fs::read_to_string("/proc/loadavg"),
                        )
                        .await
                    {
                        cached_host_load = parse_loadavg(text.as_bytes());
                    }
                    stats.host_load = cached_host_load;
                    let event = ContainerAppEvent::StatsUpdated {
                        request_id,
                        container: container.clone(),
                        stats,
                    };
                    if tokio::time::timeout(
                        Duration::from_secs(1),
                        send_event(&event_tx, event),
                    )
                    .await
                    .is_err()
                    {
                        log::debug!("容器统计事件投递超时，等待下一帧覆盖，请求: {request_id}");
                    }
                }
                result = &mut stream_task => break result,
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return;
        }
        failures = failures.saturating_add(1);
        let message = match outcome {
            Ok(Ok(())) => "统计流已结束".to_string(),
            Ok(Err(error)) => error.to_string(),
            Err(error) if error.is_panic() => "统计流任务异常退出".to_string(),
            Err(_) => "统计流任务被取消".to_string(),
        };
        log::warn!("容器统计流中断，容器: {container}，连续失败: {failures}，原因: {message}");
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            send_event(
                &event_tx,
                ContainerAppEvent::StatsUnavailable {
                    request_id,
                    error: message,
                },
            ),
        )
        .await;
        let backoff = Duration::from_secs(1u64 << failures.min(3));
        let mut remaining = backoff;
        while !remaining.is_zero() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let step = remaining.min(Duration::from_millis(250));
            tokio::time::sleep(step).await;
            remaining = remaining.saturating_sub(step);
        }
    }
}
