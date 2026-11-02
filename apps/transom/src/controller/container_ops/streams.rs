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
        ContainerCommand::Stats {
            tab_id,
            request_id,
            container,
            enabled,
            interval_ms,
        } => {
            if !enabled {
                cancel_task(state, ctx, tab_id, request_id);
                return;
            }
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                log::warn!("容器统计未能启动，容器: {container}，标签: {tab_id}");
                return;
            };
            log::info!("容器统计轮询已启动，容器: {container}，间隔: {interval_ms}ms");
            let event_tx = ctx.event_tx.clone();
            spawn_long(
                state,
                ctx,
                tab_id,
                request_id,
                Arc::clone(&cancel),
                async move {
                    // 采样档位由界面下发；下限 500ms，避免异常配置把引擎打满。
                    let interval = Duration::from_millis(interval_ms.max(500));
                    let mut ticker = tokio::time::interval(interval);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    let mut tick: u64 = 0;
                    loop {
                        ticker.tick().await;
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if tick == 0 {
                            log::info!("容器统计首帧采样开始，容器: {container}");
                        }
                        // 采样是外部 I/O：引擎挂起时必须超时退出，否则页签永远停在
                        // 「正在采样统计…」而没有任何反馈。
                        let sampled = tokio::time::timeout(
                            Duration::from_secs(8),
                            engine.container_stats(&container, &cancel),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            Err(transom_container::ContainerError::Http {
                                operation: "采样容器统计".to_string(),
                                message: "请求超时".to_string(),
                            })
                        });
                        match sampled {
                            Ok(mut stats) => {
                                // 宿主负载不在 Docker stats 里。只对「本机引擎」读宿主
                                // /proc/loadavg：容器归档接口不允许 /proc，远端引擎
                                // 读到的还是本机负载，会误导，因此直接留空。
                                if tick.is_multiple_of(5)
                                    && is_local_engine(&engine)
                                    && let Ok(Ok(text)) = tokio::time::timeout(
                                        Duration::from_secs(1),
                                        tokio::fs::read_to_string("/proc/loadavg"),
                                    )
                                    .await
                                {
                                    stats.host_load = parse_loadavg(text.as_bytes());
                                }
                                if tick == 0 {
                                    log::info!(
                                        "容器统计首帧已采样，容器: {container}，CPU: {:.1}%",
                                        stats.cpu_percent
                                    );
                                }
                                tick = tick.saturating_add(1);
                                log::debug!(
                                    "容器统计采样，容器: {}，第 {} 次，宿主负载: {:?}",
                                    container,
                                    tick,
                                    stats.host_load
                                );
                                let _ = event_tx.try_send(
                                    ContainerAppEvent::StatsUpdated {
                                        request_id,
                                        container: container.clone(),
                                        stats,
                                    }
                                    .into(),
                                );
                            }
                            Err(transom_container::ContainerError::Cancelled) => break,
                            Err(error) => {
                                send_failed(&event_tx, request_id, error).await;
                                break;
                            }
                        }
                    }
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
