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

use crate::application::command_parser::{CommandParseError, parse_command_line};
use crate::application::data_dir::execution_output_path;
use crate::config::AppConfig;
use crate::domain::execution_target::{
    ExecutionEnvironmentSummary, ExecutionTarget, ExecutionTargetRegistry, ExecutionTransport,
};
use crate::domain::platform::spawn::SpawnPlanner;
use crate::domain::policy::{PolicyDecision, PolicyEngine};
use crate::domain::ssh::{SshClient, SshCommandRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self as std_io, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::time::Duration;
use uuid::Uuid;

mod runner;
#[cfg(test)]
mod tests;

use self::runner::run_execution;

const EXECUTION_RECORD_RETENTION: Duration = Duration::from_secs(5 * 60);
const TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error(transparent)]
    Parse(#[from] CommandParseError),
    #[error("command execution is denied by policy")]
    Denied,
    #[error("unknown execution server '{0}'")]
    UnknownServer(String),
    #[error("timeoutMs must be greater than zero")]
    InvalidTimeout,
    #[error("failed to initialize execution output store: {0}")]
    OutputStore(String),
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
    pub server: Option<String>,
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
    pub server: String,
    pub platform: String,
    pub command_line: String,
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    Status {
        state: ExecutionState,
        message: Option<String>,
    },
    Output {
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

#[derive(Debug, Clone)]
pub struct PreparedExecution {
    run: RunExecution,
    confirmation_request: Option<ConfirmationRequest>,
}

#[derive(Clone)]
pub struct ExecutionService {
    config: Arc<AppConfig>,
    policy_engine: Arc<PolicyEngine>,
    targets: Arc<ExecutionTargetRegistry>,
    spawn_planner: SpawnPlanner,
    ssh_client: Arc<SshClient>,
    records: Arc<RwLock<HashMap<Uuid, Arc<ExecutionRecord>>>>,
}

#[derive(Debug, Clone)]
struct RunExecution {
    command_line: String,
    server_name: String,
    backend: RunExecutionBackend,
}

#[derive(Debug, Clone)]
enum RunExecutionBackend {
    Host(HostRunExecution),
    Ssh(SshRunExecution),
}

#[derive(Debug, Clone)]
struct HostRunExecution {
    program: String,
    args: Vec<String>,
    working_directory: PathBuf,
    env: HashMap<String, String>,
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
struct SshRunExecution {
    target: crate::domain::execution_target::SshTarget,
    platform: crate::domain::platform::runtime::RuntimePlatform,
    request: SshCommandRequest,
}

struct ExecutionRecord {
    sender: broadcast::Sender<ExecutionEvent>,
    state: Mutex<ExecutionState>,
    output_store: StdMutex<RawOutputStore>,
}

struct RawOutputStore {
    path: PathBuf,
    file: Option<File>,
}

impl PreparedExecution {
    pub fn confirmation_request(&self) -> Option<&ConfirmationRequest> {
        self.confirmation_request.as_ref()
    }
}

impl ExecutionRecord {
    fn new(execution_id: Uuid) -> Result<Self, ExecutionError> {
        let path = execution_output_path(execution_id)
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))?;
        Self::with_output_path(path)
    }

    fn with_output_path(path: PathBuf) -> Result<Self, ExecutionError> {
        let (sender, _) = broadcast::channel(1_024);
        Ok(Self {
            sender,
            state: Mutex::new(ExecutionState::Running),
            output_store: StdMutex::new(RawOutputStore::new(path)?),
        })
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

    fn append_output(&self, text: &str) -> Result<(), ExecutionError> {
        let mut output_store = self
            .output_store
            .lock()
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))?;
        output_store
            .append(text)
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))
    }

    fn read_output(&self) -> Result<String, ExecutionError> {
        self.output_store
            .lock()
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))?
            .read_all()
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))
    }

    fn close_output_store(&self) -> Result<(), ExecutionError> {
        self.output_store
            .lock()
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))?
            .close()
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))
    }
}

impl RawOutputStore {
    fn new(path: PathBuf) -> Result<Self, ExecutionError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|error| ExecutionError::OutputStore(error.to_string()))?;
            }
        }

        let file = open_private_output_file(&path)
            .map_err(|error| ExecutionError::OutputStore(error.to_string()))?;

        Ok(Self {
            path,
            file: Some(file),
        })
    }

    fn append(&mut self, text: &str) -> std_io::Result<()> {
        if self.file.is_none() {
            self.file = Some(open_output_file_for_append(&self.path)?);
        }

        match self.file.as_mut() {
            Some(file) => file.write_all(text.as_bytes()),
            None => Ok(()),
        }
    }

    fn read_all(&mut self) -> std_io::Result<String> {
        self.close()?;
        fs::read_to_string(&self.path)
    }

    fn close(&mut self) -> std_io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }

        Ok(())
    }
}

impl Drop for RawOutputStore {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl ExecutionService {
    pub fn new(config: Arc<AppConfig>) -> Self {
        let policy_engine = PolicyEngine::new((*config).clone());
        let targets = ExecutionTargetRegistry::from_config(&config.execution);

        Self {
            config,
            policy_engine: Arc::new(policy_engine),
            targets: Arc::new(targets),
            spawn_planner: SpawnPlanner::current(),
            ssh_client: Arc::new(SshClient::new()),
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn default_server_name(&self) -> &str {
        &self.targets.default_target().name
    }

    pub fn available_environments(&self) -> Vec<ExecutionEnvironmentSummary> {
        self.targets.environments()
    }

    pub async fn prepare_command(
        &self,
        input: ExecuteCommandInput,
    ) -> Result<PreparedExecution, ExecutionError> {
        let parsed = parse_command_line(&input.command)?;
        let policy = self.policy_engine.evaluate(&parsed.program, &parsed.args);

        if policy.decision == PolicyDecision::Deny {
            tracing::warn!(command = %input.command, "Policy denied command");
            return Err(ExecutionError::Denied);
        }

        let target = self.resolve_target(input.server.as_deref())?;
        let timeout_ms = self.resolve_timeout(input.timeout_ms)?;
        let executable = target.path_mapper.map_command_if_path(&parsed.program);
        let args = parsed
            .args
            .iter()
            .map(|argument| target.path_mapper.map_argument_if_path(argument))
            .collect::<Vec<_>>();

        let (backend, preview_working_directory) = match &target.transport {
            ExecutionTransport::Host => {
                let working_directory = self.resolve_local_working_directory(
                    target,
                    input.working_directory.as_deref(),
                    policy.default_working_directory.as_deref(),
                )?;
                (
                    RunExecutionBackend::Host(HostRunExecution {
                        program: executable.clone(),
                        args: args.clone(),
                        working_directory: working_directory.clone(),
                        env: input.env.clone(),
                        timeout_ms,
                    }),
                    Some(working_directory.display().to_string()),
                )
            }
            ExecutionTransport::Ssh(ssh_target) => {
                let remote_working_directory = self.resolve_remote_working_directory(
                    target,
                    input.working_directory.as_deref(),
                    policy.default_working_directory.as_deref(),
                );
                (
                    RunExecutionBackend::Ssh(SshRunExecution {
                        target: ssh_target.clone(),
                        platform: target.target_platform,
                        request: SshCommandRequest {
                            executable: executable.clone(),
                            args: args.clone(),
                            env: input.env.clone(),
                            working_directory: remote_working_directory.clone(),
                            timeout_ms,
                        },
                    }),
                    remote_working_directory,
                )
            }
        };

        let confirmation_request =
            (policy.decision == PolicyDecision::RequireConfirmation).then(|| ConfirmationRequest {
                server: target.name.clone(),
                platform: target.target_platform.as_name().to_string(),
                command_line: input.command.clone(),
                executable: executable.clone(),
                args: args.clone(),
                working_directory: preview_working_directory.clone(),
                timeout_ms,
                env: input.env.clone(),
            });

        Ok(PreparedExecution {
            run: RunExecution {
                command_line: input.command,
                server_name: target.name.clone(),
                backend,
            },
            confirmation_request,
        })
    }

    pub async fn launch_prepared_command(
        &self,
        prepared: PreparedExecution,
    ) -> Result<(ExecutionLaunch, broadcast::Receiver<ExecutionEvent>), ExecutionError> {
        let execution_id = Uuid::new_v4();
        let command_line = prepared.run.command_line.clone();
        let record = Arc::new(ExecutionRecord::new(execution_id)?);
        let receiver = record.subscribe();

        self.records
            .write()
            .await
            .insert(execution_id, record.clone());
        self.spawn_execution(execution_id, record, prepared.run)
            .await;

        tracing::info!(
            execution_id = %execution_id,
            command = %command_line,
            "Execution submitted"
        );

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

    pub async fn read_output(&self, execution_id: Uuid) -> Result<String, ExecutionError> {
        let record = self
            .records
            .read()
            .await
            .get(&execution_id)
            .cloned()
            .ok_or(ExecutionError::NotFound(execution_id))?;

        record.read_output()
    }

    async fn spawn_execution(
        &self,
        execution_id: Uuid,
        record: Arc<ExecutionRecord>,
        run: RunExecution,
    ) {
        let records = self.records.clone();
        let spawn_planner = self.spawn_planner.clone();
        let ssh_client = self.ssh_client.clone();
        tokio::spawn(async move {
            run_execution(execution_id, record, run, spawn_planner, ssh_client).await;
            tokio::time::sleep(EXECUTION_RECORD_RETENTION).await;
            records.write().await.remove(&execution_id);
        });
    }

    fn resolve_timeout(&self, requested_timeout_ms: Option<u64>) -> Result<u64, ExecutionError> {
        let timeout_ms = requested_timeout_ms.unwrap_or(self.config.execution.default_timeout_ms);
        if timeout_ms == 0 {
            return Err(ExecutionError::InvalidTimeout);
        }
        Ok(timeout_ms.min(self.config.execution.max_timeout_ms))
    }

    fn resolve_target(
        &self,
        requested_server: Option<&str>,
    ) -> Result<&ExecutionTarget, ExecutionError> {
        self.targets.resolve(requested_server).ok_or_else(|| {
            ExecutionError::UnknownServer(requested_server.unwrap_or_default().to_string())
        })
    }

    fn resolve_local_working_directory(
        &self,
        target: &ExecutionTarget,
        requested: Option<&str>,
        default_from_policy: Option<&str>,
    ) -> Result<PathBuf, ExecutionError> {
        let current_directory = env::current_dir()
            .map_err(|error| ExecutionError::InvalidWorkingDirectory(error.to_string()))?;

        let Some(raw_path) = self.selected_working_directory(requested, default_from_policy) else {
            return Ok(current_directory);
        };

        let mapped = target.path_mapper.map_path(&raw_path);
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

    fn resolve_remote_working_directory(
        &self,
        target: &ExecutionTarget,
        requested: Option<&str>,
        default_from_policy: Option<&str>,
    ) -> Option<String> {
        self.selected_working_directory(requested, default_from_policy)
            .map(|path| target.path_mapper.map_path(&path))
    }

    fn selected_working_directory(
        &self,
        requested: Option<&str>,
        default_from_policy: Option<&str>,
    ) -> Option<String> {
        requested
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                default_from_policy
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .map(ToOwned::to_owned)
    }
}

fn open_private_output_file(path: &Path) -> std_io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);

    #[cfg(unix)]
    options.mode(0o600);

    options.open(path)
}

fn open_output_file_for_append(path: &Path) -> std_io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).write(true);
    options.open(path)
}
