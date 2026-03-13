/*
 * Copyright 2026-present RollW
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use super::{
    ExecutionEvent, ExecutionRecord, ExecutionState, HostRunExecution, RunExecution,
    RunExecutionBackend, SshRunExecution, TERMINATION_GRACE_PERIOD,
};
use crate::domain::platform::spawn::{SpawnPlanner, apply_spawn_plan};
use crate::domain::ssh::SshClient;
use std::io as std_io;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};
use uuid::Uuid;

pub(super) async fn run_execution(
    execution_id: Uuid,
    record: Arc<ExecutionRecord>,
    run: RunExecution,
    spawn_planner: SpawnPlanner,
    ssh_client: Arc<SshClient>,
) {
    emit_event(
        execution_id,
        &record,
        ExecutionEvent::Status {
            state: ExecutionState::Running,
            message: Some(format!(
                "Executing on {}: {}",
                run.server_name, run.command_line
            )),
        },
    );

    match run.backend {
        RunExecutionBackend::Host(host_run) => {
            run_host_execution(execution_id, record, host_run, spawn_planner).await;
        }
        RunExecutionBackend::Ssh(ssh_run) => {
            run_ssh_execution(execution_id, record, ssh_run, ssh_client).await;
        }
    }
}

async fn run_host_execution(
    execution_id: Uuid,
    record: Arc<ExecutionRecord>,
    run: HostRunExecution,
    spawn_planner: SpawnPlanner,
) {
    let spawn_plan = spawn_planner.build(&run.program, &run.args, &run.env, &run.working_directory);
    let mut command = Command::new(&spawn_plan.program);
    command
        .args(&spawn_plan.args)
        .current_dir(&run.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    apply_spawn_plan(&mut command, &spawn_plan);

    for (key, value) in &run.env {
        command.env(key, value);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            record.set_state(ExecutionState::Failed).await;
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Error {
                    message: format!("Failed to spawn '{}': {error}", run.program),
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Status {
                    state: ExecutionState::Failed,
                    message: Some("Execution failed before process start".to_string()),
                },
            );
            return;
        }
    };

    let stdout_task = spawn_output_task(execution_id, child.stdout.take(), record.clone());
    let stderr_task = spawn_output_task(execution_id, child.stderr.take(), record.clone());
    let wait_result = timeout(Duration::from_millis(run.timeout_ms), child.wait()).await;
    let (timed_out, status_result) = match wait_result {
        Ok(status_result) => (false, status_result),
        Err(_) => {
            let _ = child.start_kill();
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Error {
                    message: format!("Process timed out after {} ms", run.timeout_ms),
                },
            );
            let waited_for_exit = timeout(TERMINATION_GRACE_PERIOD, child.wait()).await;
            match waited_for_exit {
                Ok(status_result) => (true, status_result),
                Err(_) => {
                    emit_event(
                        execution_id,
                        &record,
                        ExecutionEvent::Error {
                            message: format!(
                                "Process did not exit within {} ms after timeout",
                                TERMINATION_GRACE_PERIOD.as_millis()
                            ),
                        },
                    );
                    (
                        true,
                        Err(std_io::Error::new(
                            std_io::ErrorKind::TimedOut,
                            format!(
                                "process did not exit within {} ms after kill",
                                TERMINATION_GRACE_PERIOD.as_millis()
                            ),
                        )),
                    )
                }
            }
        }
    };

    drop(child);

    if let Some(task) = stdout_task {
        let _ = task.await;
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }

    finalize_output_store(execution_id, &record);

    match status_result {
        Ok(status) => {
            let success = status.success() && !timed_out;
            let code = status.code().unwrap_or(-1);
            let final_state = if success {
                ExecutionState::Completed
            } else {
                ExecutionState::Failed
            };

            record.set_state(final_state.clone()).await;
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Exit {
                    code,
                    success,
                    timed_out,
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Status {
                    state: final_state,
                    message: Some(format!("Process finished with code {code}")),
                },
            );
        }
        Err(error) => {
            record.set_state(ExecutionState::Failed).await;
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Error {
                    message: format!("Failed while waiting for process: {error}"),
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Status {
                    state: ExecutionState::Failed,
                    message: Some("Execution failed while waiting for process".to_string()),
                },
            );
        }
    }
}

async fn run_ssh_execution(
    execution_id: Uuid,
    record: Arc<ExecutionRecord>,
    run: SshRunExecution,
    ssh_client: Arc<SshClient>,
) {
    let output_record = record.clone();
    let SshRunExecution {
        target,
        platform,
        request,
    } = run;

    match ssh_client
        .execute_command(target, platform, request, move |text| {
            emit_event(
                execution_id,
                &output_record,
                ExecutionEvent::Output { text },
            );
        })
        .await
    {
        Ok(result) => {
            let final_state = if result.success {
                ExecutionState::Completed
            } else {
                ExecutionState::Failed
            };
            record.set_state(final_state.clone()).await;
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Exit {
                    code: result.code,
                    success: result.success,
                    timed_out: result.timed_out,
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Status {
                    state: final_state,
                    message: Some(format!("Process finished with code {}", result.code)),
                },
            );
        }
        Err(error) => {
            record.set_state(ExecutionState::Failed).await;
            let timed_out = matches!(error, crate::domain::ssh::SshError::Timeout(_));
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Error {
                    message: error.to_string(),
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Exit {
                    code: -1,
                    success: false,
                    timed_out,
                },
            );
            emit_event(
                execution_id,
                &record,
                ExecutionEvent::Status {
                    state: ExecutionState::Failed,
                    message: Some(if timed_out {
                        "Execution timed out while waiting for remote process".to_string()
                    } else {
                        "Execution failed while waiting for remote process".to_string()
                    }),
                },
            );
        }
    }

    finalize_output_store(execution_id, &record);
}

fn finalize_output_store(execution_id: Uuid, record: &ExecutionRecord) {
    if let Err(error) = record.close_output_store() {
        tracing::error!(
            execution_id = %execution_id,
            error = %error,
            "Failed to close execution output store"
        );
    }
}

async fn stream_output<R>(execution_id: Uuid, reader: R, record: Arc<ExecutionRecord>)
where
    R: AsyncRead + Unpin,
{
    let mut buffered_reader = BufReader::new(reader);
    let mut buffer = Vec::with_capacity(4_096);

    loop {
        buffer.clear();
        match buffered_reader.read_until(b'\n', &mut buffer).await {
            Ok(0) => return,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buffer).into_owned();
                emit_event(execution_id, &record, ExecutionEvent::Output { text });
            }
            Err(error) => {
                emit_event(
                    execution_id,
                    &record,
                    ExecutionEvent::Error {
                        message: format!("Failed to read process output: {error}"),
                    },
                );
                return;
            }
        }
    }
}

fn spawn_output_task<R>(
    execution_id: Uuid,
    reader: Option<R>,
    record: Arc<ExecutionRecord>,
) -> Option<JoinHandle<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    reader.map(|reader| {
        tokio::spawn(async move {
            stream_output(execution_id, reader, record).await;
        })
    })
}

fn emit_event(execution_id: Uuid, record: &ExecutionRecord, event: ExecutionEvent) {
    log_execution_event(execution_id, &event);
    if let ExecutionEvent::Output { text } = &event {
        if let Err(error) = record.append_output(text) {
            tracing::error!(
                execution_id = %execution_id,
                error = %error,
                "Failed to persist merged execution output"
            );
        }
    }
    record.send(event);
}

fn log_execution_event(execution_id: Uuid, event: &ExecutionEvent) {
    let execution_id = short_id(execution_id);
    match event {
        ExecutionEvent::Status { message, .. } => {
            if let Some(message) = message {
                tracing::info!("[{execution_id}] {message}");
            }
        }
        ExecutionEvent::Output { text } => {
            let line = text.trim_end_matches(['\n', '\r']);
            tracing::info!("[{execution_id}] output | {line}");
        }
        ExecutionEvent::Exit {
            code,
            success,
            timed_out,
        } => {
            if *success {
                tracing::info!(
                    "[{execution_id}] exit code={code} success={success} timed_out={timed_out}"
                );
            } else {
                tracing::warn!(
                    "[{execution_id}] exit code={code} success={success} timed_out={timed_out}"
                );
            }
        }
        ExecutionEvent::Error { message } => {
            tracing::error!("[{execution_id}] {message}");
        }
    }
}

fn short_id(execution_id: Uuid) -> String {
    execution_id.to_string().chars().take(8).collect()
}
