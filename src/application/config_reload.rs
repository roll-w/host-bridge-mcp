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

use crate::application::execution_service::{ExecutionRuntimeConfig, ExecutionService};
use crate::application::operator_console::{OperatorConsole, PreparedLoggingReconfigure};
use crate::application::shutdown_controller::ShutdownController;
use crate::config::{AppConfig, LoggingConfig, ResolvedConfigPath};
use crate::transport::mcp_streamable_http::{RequestAuthController, RequestAuthState};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

const CONFIG_RELOAD_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigFileFingerprint {
    exists: bool,
    content_hash: Option<u64>,
}

struct PreparedReload {
    config: AppConfig,
    fingerprint: ConfigFileFingerprint,
    prepared_actions: Vec<Box<dyn PreparedConfigReloadAction>>,
}

struct ConfigReloadContext {
    participants: Vec<Box<dyn ConfigReloadParticipant>>,
}

pub(crate) trait ConfigReloadParticipant: Send + Sync {
    fn prepare(
        &self,
        config: &AppConfig,
        applied_logging: &LoggingConfig,
    ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError>;
}

pub(crate) trait PreparedConfigReloadAction {
    fn apply(self: Box<Self>);
}

struct PreparedAuthReload {
    auth_controller: RequestAuthController,
    auth_state: RequestAuthState,
}

struct PreparedExecutionReload {
    execution_service: ExecutionService,
    execution_runtime: Arc<ExecutionRuntimeConfig>,
}

struct PreparedLoggingReload {
    operator_console: OperatorConsole,
    logging_reconfigure: PreparedLoggingReconfigure,
}

struct ConfigReloadState {
    running_bind_address: String,
    applied_logging: LoggingConfig,
    last_applied_fingerprint: ConfigFileFingerprint,
    last_failed_fingerprint: Option<ConfigFileFingerprint>,
}

struct ConfigFileWatcher {
    _watcher: RecommendedWatcher,
    receiver: UnboundedReceiver<notify::Result<Event>>,
    config_path: PathBuf,
    watched_directory: PathBuf,
}

enum WatchLoopExit {
    Shutdown,
    WatcherClosed,
}

pub(crate) fn spawn_config_reloader(
    config_path: ResolvedConfigPath,
    initial_config: AppConfig,
    participants: Vec<Box<dyn ConfigReloadParticipant>>,
    shutdown_controller: ShutdownController,
) {
    tokio::spawn(async move {
        let mut state = ConfigReloadState::new(&config_path, initial_config);
        let reload_context = ConfigReloadContext::new(participants);

        match ConfigFileWatcher::try_new(&config_path) {
            Ok(watcher) => {
                tracing::info!(
                    path = %config_path.path,
                    directory = %watcher.watched_directory.display(),
                    "Watching config file for changes"
                );

                if matches!(
                    watch_config_events(
                        &config_path,
                        &reload_context,
                        &shutdown_controller,
                        &mut state,
                        watcher,
                    )
                    .await,
                    WatchLoopExit::Shutdown
                ) {
                    return;
                }

                tracing::warn!(
                    path = %config_path.path,
                    "Config watcher stopped unexpectedly; falling back to polling"
                );
            }
            Err(error) => {
                tracing::warn!(
                    path = %config_path.path,
                    error = %error,
                    "Failed to initialize config watcher; falling back to polling"
                );
            }
        }

        watch_config_poll_loop(
            &config_path,
            &reload_context,
            &shutdown_controller,
            &mut state,
            CONFIG_RELOAD_FALLBACK_POLL_INTERVAL,
        )
            .await;
    });
}

async fn watch_config_events(
    config_path: &ResolvedConfigPath,
    reload_context: &ConfigReloadContext,
    shutdown_controller: &ShutdownController,
    state: &mut ConfigReloadState,
    mut watcher: ConfigFileWatcher,
) -> WatchLoopExit {
    loop {
        tokio::select! {
            _ = shutdown_controller.wait_for_shutdown() => {
                return WatchLoopExit::Shutdown;
            }
            maybe_event = watcher.receiver.recv() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if event_targets_config_path(
                            &event,
                            &watcher.config_path,
                            &watcher.watched_directory,
                        ) {
                            tracing::info!(
                                path = %config_path.path,
                                event = ?event.kind,
                                "Config file change detected"
                            );
                            state.reload_if_needed(config_path, reload_context);
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(
                            path = %config_path.path,
                            error = %error,
                            "Config watcher reported an error"
                        );
                    }
                    None => {
                        return WatchLoopExit::WatcherClosed;
                    }
                }
            }
        }
    }
}

async fn watch_config_poll_loop(
    config_path: &ResolvedConfigPath,
    reload_context: &ConfigReloadContext,
    shutdown_controller: &ShutdownController,
    state: &mut ConfigReloadState,
    poll_interval: Duration,
) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown_controller.wait_for_shutdown() => {
                return;
            }
            _ = interval.tick() => {
                state.reload_if_needed(config_path, reload_context);
            }
        }
    }
}

impl ConfigReloadContext {
    fn new(participants: Vec<Box<dyn ConfigReloadParticipant>>) -> Self {
        Self { participants }
    }

    fn prepare_reload(
        &self,
        config_path: &ResolvedConfigPath,
        fingerprint: ConfigFileFingerprint,
        applied_logging: &LoggingConfig,
    ) -> Result<PreparedReload, ReloadPrepareError> {
        PreparedReload::prepare(
            config_path,
            fingerprint,
            applied_logging,
            &self.participants,
        )
    }
}

impl ConfigReloadState {
    fn new(config_path: &ResolvedConfigPath, initial_config: AppConfig) -> Self {
        let last_applied_fingerprint = match ConfigFileFingerprint::capture(&config_path.path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(
                    path = %config_path.path,
                    error = %error,
                    "Failed to inspect config file for hot reload; starting with a missing fingerprint"
                );
                ConfigFileFingerprint::missing()
            }
        };

        Self {
            running_bind_address: initial_config.server.bind_address,
            applied_logging: initial_config.logging,
            last_applied_fingerprint,
            last_failed_fingerprint: None,
        }
    }

    fn reload_if_needed(
        &mut self,
        config_path: &ResolvedConfigPath,
        reload_context: &ConfigReloadContext,
    ) {
        let next_fingerprint = match ConfigFileFingerprint::capture(&config_path.path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(
                    path = %config_path.path,
                    error = %error,
                    "Failed to inspect config file for hot reload; keeping previous configuration"
                );
                return;
            }
        };

        if next_fingerprint == self.last_applied_fingerprint {
            self.last_failed_fingerprint = None;
            return;
        }

        let mut prepared = match reload_context.prepare_reload(
            config_path,
            next_fingerprint.clone(),
            &self.applied_logging,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                if self.last_failed_fingerprint.as_ref() != Some(&next_fingerprint) {
                    tracing::error!(
                        path = %config_path.path,
                        error = %error,
                        "Failed to reload config file; keeping previous configuration"
                    );
                    self.last_failed_fingerprint = Some(next_fingerprint);
                }
                return;
            }
        };

        prepared.apply_actions();

        self.applied_logging = prepared.config.logging.clone();
        self.last_applied_fingerprint = prepared.fingerprint;
        self.last_failed_fingerprint = None;

        if prepared.config.server.bind_address != self.running_bind_address {
            tracing::warn!(
                active_address = %self.running_bind_address,
                configured_address = %prepared.config.server.bind_address,
                "Config reloaded, but server.address changes require a restart"
            );
        } else {
            tracing::info!(path = %config_path.path, "Config file reloaded");
        }
    }
}

impl ConfigFileWatcher {
    fn try_new(config_path: &ResolvedConfigPath) -> notify::Result<Self> {
        let config_path = normalize_watch_path_lexically(Path::new(&config_path.path));
        let watched_directory = watched_directory_for_config_path(&config_path);
        let (sender, receiver) = unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |result| {
            let _ = sender.send(result);
        })?;
        watcher.watch(&watched_directory, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver,
            config_path,
            watched_directory,
        })
    }
}

impl ConfigReloadParticipant for RequestAuthController {
    fn prepare(
        &self,
        config: &AppConfig,
        _applied_logging: &LoggingConfig,
    ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError> {
        let auth_state = RequestAuthController::prepare(&config.server.access)
            .map_err(ReloadPrepareError::Auth)?;

        Ok(Some(Box::new(PreparedAuthReload {
            auth_controller: self.clone(),
            auth_state,
        })))
    }
}

impl ConfigReloadParticipant for ExecutionService {
    fn prepare(
        &self,
        config: &AppConfig,
        _applied_logging: &LoggingConfig,
    ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError> {
        Ok(Some(Box::new(PreparedExecutionReload {
            execution_service: self.clone(),
            execution_runtime: ExecutionService::prepare_runtime(config),
        })))
    }
}

impl ConfigReloadParticipant for OperatorConsole {
    fn prepare(
        &self,
        config: &AppConfig,
        applied_logging: &LoggingConfig,
    ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError> {
        if &config.logging == applied_logging {
            return Ok(None);
        }

        let Some(logging_reconfigure) = self
            .prepare_logging_reconfigure(config.logging.clone())
            .map_err(ReloadPrepareError::Logging)?
        else {
            return Ok(None);
        };

        Ok(Some(Box::new(PreparedLoggingReload {
            operator_console: self.clone(),
            logging_reconfigure,
        })))
    }
}

impl PreparedConfigReloadAction for PreparedAuthReload {
    fn apply(self: Box<Self>) {
        let Self {
            auth_controller,
            auth_state,
        } = *self;
        auth_controller.apply(auth_state);
    }
}

impl PreparedConfigReloadAction for PreparedExecutionReload {
    fn apply(self: Box<Self>) {
        let Self {
            execution_service,
            execution_runtime,
        } = *self;
        execution_service.apply_runtime(execution_runtime);
    }
}

impl PreparedConfigReloadAction for PreparedLoggingReload {
    fn apply(self: Box<Self>) {
        let Self {
            operator_console,
            logging_reconfigure,
        } = *self;
        operator_console.apply_logging_reconfigure(logging_reconfigure);
    }
}

impl PreparedReload {
    fn apply_actions(&mut self) {
        for prepared_action in std::mem::take(&mut self.prepared_actions) {
            prepared_action.apply();
        }
    }
}

impl PreparedReload {
    fn prepare(
        config_path: &ResolvedConfigPath,
        fingerprint: ConfigFileFingerprint,
        applied_logging: &LoggingConfig,
        participants: &[Box<dyn ConfigReloadParticipant>],
    ) -> Result<Self, ReloadPrepareError> {
        let config =
            AppConfig::load_from_resolved_path(config_path).map_err(ReloadPrepareError::Config)?;
        let mut prepared_actions = Vec::with_capacity(participants.len());

        for participant in participants {
            if let Some(prepared_action) = participant.prepare(&config, applied_logging)? {
                prepared_actions.push(prepared_action);
            }
        }

        Ok(Self {
            config,
            fingerprint,
            prepared_actions,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReloadPrepareError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Auth(#[from] crate::transport::mcp_streamable_http::TransportAuthError),
    #[error(transparent)]
    Logging(#[from] std::io::Error),
}

impl ConfigFileFingerprint {
    fn capture(path: &str) -> std::io::Result<Self> {
        match std::fs::read(Path::new(path)) {
            Ok(contents) => {
                let mut hasher = DefaultHasher::new();
                contents.hash(&mut hasher);

                Ok(Self {
                    exists: true,
                    content_hash: Some(hasher.finish()),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::missing()),
            Err(error) => Err(error),
        }
    }

    fn missing() -> Self {
        Self {
            exists: false,
            content_hash: None,
        }
    }
}

fn watched_directory_for_config_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn event_targets_config_path(event: &Event, config_path: &Path, watched_directory: &Path) -> bool {
    let config_path = normalize_watch_path_lexically(config_path);
    let watched_directory = normalize_watch_path_lexically(watched_directory);
    let target_file_name = config_path.file_name();

    event.paths.iter().any(|path| {
        let path = normalize_watch_path_lexically(path);
        path == config_path
            || (path.parent() == Some(watched_directory.as_path())
            && path.file_name().is_some()
            && same_file_name(path.file_name(), target_file_name))
    })
}

fn normalize_watch_path_lexically(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(current_directory) = std::env::current_dir() {
        current_directory.join(path)
    } else {
        path.to_path_buf()
    };

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(segment) => normalized.push(segment),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn same_file_name(left: Option<&OsStr>, right: Option<&OsStr>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::execution_service::{ExecuteCommandInput, ExecutionError};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestReloadParticipant {
        name: &'static str,
        trace: Arc<Mutex<Vec<String>>>,
    }

    struct TestPreparedReloadAction {
        name: &'static str,
        trace: Arc<Mutex<Vec<String>>>,
    }

    struct FailingReloadParticipant;

    impl ConfigReloadParticipant for TestReloadParticipant {
        fn prepare(
            &self,
            _config: &AppConfig,
            _applied_logging: &LoggingConfig,
        ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError> {
            self.trace
                .lock()
                .expect("trace should be lockable")
                .push(format!("prepare:{}", self.name));

            Ok(Some(Box::new(TestPreparedReloadAction {
                name: self.name,
                trace: Arc::clone(&self.trace),
            })))
        }
    }

    impl PreparedConfigReloadAction for TestPreparedReloadAction {
        fn apply(self: Box<Self>) {
            let Self { name, trace } = *self;
            trace
                .lock()
                .expect("trace should be lockable")
                .push(format!("apply:{name}"));
        }
    }

    impl ConfigReloadParticipant for FailingReloadParticipant {
        fn prepare(
            &self,
            _config: &AppConfig,
            _applied_logging: &LoggingConfig,
        ) -> Result<Option<Box<dyn PreparedConfigReloadAction>>, ReloadPrepareError> {
            Err(ReloadPrepareError::Logging(std::io::Error::other(
                "reload failure",
            )))
        }
    }

    fn write_temp_config(contents: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("host-bridge-reload-{unique}.yaml"));
        fs::write(&path, contents).expect("temp config should be written");
        path
    }

    fn temp_config_path(path: &Path) -> ResolvedConfigPath {
        ResolvedConfigPath {
            path: path.display().to_string(),
            explicit: true,
        }
    }

    #[tokio::test]
    async fn applies_reloaded_execution_settings_from_new_config_snapshot() {
        let config_path = write_temp_config(
            r#"server:
  address: 127.0.0.1:8787
logging:
  memory-buffer-lines: 10
execution:
  default-action: allow
"#,
        );
        let resolved_path = temp_config_path(&config_path);
        let initial_config =
            AppConfig::load_from_resolved_path(&resolved_path).expect("initial config should load");
        let service = ExecutionService::new(Arc::new(initial_config.clone()));
        let auth_controller =
            RequestAuthController::new(&initial_config.server.access).expect("auth should load");
        let console = OperatorConsole::new(initial_config.logging.clone())
            .expect("console should initialize");
        let reload_context = ConfigReloadContext::new(vec![
            Box::new(console.clone()),
            Box::new(auth_controller.clone()),
            Box::new(service.clone()),
        ]);
        let mut state = ConfigReloadState::new(&resolved_path, initial_config);

        let initial_prepare = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                server: None,
                working_directory: None,
                env: HashMap::new(),
                timeout_ms: None,
            })
            .await
            .expect("command should prepare before reload");
        assert!(initial_prepare.confirmation_request().is_none());

        fs::write(
            &config_path,
            r#"server:
  address: 127.0.0.1:9999
logging:
  memory-buffer-lines: 10
execution:
  default-action: deny
"#,
        )
            .expect("updated config should be written");

        state.reload_if_needed(&resolved_path, &reload_context);

        let result = service
            .prepare_command(ExecuteCommandInput {
                command: "cargo build".to_string(),
                server: None,
                working_directory: None,
                env: HashMap::new(),
                timeout_ms: None,
            })
            .await;
        assert!(matches!(result, Err(ExecutionError::Denied)));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn fingerprint_marks_missing_file() {
        let missing = ConfigFileFingerprint::capture("definitely-missing-host-bridge-config.yaml")
            .expect("missing config fingerprint should resolve");

        assert_eq!(missing, ConfigFileFingerprint::missing());
    }

    #[test]
    fn watched_directory_uses_parent_directory() {
        let config_path = PathBuf::from("configs/host-bridge.yaml");

        assert_eq!(
            watched_directory_for_config_path(&config_path),
            PathBuf::from("configs")
        );
    }

    #[test]
    fn reload_context_prepares_and_applies_participants_in_order() {
        let config_path = write_temp_config(
            r#"server:
  address: 127.0.0.1:8787
logging:
  memory-buffer-lines: 10
execution:
  default-action: allow
"#,
        );
        let resolved_path = temp_config_path(&config_path);
        let initial_config =
            AppConfig::load_from_resolved_path(&resolved_path).expect("config should load");
        let state = ConfigReloadState::new(&resolved_path, initial_config);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let first = TestReloadParticipant {
            name: "first",
            trace: Arc::clone(&trace),
        };
        let second = TestReloadParticipant {
            name: "second",
            trace: Arc::clone(&trace),
        };
        let reload_context = ConfigReloadContext::new(vec![Box::new(first), Box::new(second)]);
        let fingerprint =
            ConfigFileFingerprint::capture(&resolved_path.path).expect("fingerprint should load");

        let mut prepared = reload_context
            .prepare_reload(&resolved_path, fingerprint, &state.applied_logging)
            .expect("reload preparation should succeed");

        assert_eq!(
            trace.lock().expect("trace should be lockable").as_slice(),
            ["prepare:first".to_string(), "prepare:second".to_string()]
        );

        prepared.apply_actions();

        assert_eq!(
            trace.lock().expect("trace should be lockable").as_slice(),
            [
                "prepare:first".to_string(),
                "prepare:second".to_string(),
                "apply:first".to_string(),
                "apply:second".to_string(),
            ]
        );

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn reload_if_needed_clears_failed_fingerprint_when_config_returns_to_applied_state() {
        let config_path = write_temp_config(
            r#"server:
  address: 127.0.0.1:8787
logging:
  memory-buffer-lines: 10
execution:
  default-action: allow
"#,
        );
        let resolved_path = temp_config_path(&config_path);
        let initial_config =
            AppConfig::load_from_resolved_path(&resolved_path).expect("config should load");
        let mut state = ConfigReloadState::new(&resolved_path, initial_config);
        let reload_context = ConfigReloadContext::new(vec![Box::new(FailingReloadParticipant)]);
        let failing_contents = r#"server:
  address: 127.0.0.1:8787
logging:
  memory-buffer-lines: 20
execution:
  default-action: deny
"#;

        fs::write(&config_path, failing_contents).expect("failing config should be written");
        let failing_fingerprint = ConfigFileFingerprint::capture(&resolved_path.path)
            .expect("failing fingerprint should load");

        state.reload_if_needed(&resolved_path, &reload_context);
        assert_eq!(
            state.last_failed_fingerprint,
            Some(failing_fingerprint.clone())
        );

        fs::write(
            &config_path,
            r#"server:
  address: 127.0.0.1:8787
logging:
  memory-buffer-lines: 10
execution:
  default-action: allow
"#,
        )
            .expect("initial config should be restored");

        state.reload_if_needed(&resolved_path, &reload_context);
        assert_eq!(state.last_failed_fingerprint, None);

        fs::write(&config_path, failing_contents).expect("failing config should be rewritten");

        state.reload_if_needed(&resolved_path, &reload_context);
        assert_eq!(state.last_failed_fingerprint, Some(failing_fingerprint));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn normalize_watch_path_expands_relative_paths_from_current_directory() {
        let current_directory =
            std::env::current_dir().expect("current directory should be available");

        assert_eq!(
            normalize_watch_path_lexically(Path::new("./configs/../host-bridge.yaml")),
            current_directory.join("host-bridge.yaml")
        );
    }

    #[test]
    fn event_targets_config_path_matches_absolute_event_for_relative_config_path() {
        let current_directory =
            std::env::current_dir().expect("current directory should be available");
        let watched_directory = PathBuf::from(".");
        let config_path = PathBuf::from("host-bridge.yaml");
        let event =
            Event::new(notify::EventKind::Any).add_path(current_directory.join("host-bridge.yaml"));

        assert!(event_targets_config_path(
            &event,
            &config_path,
            &watched_directory,
        ));
    }

    #[test]
    fn event_targets_config_path_matches_target_file_name_in_watched_directory() {
        let watched_directory = PathBuf::from("configs");
        let config_path = watched_directory.join("host-bridge.yaml");
        let event = Event::new(notify::EventKind::Any)
            .add_path(watched_directory.join("tmp.yaml"))
            .add_path(config_path.clone());

        assert!(event_targets_config_path(
            &event,
            &config_path,
            &watched_directory,
        ));
    }

    #[test]
    fn event_targets_config_path_ignores_other_files() {
        let watched_directory = PathBuf::from("configs");
        let config_path = watched_directory.join("host-bridge.yaml");
        let event =
            Event::new(notify::EventKind::Any).add_path(watched_directory.join("other.yaml"));

        assert!(!event_targets_config_path(
            &event,
            &config_path,
            &watched_directory,
        ));
    }

    #[test]
    fn logging_reconfiguration_swaps_log_path() {
        let initial_path =
            std::env::temp_dir().join(format!("host-bridge-log-a-{}.log", uuid::Uuid::new_v4()));
        let next_path =
            std::env::temp_dir().join(format!("host-bridge-log-b-{}.log", uuid::Uuid::new_v4()));
        let console = OperatorConsole::new(LoggingConfig {
            memory_buffer_lines: 2,
            file_path: Some(initial_path.display().to_string()),
            persist_file: true,
        })
            .expect("console should initialize");

        console
            .reconfigure_logging(LoggingConfig {
                memory_buffer_lines: 2,
                file_path: Some(next_path.display().to_string()),
                persist_file: true,
            })
            .expect("logging should reconfigure");

        assert_eq!(
            console.snapshot().log_file_path,
            next_path.display().to_string()
        );

        let _ = fs::remove_file(initial_path);
        let _ = fs::remove_file(next_path);
    }
}
