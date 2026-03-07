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
use crate::application::operator_console::{ConsoleLogLevel, OperatorConsole};
use crate::config::AppConfig;
use crate::domain::path_mapping::PathMapper;
use crate::domain::policy::{PolicyDecision, PolicyEngine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error(transparent)]
    Parse(#[from] CommandParseError),
    #[error("command execution is denied by policy")]
    Denied,
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

#[derive(Debug, Clone)]
pub struct PreparedExecution {
    run: RunExecution,
    confirmation_request: Option<ConfirmationRequest>,
}

#[derive(Clone)]
pub struct ExecutionService {
    config: Arc<AppConfig>,
    policy_engine: Arc<PolicyEngine>,
    path_mapper: Arc<PathMapper>,
    operator_console: OperatorConsole,
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

impl PreparedExecution {
    pub fn confirmation_request(&self) -> Option<&ConfirmationRequest> {
        self.confirmation_request.as_ref()
    }
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
    pub fn new(config: Arc<AppConfig>, operator_console: OperatorConsole) -> Self {
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
            operator_console,
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn prepare_command(
        &self,
        input: ExecuteCommandInput,
    ) -> Result<PreparedExecution, ExecutionError> {
        let parsed = parse_command_line(&input.command)?;
        let policy = self.policy_engine.evaluate(&parsed.program, &parsed.args);

        if policy.decision == PolicyDecision::Deny {
            self.operator_console.push_log(
                ConsoleLogLevel::Warn,
                format!("Policy denied command: {}", input.command),
            );
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

        let confirmation_request =
            (policy.decision == PolicyDecision::RequireConfirmation).then(|| ConfirmationRequest {
                command_line: input.command.clone(),
                executable: executable.clone(),
                args: args.clone(),
                working_directory: working_directory.display().to_string(),
                timeout_ms,
                env: input.env.clone(),
            });

        Ok(PreparedExecution {
            run: RunExecution {
                command_line: input.command,
                executable,
                args,
                working_directory,
                env: input.env,
                timeout_ms,
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
        let record = Arc::new(ExecutionRecord::new());
        let receiver = record.subscribe();

        self.records
            .write()
            .await
            .insert(execution_id, record.clone());
        self.spawn_execution(execution_id, record, prepared.run)
            .await;

        self.operator_console.push_log(
            ConsoleLogLevel::Info,
            format!(
                "Execution submitted [{}]: {command_line}",
                short_id(execution_id)
            ),
        );
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

    async fn spawn_execution(
        &self,
        execution_id: Uuid,
        record: Arc<ExecutionRecord>,
        run: RunExecution,
    ) {
        let operator_console = self.operator_console.clone();
        tokio::spawn(async move {
            run_execution(execution_id, record, run, operator_console).await;
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
        let current_directory = env::current_dir()
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

async fn run_execution(
    execution_id: Uuid,
    record: Arc<ExecutionRecord>,
    run: RunExecution,
    operator_console: OperatorConsole,
) {
    emit_event(
        execution_id,
        &record,
        &operator_console,
        ExecutionEvent::Status {
            state: ExecutionState::Running,
            message: Some(format!("Executing: {}", run.command_line)),
        },
    );

    let resolved_executable = resolve_executable_for_spawn(&run.executable, &run.env);
    let mut command = Command::new(&resolved_executable);
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
            emit_event(
                execution_id,
                &record,
                &operator_console,
                ExecutionEvent::Error {
                    message: format!("Failed to spawn '{}': {error}", run.executable),
                },
            );
            emit_event(
                execution_id,
                &record,
                &operator_console,
                ExecutionEvent::Status {
                    state: ExecutionState::Failed,
                    message: Some("Execution failed before process start".to_string()),
                },
            );
            return;
        }
    };

    let stdout_task = spawn_output_task(
        execution_id,
        child.stdout.take(),
        OutputKind::Stdout,
        record.clone(),
        operator_console.clone(),
    );

    let stderr_task = spawn_output_task(
        execution_id,
        child.stderr.take(),
        OutputKind::Stderr,
        record.clone(),
        operator_console.clone(),
    );

    let wait_result = timeout(Duration::from_millis(run.timeout_ms), child.wait()).await;
    let (timed_out, status_result) = match wait_result {
        Ok(status_result) => (false, status_result),
        Err(_) => {
            let _ = child.start_kill();
            emit_event(
                execution_id,
                &record,
                &operator_console,
                ExecutionEvent::Error {
                    message: format!("Process timed out after {} ms", run.timeout_ms),
                },
            );
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
            emit_event(
                execution_id,
                &record,
                &operator_console,
                ExecutionEvent::Exit {
                    code,
                    success,
                    timed_out,
                },
            );
            emit_event(
                execution_id,
                &record,
                &operator_console,
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
                &operator_console,
                ExecutionEvent::Error {
                    message: format!("Failed while waiting for process: {error}"),
                },
            );
            emit_event(
                execution_id,
                &record,
                &operator_console,
                ExecutionEvent::Status {
                    state: ExecutionState::Failed,
                    message: Some("Execution failed while waiting for process".to_string()),
                },
            );
        }
    }
}

fn resolve_executable_for_spawn(
    executable: &str,
    environment: &HashMap<String, String>,
) -> PathBuf {
    if cfg!(windows) {
        return resolve_windows_executable_path(executable, environment)
            .unwrap_or_else(|| PathBuf::from(executable));
    }

    PathBuf::from(executable)
}

fn resolve_windows_executable_path(
    executable: &str,
    environment: &HashMap<String, String>,
) -> Option<PathBuf> {
    let executable_path = Path::new(executable);
    if executable_path.is_absolute() || executable.contains('/') || executable.contains('\\') {
        return resolve_path_candidate(executable_path, &windows_path_extensions(environment));
    }

    if executable_path.extension().is_some() {
        return Some(PathBuf::from(executable));
    }

    let path_value = resolved_env_var(environment, "PATH")?;
    let extensions = windows_path_extensions(environment);

    for directory in env::split_paths(&path_value) {
        let candidate = directory.join(executable);
        if let Some(resolved) = resolve_path_candidate(&candidate, &extensions) {
            return Some(resolved);
        }
    }

    None
}

fn resolve_path_candidate(path: &Path, extensions: &[String]) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }

    if path.extension().is_some() {
        return None;
    }

    for extension in extensions {
        for candidate in extension_candidates(path, extension) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn extension_candidates(path: &Path, extension: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut suffixes = vec![extension.to_string()];
    let lowercase = extension.to_ascii_lowercase();
    if lowercase != extension {
        suffixes.push(lowercase);
    }
    let uppercase = extension.to_ascii_uppercase();
    if uppercase != extension && uppercase != suffixes[0] {
        suffixes.push(uppercase);
    }

    for suffix in suffixes {
        candidates.push(PathBuf::from(format!("{}{}", path.display(), suffix)));
    }

    candidates
}

fn windows_path_extensions(environment: &HashMap<String, String>) -> Vec<String> {
    let Some(raw_extensions) = resolved_env_var(environment, "PATHEXT") else {
        return Vec::new();
    };

    raw_extensions
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            if extension.starts_with('.') {
                extension.to_string()
            } else {
                format!(".{extension}")
            }
        })
        .collect()
}

fn resolved_env_var(environment: &HashMap<String, String>, name: &str) -> Option<OsString> {
    if cfg!(windows) {
        if let Some((_, value)) = environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
        {
            return Some(OsString::from(value));
        }
    } else if let Some(value) = environment.get(name) {
        return Some(OsString::from(value));
    }

    env::var_os(name)
}

async fn stream_output<R>(
    execution_id: Uuid,
    reader: R,
    output_kind: OutputKind,
    record: Arc<ExecutionRecord>,
    operator_console: OperatorConsole,
) where
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
                emit_event(
                    execution_id,
                    &record,
                    &operator_console,
                    ExecutionEvent::Output {
                        stream: output_kind.clone(),
                        text,
                    },
                );
            }
            Err(error) => {
                emit_event(
                    execution_id,
                    &record,
                    &operator_console,
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
    output_kind: OutputKind,
    record: Arc<ExecutionRecord>,
    operator_console: OperatorConsole,
) -> Option<JoinHandle<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    reader.map(|reader| {
        tokio::spawn(async move {
            stream_output(execution_id, reader, output_kind, record, operator_console).await;
        })
    })
}

fn emit_event(
    execution_id: Uuid,
    record: &ExecutionRecord,
    operator_console: &OperatorConsole,
    event: ExecutionEvent,
) {
    log_execution_event(execution_id, operator_console, &event);
    record.send(event);
}

fn log_execution_event(
    execution_id: Uuid,
    operator_console: &OperatorConsole,
    event: &ExecutionEvent,
) {
    let execution_id = short_id(execution_id);
    match event {
        ExecutionEvent::Status { message, .. } => {
            if let Some(message) = message {
                operator_console
                    .push_log(ConsoleLogLevel::Info, format!("[{execution_id}] {message}"));
            }
        }
        ExecutionEvent::Output { stream, text } => {
            let stream_name = match stream {
                OutputKind::Stdout => "stdout",
                OutputKind::Stderr => "stderr",
            };
            let line = text.trim_end_matches(['\n', '\r']);
            operator_console.push_log(
                ConsoleLogLevel::Info,
                format!("[{execution_id}] {stream_name} | {line}"),
            );
        }
        ExecutionEvent::Exit {
            code,
            success,
            timed_out,
        } => {
            let level = if *success {
                ConsoleLogLevel::Info
            } else {
                ConsoleLogLevel::Warn
            };
            operator_console.push_log(
                level,
                format!(
                    "[{execution_id}] exit code={code} success={success} timed_out={timed_out}"
                ),
            );
        }
        ExecutionEvent::Error { message } => {
            operator_console.push_log(
                ConsoleLogLevel::Error,
                format!("[{execution_id}] {message}"),
            );
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
        CommandPolicyConfig, CommandRuleConfig, ExecutionConfig, LoggingConfig, PolicyAction,
        ServerConfig,
    };
    use std::fs;

    #[test]
    fn reject_zero_timeout() {
        let config = Arc::new(AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                default_action: PolicyAction::Allow,
                ..ExecutionConfig::default()
            },
            logging: LoggingConfig::default(),
        });
        let service = ExecutionService::new(config, OperatorConsole::default());
        let result = service.resolve_timeout(Some(0));
        assert!(matches!(result, Err(ExecutionError::InvalidTimeout)));
    }

    #[tokio::test]
    async fn prepare_command_marks_confirmation_when_policy_requires_it() {
        let config = Arc::new(AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
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
            },
            logging: LoggingConfig::default(),
        });

        let service = ExecutionService::new(config, OperatorConsole::default());
        let prepared = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                working_directory: None,
                env: HashMap::new(),
                timeout_ms: None,
            })
            .await
            .expect("command should prepare");

        assert!(prepared.confirmation_request().is_some());
    }

    #[test]
    fn resolve_windows_executable_uses_pathext_for_bare_command() {
        let sandbox = temp_sandbox("bare");
        let npm_cmd = sandbox.join("npm.cmd");
        fs::write(&npm_cmd, "").expect("test command file should be created");

        let environment = HashMap::from([
            ("PATH".to_string(), sandbox.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);

        let resolved = resolve_windows_executable_path("npm", &environment)
            .expect("resolver should find npm.cmd");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            npm_cmd.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn resolve_windows_executable_uses_pathext_for_explicit_path_without_extension() {
        let sandbox = temp_sandbox("path");
        let tool_prefix = sandbox.join("tool");
        let tool_cmd = sandbox.join("tool.cmd");
        fs::write(&tool_cmd, "").expect("test command file should be created");

        let environment = HashMap::from([("PATHEXT".to_string(), ".CMD".to_string())]);
        let resolved =
            resolve_windows_executable_path(&tool_prefix.display().to_string(), &environment)
                .expect("resolver should use PATHEXT for explicit path");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            tool_cmd.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    fn temp_sandbox(label: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "host-bridge-mcp-execution-service-{label}-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("temporary sandbox should be created");
        path
    }
}
