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

use crate::application::command_parser::{parse_command_line, CommandParseError};
use crate::config::AppConfig;
use crate::domain::path_mapping::PathMapper;
use crate::domain::policy::{PolicyDecision, PolicyEngine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error(transparent)]
    Parse(#[from] CommandParseError),
    #[error("command execution is denied by policy")]
    Denied,
    #[error("command requires confirmation")]
    ConfirmationRequired(ConfirmationRequest),
    #[error("timeoutMs must be greater than zero")]
    InvalidTimeout,
    #[error("execution '{0}' not found")]
    NotFound(Uuid),
    #[error("invalid working directory: {0}")]
    InvalidWorkingDirectory(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteCommandInput {
    pub command: String,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationRequest {
    pub command_line: String,
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: String,
    pub timeout_ms: u64,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchState {
    Running,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionLaunch {
    pub execution_id: Uuid,
    pub state: LaunchState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    Status {
        state: ExecutionState,
        message: Option<String>,
    },
    Output {
        stream: OutputKind,
        text: String,
    },
    Exit {
        code: i32,
        success: bool,
        timed_out: bool,
    },
    Error {
        message: String,
    },
}

pub struct ExecutionSubscription {
    pub current_state: ExecutionState,
    pub receiver: broadcast::Receiver<ExecutionEvent>,
}

#[derive(Clone)]
pub struct ExecutionService {
    config: Arc<AppConfig>,
    policy_engine: Arc<PolicyEngine>,
    path_mapper: Arc<PathMapper>,
    records: Arc<RwLock<HashMap<Uuid, Arc<ExecutionRecord>>>>,
}

#[derive(Debug, Clone)]
struct RunExecution {
    command_line: String,
    executable: String,
    args: Vec<String>,
    working_directory: PathBuf,
    env: HashMap<String, String>,
    timeout_ms: u64,
}

struct ExecutionRecord {
    sender: broadcast::Sender<ExecutionEvent>,
    state: Mutex<ExecutionState>,
}

impl ExecutionRecord {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(1_024);
        Self {
            sender,
            state: Mutex::new(ExecutionState::Running),
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.sender.subscribe()
    }

    async fn get_state(&self) -> ExecutionState {
        self.state.lock().await.clone()
    }

    async fn set_state(&self, state: ExecutionState) {
        *self.state.lock().await = state;
    }

    fn send(&self, event: ExecutionEvent) {
        let _ = self.sender.send(event);
    }
}

impl ExecutionService {
    pub fn new(config: Arc<AppConfig>) -> Self {
        let policy_engine = PolicyEngine::new((*config).clone());
        let path_mapper = PathMapper::new(
            config.execution.path_mappings.clone(),
            config.execution.target_platform,
            config.execution.enable_builtin_wsl_mapping,
        );

        Self {
            config,
            policy_engine: Arc::new(policy_engine),
            path_mapper: Arc::new(path_mapper),
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn submit_command_stream(
        &self,
        input: ExecuteCommandInput,
    ) -> Result<(ExecutionLaunch, broadcast::Receiver<ExecutionEvent>), ExecutionError> {
        let parsed = parse_command_line(&input.command)?;
        let policy = self.policy_engine.evaluate(&parsed.program, &parsed.args);

        if policy.decision == PolicyDecision::Deny {
            return Err(ExecutionError::Denied);
        }

        let timeout_ms = self.resolve_timeout(input.timeout_ms)?;
        let working_directory = self.resolve_working_directory(
            input.working_directory.as_deref(),
            policy.default_working_directory.as_deref(),
        )?;

        let executable = self.path_mapper.map_command_if_path(&parsed.program);
        let args = parsed
            .args
            .iter()
            .map(|argument| self.path_mapper.map_argument_if_path(argument))
            .collect::<Vec<_>>();

        if policy.decision == PolicyDecision::RequireConfirmation {
            return Err(ExecutionError::ConfirmationRequired(ConfirmationRequest {
                command_line: input.command,
                executable,
                args,
                working_directory: working_directory.display().to_string(),
                timeout_ms,
                env: input.env,
            }));
        }

        let run = RunExecution {
            command_line: input.command,
            executable,
            args,
            working_directory,
            env: input.env,
            timeout_ms,
        };

        let execution_id = Uuid::new_v4();
        let record = Arc::new(ExecutionRecord::new());
        let receiver = record.subscribe();
        self.records
            .write()
            .await
            .insert(execution_id, record.clone());
        self.spawn_execution(record, run).await;

        tracing::info!(execution_id = %execution_id, "Execution submitted");

        Ok((
            ExecutionLaunch {
                execution_id,
                state: LaunchState::Running,
            },
            receiver,
        ))
    }

    pub async fn subscribe(
        &self,
        execution_id: Uuid,
    ) -> Result<ExecutionSubscription, ExecutionError> {
        let record = self
            .records
            .read()
            .await
            .get(&execution_id)
            .cloned()
            .ok_or(ExecutionError::NotFound(execution_id))?;

        Ok(ExecutionSubscription {
            current_state: record.get_state().await,
            receiver: record.subscribe(),
        })
    }

    async fn spawn_execution(&self, record: Arc<ExecutionRecord>, run: RunExecution) {
        tokio::spawn(async move {
            run_execution(record, run).await;
        });
    }

    fn resolve_timeout(&self, requested_timeout_ms: Option<u64>) -> Result<u64, ExecutionError> {
        let timeout_ms = requested_timeout_ms.unwrap_or(self.config.execution.default_timeout_ms);
        if timeout_ms == 0 {
            return Err(ExecutionError::InvalidTimeout);
        }
        Ok(timeout_ms.min(self.config.execution.max_timeout_ms))
    }

    fn resolve_working_directory(
        &self,
        requested: Option<&str>,
        default_from_policy: Option<&str>,
    ) -> Result<PathBuf, ExecutionError> {
        let current_directory = std::env::current_dir()
            .map_err(|error| ExecutionError::InvalidWorkingDirectory(error.to_string()))?;

        let selected = requested
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                default_from_policy
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            });

        let Some(raw_path) = selected else {
            return Ok(current_directory);
        };

        let mapped = self.path_mapper.map_path(raw_path);
        let candidate = PathBuf::from(mapped);
        let resolved = if candidate.is_relative() {
            current_directory.join(candidate)
        } else {
            candidate
        };

        if !resolved.exists() {
            return Err(ExecutionError::InvalidWorkingDirectory(format!(
                "path does not exist: {}",
                resolved.display()
            )));
        }

        if !resolved.is_dir() {
            return Err(ExecutionError::InvalidWorkingDirectory(format!(
                "path is not a directory: {}",
                resolved.display()
            )));
        }

        Ok(resolved)
    }
}

async fn run_execution(record: Arc<ExecutionRecord>, run: RunExecution) {
    record.send(ExecutionEvent::Status {
        state: ExecutionState::Running,
        message: Some(format!("Executing: {}", run.command_line)),
    });

    let mut command = Command::new(&run.executable);
    command
        .args(&run.args)
        .current_dir(&run.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    for (key, value) in &run.env {
        command.env(key, value);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            record.set_state(ExecutionState::Failed).await;
            record.send(ExecutionEvent::Error {
                message: format!("Failed to spawn '{}': {error}", run.executable),
            });
            record.send(ExecutionEvent::Status {
                state: ExecutionState::Failed,
                message: Some("Execution failed before process start".to_string()),
            });
            return;
        }
    };

    let stdout_task = child.stdout.take().map(|stdout| {
        let stream_record = record.clone();
        tokio::spawn(async move {
            stream_output(stdout, OutputKind::Stdout, stream_record).await;
        })
    });

    let stderr_task = child.stderr.take().map(|stderr| {
        let stream_record = record.clone();
        tokio::spawn(async move {
            stream_output(stderr, OutputKind::Stderr, stream_record).await;
        })
    });

    let wait_result = timeout(Duration::from_millis(run.timeout_ms), child.wait()).await;
    let (timed_out, status_result) = match wait_result {
        Ok(status_result) => (false, status_result),
        Err(_) => {
            let _ = child.start_kill();
            record.send(ExecutionEvent::Error {
                message: format!("Process timed out after {} ms", run.timeout_ms),
            });
            (true, child.wait().await)
        }
    };

    if let Some(task) = stdout_task {
        let _ = task.await;
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }

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
            record.send(ExecutionEvent::Exit {
                code,
                success,
                timed_out,
            });
            record.send(ExecutionEvent::Status {
                state: final_state,
                message: Some(format!("Process finished with code {code}")),
            });
        }
        Err(error) => {
            record.set_state(ExecutionState::Failed).await;
            record.send(ExecutionEvent::Error {
                message: format!("Failed while waiting for process: {error}"),
            });
            record.send(ExecutionEvent::Status {
                state: ExecutionState::Failed,
                message: Some("Execution failed while waiting for process".to_string()),
            });
        }
    }
}

async fn stream_output<R>(reader: R, output_kind: OutputKind, record: Arc<ExecutionRecord>)
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
                record.send(ExecutionEvent::Output {
                    stream: output_kind.clone(),
                    text,
                });
            }
            Err(error) => {
                record.send(ExecutionEvent::Error {
                    message: format!("Failed to read process output: {error}"),
                });
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutionConfig, PolicyAction, ServerConfig};

    #[test]
    fn reject_zero_timeout() {
        let config = Arc::new(AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                default_policy: PolicyAction::Allow,
                ..ExecutionConfig::default()
            },
        });
        let service = ExecutionService::new(config);
        let result = service.resolve_timeout(Some(0));
        assert!(matches!(result, Err(ExecutionError::InvalidTimeout)));
    }
}
