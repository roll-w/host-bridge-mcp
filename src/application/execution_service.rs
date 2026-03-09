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
use crate::application::data_dir::execution_output_path;
use crate::config::AppConfig;
use crate::domain::execution_target::{
    ExecutionTarget, ExecutionTargetRegistry, ExecutionTransport,
};
use crate::domain::platform::spawn::{apply_spawn_plan, SpawnPlanner};
use crate::domain::policy::{PolicyDecision, PolicyEngine};
use crate::domain::ssh::build_ssh_invocation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self as std_io, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

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
    records: Arc<RwLock<HashMap<Uuid, Arc<ExecutionRecord>>>>,
}

#[derive(Debug, Clone)]
struct RunExecution {
    command_line: String,
    program: String,
    args: Vec<String>,
    working_directory: PathBuf,
    env: HashMap<String, String>,
    timeout_ms: u64,
    server_name: String,
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
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn command_environment_name(&self) -> &'static str {
        self.targets.default_target().target_platform.as_name()
    }

    pub fn default_server_name(&self) -> &str {
        &self.targets.default_target().name
    }

    pub fn available_server_names(&self) -> Vec<String> {
        self.targets.target_names()
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

        let (program, program_args, working_directory, preview_working_directory, local_env) =
            match &target.transport {
                ExecutionTransport::Host => {
                    let working_directory = self.resolve_local_working_directory(
                        target,
                        input.working_directory.as_deref(),
                        policy.default_working_directory.as_deref(),
                    )?;
                    (
                        executable.clone(),
                        args.clone(),
                        working_directory.clone(),
                        Some(working_directory.display().to_string()),
                        input.env.clone(),
                    )
                }
                ExecutionTransport::Ssh(ssh_target) => {
                    let remote_working_directory = self.resolve_remote_working_directory(
                        target,
                        input.working_directory.as_deref(),
                        policy.default_working_directory.as_deref(),
                    );
                    let invocation = build_ssh_invocation(
                        ssh_target,
                        target.target_platform,
                        &executable,
                        &args,
                        &input.env,
                        remote_working_directory.as_deref(),
                    );
                    (
                        invocation.program,
                        invocation.args,
                        env::current_dir().map_err(|error| {
                            ExecutionError::InvalidWorkingDirectory(error.to_string())
                        })?,
                        remote_working_directory,
                        HashMap::new(),
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
                program,
                args: program_args,
                working_directory,
                env: local_env,
                timeout_ms,
                server_name: target.name.clone(),
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
        tokio::spawn(async move {
            run_execution(execution_id, record, run, spawn_planner).await;
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

async fn run_execution(
    execution_id: Uuid,
    record: Arc<ExecutionRecord>,
    run: RunExecution,
    spawn_planner: SpawnPlanner,
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

    if let Err(error) = record.close_output_store() {
        tracing::error!(
            execution_id = %execution_id,
            error = %error,
            "Failed to close execution output store"
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CommandPolicyConfig, CommandRuleConfig, ExecutionConfig, ExecutionServerConfig,
        PathMappingRule, PolicyAction, TargetPlatform,
    };

    fn test_config(execution: ExecutionConfig) -> Arc<AppConfig> {
        let mut config = AppConfig::default();
        config.execution = execution;
        Arc::new(config)
    }

    #[test]
    fn reject_zero_timeout() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Allow,
            ..ExecutionConfig::default()
        });
        let service = ExecutionService::new(config);
        let result = service.resolve_timeout(Some(0));
        assert!(matches!(result, Err(ExecutionError::InvalidTimeout)));
    }

    #[tokio::test]
    async fn prepare_command_marks_confirmation_when_policy_requires_it() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            commands: vec![CommandPolicyConfig {
                command: "cargo".to_string(),
                default_working_directory: None,
                action: PolicyAction::Confirm,
                rules: vec![CommandRuleConfig {
                    args_prefix: vec!["build".to_string()],
                    action: PolicyAction::Confirm,
                    default_working_directory: None,
                }],
            }],
            ..ExecutionConfig::default()
        });

        let service = ExecutionService::new(config);
        let prepared = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                server: None,
                working_directory: None,
                env: HashMap::new(),
                timeout_ms: None,
            })
            .await
            .expect("command should prepare");

        let confirmation = prepared
            .confirmation_request()
            .expect("confirmation should exist");
        assert_eq!(confirmation.server, "host");
        assert_eq!(confirmation.platform, service.command_environment_name());
    }

    #[tokio::test]
    async fn prepare_command_rejects_unknown_server() {
        let service = ExecutionService::new(test_config(ExecutionConfig {
            default_action: PolicyAction::Allow,
            ..ExecutionConfig::default()
        }));

        let error = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                server: Some("missing".to_string()),
                working_directory: None,
                env: HashMap::new(),
                timeout_ms: None,
            })
            .await
            .expect_err("unknown server should fail");

        assert!(matches!(error, ExecutionError::UnknownServer(name) if name == "missing"));
    }

    #[tokio::test]
    async fn prepare_command_builds_ssh_invocation_for_remote_server() {
        let service = ExecutionService::new(test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 2222,
                user: Some("deploy".to_string()),
                target_platform: TargetPlatform::Linux,
                enable_builtin_wsl_mapping: false,
                path_mappings: vec![PathMappingRule {
                    from: "/workspace".to_string(),
                    to: "/srv/workspace".to_string(),
                    platforms: Vec::new(),
                }],
                identity_file: Some("/home/dev/.ssh/id_ed25519".to_string()),
                known_hosts_file: Some("/home/dev/.ssh/known_hosts".to_string()),
            }],
            ..ExecutionConfig::default()
        }));

        let prepared = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                server: Some("prod".to_string()),
                working_directory: Some("/workspace/app".to_string()),
                env: HashMap::from([("RUST_LOG".to_string(), "debug".to_string())]),
                timeout_ms: Some(5_000),
            })
            .await
            .expect("remote command should prepare");

        let confirmation = prepared
            .confirmation_request()
            .expect("confirmation should exist");
        assert_eq!(confirmation.server, "prod");
        assert_eq!(confirmation.platform, "linux");
        assert_eq!(
            confirmation.working_directory.as_deref(),
            Some("/srv/workspace/app")
        );
        assert_eq!(prepared.run.program, "ssh");
        assert!(prepared.run.args.contains(&"-p".to_string()));
        assert!(prepared.run.args.contains(&"2222".to_string()));
        assert!(
            prepared
                .run
                .args
                .contains(&"deploy@prod.example.com".to_string())
        );
        let remote_command = prepared
            .run
            .args
            .last()
            .expect("remote command should be present");
        assert!(remote_command.contains("/srv/workspace/app"));
        assert!(remote_command.contains("RUST_LOG=debug"));
        assert!(remote_command.contains("cargo"));
    }

    #[test]
    fn command_environment_name_returns_resolved_platform() {
        let config = test_config(ExecutionConfig {
            target_platform: TargetPlatform::Windows,
            ..ExecutionConfig::default()
        });
        let service = ExecutionService::new(config);

        assert_eq!(service.command_environment_name(), "windows");
    }

    #[test]
    fn execution_record_preserves_merged_output_order() {
        let path =
            std::env::temp_dir().join(format!("host-bridge-mcp-test-{}.log", Uuid::new_v4()));
        let record =
            ExecutionRecord::with_output_path(path.clone()).expect("record should initialize");

        record
            .append_output("first\n")
            .expect("first write should succeed");
        record
            .append_output("second\n")
            .expect("second write should succeed");

        assert_eq!(
            record.read_output().expect("output should be readable"),
            "first\nsecond\n"
        );

        drop(record);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn execution_output_file_is_named_from_execution_id() {
        let execution_id =
            Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").expect("uuid should parse");
        let path = execution_output_path(execution_id).expect("path should resolve");

        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("123e4567-e89b-12d3-a456-426614174000.log")
        );
    }

    #[test]
    fn execution_output_file_is_retained_after_record_drop() {
        let path =
            std::env::temp_dir().join(format!("host-bridge-mcp-output-{}.log", Uuid::new_v4()));
        {
            let record =
                ExecutionRecord::with_output_path(path.clone()).expect("record should initialize");
            record
                .append_output("persisted\n")
                .expect("output should be written");
        }

        assert!(path.exists());
        let _ = fs::remove_file(path);
    }
}
