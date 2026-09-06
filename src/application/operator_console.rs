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

mod approvals;
mod log_store;
mod sanitize;

use self::approvals::{PendingApproval, PendingApprovalGuard, SessionApprovalKey};
use self::log_store::{LogStore, PreparedLogStore};
use self::sanitize::sanitize_console_text;
use crate::application::data_dir::DataDirectory;
use crate::application::execution_service::ConfirmationRequest;
use crate::config::LoggingConfig;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, oneshot};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime as TracingSystemTime};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLogEntry {
    pub timestamp: String,
    pub level: ConsoleLogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApprovalView {
    pub id: Uuid,
    pub execution_id: Uuid,
    pub request: ConfirmationRequest,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    ApproveOnce,
    ApproveForSession,
    Reject,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleSnapshot {
    pub interactive: bool,
    #[serde(skip_serializing)]
    pub total_log_count: usize,
    #[serde(skip_serializing)]
    pub log_file_path: String,
    pub pending_approvals: Vec<PendingApprovalView>,
}

fn current_console_timestamp() -> String {
    let timer = TracingSystemTime::default();
    let mut output = String::new();
    let mut writer = Writer::new(&mut output);

    if timer.format_time(&mut writer).is_err() {
        return "1970-01-01T00:00:00.000000Z".to_string();
    }

    output.truncate(output.trim_end().len());
    output
}

fn push_runtime_log(state: &mut ConsoleState, entry: ConsoleLogEntry) {
    if state.runtime_logs.len() >= state.log_store.buffer_limit() {
        state.runtime_logs.pop_front();
    }
    state.runtime_logs.push_back(entry);
}

#[derive(Debug, thiserror::Error)]
pub enum ConsoleApprovalError {
    #[error("interactive TUI is unavailable")]
    Unavailable,
    #[error("approval request was cancelled")]
    Cancelled,
}

#[derive(Clone)]
pub struct OperatorConsole {
    state: Arc<Mutex<ConsoleState>>,
}

pub(crate) struct PreparedLoggingReconfigure {
    prepared: PreparedLogStore,
}

struct ConsoleState {
    interactive: bool,
    log_store: LogStore,
    runtime_logs: VecDeque<ConsoleLogEntry>,
    runtime_log_sender: broadcast::Sender<ConsoleLogEntry>,
    pending_approvals: Vec<PendingApproval>,
    session_approval_grants: HashSet<SessionApprovalKey>,
}

impl OperatorConsole {
    pub fn new(logging: LoggingConfig) -> io::Result<Self> {
        let data_directory = DataDirectory::new(None)?;
        Self::with_data_directory(logging, data_directory)
    }

    pub fn with_data_directory(
        logging: LoggingConfig,
        data_directory: DataDirectory,
    ) -> io::Result<Self> {
        let (runtime_log_sender, _) = broadcast::channel(1_024);
        Ok(Self {
            state: Arc::new(Mutex::new(ConsoleState {
                interactive: false,
                log_store: LogStore::new(logging, &data_directory)?,
                runtime_logs: VecDeque::new(),
                runtime_log_sender,
                pending_approvals: Vec::new(),
                session_approval_grants: HashSet::new(),
            })),
        })
    }

    pub fn set_interactive(&self, interactive: bool) {
        self.state
            .lock()
            .expect("console lock poisoned")
            .interactive = interactive;
    }

    pub fn push_log(&self, level: ConsoleLogLevel, message: impl Into<String>) {
        self.push_log_inner(level, message.into(), true);
    }

    pub fn push_command_output_log(&self, level: ConsoleLogLevel, message: impl Into<String>) {
        self.push_log_inner(level, message.into(), false);
    }

    fn push_log_inner(&self, level: ConsoleLogLevel, message: String, runtime_log: bool) {
        let raw_message = sanitize_console_text(&message);
        let mut state = self.state.lock().expect("console lock poisoned");
        for line in raw_message.lines() {
            let entry = state.log_store.append(level, line.to_string());
            if runtime_log {
                push_runtime_log(&mut state, entry.clone());
                let _ = state.runtime_log_sender.send(entry);
            }
        }

        if raw_message.ends_with('\n') {
            let entry = state.log_store.append(level, String::new());
            if runtime_log {
                push_runtime_log(&mut state, entry.clone());
                let _ = state.runtime_log_sender.send(entry);
            }
        }
    }

    pub fn subscribe_runtime_logs(&self) -> broadcast::Receiver<ConsoleLogEntry> {
        self.state
            .lock()
            .expect("console lock poisoned")
            .runtime_log_sender
            .subscribe()
    }

    pub fn snapshot(&self) -> ConsoleSnapshot {
        let state = self.state.lock().expect("console lock poisoned");
        ConsoleSnapshot {
            interactive: state.interactive,
            total_log_count: state.log_store.total_log_count(),
            log_file_path: state.log_store.log_path().display().to_string(),
            pending_approvals: state
                .pending_approvals
                .iter()
                .map(|approval| PendingApprovalView {
                    id: approval.id,
                    execution_id: approval.execution_id,
                    request: approval.request.clone(),
                    created_at: approval.created_at.clone(),
                })
                .collect(),
        }
    }

    pub fn read_logs(&self, start: usize, limit: usize) -> Vec<ConsoleLogEntry> {
        self.state
            .lock()
            .expect("console lock poisoned")
            .log_store
            .read_range(start, limit)
    }

    pub fn read_runtime_logs(&self, start: usize, limit: usize) -> Vec<ConsoleLogEntry> {
        let state = self.state.lock().expect("console lock poisoned");
        state
            .runtime_logs
            .iter()
            .skip(start)
            .take(limit)
            .cloned()
            .collect()
    }

    pub(crate) fn clear_session_approvals(&self) {
        self.state
            .lock()
            .expect("console lock poisoned")
            .session_approval_grants
            .clear();
    }

    #[cfg(test)]
    pub fn reconfigure_logging(&self, logging: LoggingConfig) -> io::Result<()> {
        let prepared = self.prepare_logging_reconfigure(logging)?;
        if let Some(prepared) = prepared {
            self.apply_logging_reconfigure(prepared);
        }
        Ok(())
    }

    pub(crate) fn prepare_logging_reconfigure(
        &self,
        logging: LoggingConfig,
    ) -> io::Result<Option<PreparedLoggingReconfigure>> {
        let snapshot = {
            self.state
                .lock()
                .expect("console lock poisoned")
                .log_store
                .reconfigure_snapshot()
        };
        let prepared = snapshot.prepare(logging)?;
        Ok(prepared.map(|prepared| PreparedLoggingReconfigure { prepared }))
    }

    pub(crate) fn apply_logging_reconfigure(&self, prepared: PreparedLoggingReconfigure) {
        let mut state = self.state.lock().expect("console lock poisoned");
        state.log_store.apply_reconfigure(prepared.prepared);
        while state.runtime_logs.len() > state.log_store.buffer_limit() {
            state.runtime_logs.pop_front();
        }
    }

    pub async fn request_confirmation(
        &self,
        execution_id: Uuid,
        request: ConfirmationRequest,
        session_id: Option<String>,
    ) -> Result<bool, ConsoleApprovalError> {
        let session_approval_key = session_id
            .as_deref()
            .map(|session_id| SessionApprovalKey::new(session_id, &request));
        let request_preview = request.command_line.clone();
        let (sender, receiver) = oneshot::channel();
        let (approval_id, receiver) = {
            let mut state = self.state.lock().expect("console lock poisoned");

            if session_approval_key
                .as_ref()
                .is_some_and(|key| state.session_approval_grants.contains(key))
            {
                return Ok(true);
            }

            if !state.interactive {
                return Err(ConsoleApprovalError::Unavailable);
            }

            let approval_id = Uuid::new_v4();
            state.pending_approvals.push(PendingApproval::new(
                approval_id,
                execution_id,
                request,
                sender,
                session_approval_key,
            ));

            (approval_id, receiver)
        };

        self.push_log(
            ConsoleLogLevel::Warn,
            format!("Approval pending [{approval_id}]: {request_preview}"),
        );

        let mut guard = PendingApprovalGuard::new(self.clone(), approval_id);

        let approved = receiver
            .await
            .map_err(|_| ConsoleApprovalError::Cancelled)?;
        guard.disarm();
        Ok(approved)
    }

    pub fn resolve_confirmation(&self, approval_id: Uuid, decision: ApprovalDecision) -> bool {
        let (mut approval, decision) = {
            let mut state = self.state.lock().expect("console lock poisoned");
            let Some(index) = state
                .pending_approvals
                .iter()
                .position(|pending| pending.id == approval_id)
            else {
                return false;
            };
            let approval = state.pending_approvals.remove(index);
            let effective_decision = if decision == ApprovalDecision::ApproveForSession
                && approval.session_approval_key().is_none()
            {
                ApprovalDecision::ApproveOnce
            } else {
                decision
            };

            if effective_decision == ApprovalDecision::ApproveForSession {
                if let Some(key) = approval.session_approval_key() {
                    state.session_approval_grants.insert(key.clone());
                }
            }

            (approval, effective_decision)
        };

        let approved = decision != ApprovalDecision::Reject;
        let decision_label = match decision {
            ApprovalDecision::ApproveOnce => "approved",
            ApprovalDecision::ApproveForSession => "approved for session",
            ApprovalDecision::Reject => "rejected",
        };
        self.push_log(
            ConsoleLogLevel::Info,
            format!(
                "Approval {decision_label} [{}]: {}",
                approval.id, approval.request.command_line
            ),
        );

        approval.deliver(approved);

        true
    }

    pub fn shutdown(&self, reason: &str) {
        let pending_approvals = {
            let mut state = self.state.lock().expect("console lock poisoned");
            state.interactive = false;
            state.session_approval_grants.clear();
            state.pending_approvals.drain(..).collect::<Vec<_>>()
        };

        for mut approval in pending_approvals {
            approval.cancel();
        }

        self.push_log(ConsoleLogLevel::Error, reason.to_string());
    }

    fn cancel_pending_confirmation(&self, approval_id: Uuid) {
        let cancelled = {
            let mut state = self.state.lock().expect("console lock poisoned");
            let Some(index) = state
                .pending_approvals
                .iter()
                .position(|pending| pending.id == approval_id)
            else {
                return;
            };
            state.pending_approvals.remove(index)
        };

        self.push_log(
            ConsoleLogLevel::Warn,
            format!(
                "Approval cancelled [{}]: {}",
                cancelled.id, cancelled.request.command_line
            ),
        );
    }
}

impl Default for OperatorConsole {
    fn default() -> Self {
        Self::new(LoggingConfig::default()).expect("default operator console should initialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tokio::time::{Duration, timeout};

    fn sample_request() -> ConfirmationRequest {
        ConfirmationRequest {
            server: "host".to_string(),
            platform: "linux".to_string(),
            command_line: "cargo build".to_string(),
            executable: "cargo".to_string(),
            args: vec!["build".to_string()],
            working_directory: Some("/workspace".to_string()),
            timeout_ms: 1_000,
            env: HashMap::new(),
            contains_shell_operator: false,
        }
    }

    fn sample_shell_request() -> ConfirmationRequest {
        let mut request = sample_request();
        request.command_line = "cargo build && cargo test".to_string();
        request.args = vec![
            "build".to_string(),
            "&&".to_string(),
            "cargo".to_string(),
            "test".to_string(),
        ];
        request.contains_shell_operator = true;
        request
    }

    fn sample_logging() -> LoggingConfig {
        LoggingConfig::default()
    }

    #[tokio::test]
    async fn request_confirmation_requires_interactive_console() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");

        let result = console
            .request_confirmation(Uuid::new_v4(), sample_request(), None)
            .await;
        assert!(matches!(result, Err(ConsoleApprovalError::Unavailable)));
    }

    #[tokio::test]
    async fn resolve_confirmation_wakes_waiter() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.set_interactive(true);

        let waiter_console = console.clone();
        let wait_task = tokio::spawn(async move {
            waiter_console
                .request_confirmation(Uuid::new_v4(), sample_request(), None)
                .await
        });

        tokio::task::yield_now().await;
        let approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(approval_id, ApprovalDecision::ApproveOnce));

        let approved = timeout(Duration::from_secs(1), wait_task)
            .await
            .expect("wait task should complete before timeout")
            .expect("wait task should not panic");
        assert_eq!(approved.expect("approval should be delivered"), true);
    }

    #[tokio::test]
    async fn session_approval_reuses_same_execution_scope_only_for_same_session() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.set_interactive(true);

        let waiter_console = console.clone();
        let wait_task = tokio::spawn(async move {
            waiter_console
                .request_confirmation(
                    Uuid::new_v4(),
                    sample_shell_request(),
                    Some("session-a".to_string()),
                )
                .await
        });

        tokio::task::yield_now().await;
        let approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(approval_id, ApprovalDecision::ApproveForSession));

        let approved = timeout(Duration::from_secs(1), wait_task)
            .await
            .expect("wait task should complete before timeout")
            .expect("wait task should not panic")
            .expect("approval should be delivered");
        assert!(approved);

        let reused = console
            .request_confirmation(
                Uuid::new_v4(),
                sample_shell_request(),
                Some("session-a".to_string()),
            )
            .await
            .expect("same-session approval should be reused");
        assert!(reused);
        assert!(console.snapshot().pending_approvals.is_empty());

        let mut changed_timeout_request = sample_shell_request();
        changed_timeout_request.timeout_ms += 1_000;
        let timeout_reused = console
            .request_confirmation(
                Uuid::new_v4(),
                changed_timeout_request,
                Some("session-a".to_string()),
            )
            .await
            .expect("timeout changes should not invalidate same-session approval");
        assert!(timeout_reused);
        assert!(console.snapshot().pending_approvals.is_empty());

        let mut changed_request = sample_shell_request();
        changed_request
            .env
            .insert("RUST_LOG".to_string(), "debug".to_string());
        let changed_request_console = console.clone();
        let changed_request_waiter = tokio::spawn(async move {
            changed_request_console
                .request_confirmation(
                    Uuid::new_v4(),
                    changed_request,
                    Some("session-a".to_string()),
                )
                .await
        });

        tokio::task::yield_now().await;
        let changed_approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(changed_approval_id, ApprovalDecision::Reject));

        let changed_result = timeout(Duration::from_secs(1), changed_request_waiter)
            .await
            .expect("changed request should complete before timeout")
            .expect("changed request task should not panic")
            .expect("changed request rejection should be delivered");
        assert!(!changed_result);

        let other_session_console = console.clone();
        let other_session_waiter = tokio::spawn(async move {
            other_session_console
                .request_confirmation(
                    Uuid::new_v4(),
                    sample_shell_request(),
                    Some("session-b".to_string()),
                )
                .await
        });

        tokio::task::yield_now().await;
        let other_approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(other_approval_id, ApprovalDecision::Reject));

        let rejected = timeout(Duration::from_secs(1), other_session_waiter)
            .await
            .expect("other session should complete before timeout")
            .expect("other session task should not panic")
            .expect("rejection should be delivered");
        assert!(!rejected);
    }

    #[tokio::test]
    async fn session_approval_without_session_id_is_one_time_only() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.set_interactive(true);

        let waiter_console = console.clone();
        let wait_task = tokio::spawn(async move {
            waiter_console
                .request_confirmation(Uuid::new_v4(), sample_request(), None)
                .await
        });

        tokio::task::yield_now().await;
        let approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(approval_id, ApprovalDecision::ApproveForSession));

        let approved = timeout(Duration::from_secs(1), wait_task)
            .await
            .expect("wait task should complete before timeout")
            .expect("wait task should not panic")
            .expect("approval should be delivered");
        assert!(approved);

        let waiter_console = console.clone();
        let second_waiter = tokio::spawn(async move {
            waiter_console
                .request_confirmation(Uuid::new_v4(), sample_request(), None)
                .await
        });

        tokio::task::yield_now().await;
        assert_eq!(console.snapshot().pending_approvals.len(), 1);
        let second_approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(second_approval_id, ApprovalDecision::Reject));

        let rejected = timeout(Duration::from_secs(1), second_waiter)
            .await
            .expect("second waiter should complete before timeout")
            .expect("second waiter should not panic")
            .expect("rejection should be delivered");
        assert!(!rejected);
    }

    #[test]
    fn reads_logs_from_file_and_buffer() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.push_log(ConsoleLogLevel::Info, "line-1");
        console.push_log(ConsoleLogLevel::Warn, "line-2");
        console.push_log(ConsoleLogLevel::Error, "line-3");

        let snapshot = console.snapshot();
        assert_eq!(snapshot.total_log_count, 3);

        let entries = console.read_logs(0, 3);
        assert_eq!(entries.len(), 3);
        assert!(!entries[0].timestamp.is_empty());
        assert_eq!(entries[0].message, "line-1");
        assert_eq!(entries[1].message, "line-2");
        assert_eq!(entries[2].message, "line-3");
    }

    #[test]
    fn strips_ansi_sequences_from_logs() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.push_log(
            ConsoleLogLevel::Info,
            "\u{1b}[15;12Hhello \u{1b}[31mworld\u{1b}[0m",
        );

        let entries = console.read_logs(0, 1);
        assert_eq!(entries[0].message, "hello world");
    }

    #[test]
    fn writes_runtime_log_to_data_directory() {
        let data_root =
            std::env::temp_dir().join(format!("host-bridge-mcp-test-{}", Uuid::new_v4()));
        let data_directory =
            DataDirectory::from_root(data_root.clone()).expect("data directory should initialize");
        let log_path = data_root.join("logs/host-bridge.log");
        {
            let console =
                OperatorConsole::with_data_directory(LoggingConfig::default(), data_directory)
                    .expect("console should initialize");
            console.push_log(ConsoleLogLevel::Info, "line-1");
            assert!(log_path.exists());
        }

        assert!(log_path.exists());
        let _ = fs::remove_dir_all(data_root);
    }

    #[test]
    fn persistent_log_file_is_archived_on_startup() {
        let log_dir =
            std::env::temp_dir().join(format!("host-bridge-mcp-persist-{}", Uuid::new_v4()));
        fs::create_dir_all(&log_dir).expect("log directory should be created");
        let data_directory =
            DataDirectory::from_root(log_dir.clone()).expect("data directory should initialize");
        let log_path = log_dir.join("logs/host-bridge.log");
        fs::create_dir_all(log_path.parent().expect("log parent should exist"))
            .expect("log directory should be created");
        let seed_line = "2026-03-09T16:16:21.751592Z  INFO line-0\n";
        fs::write(&log_path, seed_line).expect("seed log file should be written");

        {
            let console =
                OperatorConsole::with_data_directory(LoggingConfig::default(), data_directory)
                    .expect("console should initialize");

            assert!(console.read_logs(0, 1).is_empty());

            console.push_log(ConsoleLogLevel::Warn, "line-1");

            let entries = console.read_logs(0, 2);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].message, "line-1");
        }

        let contents = fs::read_to_string(&log_path).expect("log file should remain readable");
        assert!(!contents.contains(seed_line));
        assert!(contents.contains(" WARN line-1\n"));

        let archived_logs: Vec<_> =
            fs::read_dir(log_path.parent().expect("log path should have a parent"))
                .expect("log directory should remain readable")
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("host-bridge.") && name != "host-bridge.log"
                        })
                })
                .collect();
        assert_eq!(archived_logs.len(), 1);

        let archived_name = archived_logs[0]
            .file_name()
            .and_then(|name| name.to_str())
            .expect("archived log file should have a valid file name");
        assert!(archived_name.contains(".20"));
        assert!(archived_name.ends_with(".1.log"));

        let archived_contents = fs::read_to_string(&archived_logs[0])
            .expect("archived log file should remain readable");
        assert!(archived_contents.contains(seed_line));
        assert!(!archived_contents.contains(" WARN line-1\n"));

        let _ = fs::remove_dir_all(log_dir);
    }

    #[test]
    fn reconfigure_logging_preserves_buffered_entries() {
        let console =
            OperatorConsole::new(LoggingConfig::default()).expect("console should initialize");
        console.push_log(ConsoleLogLevel::Info, "line-1");
        console.push_log(ConsoleLogLevel::Warn, "line-2");

        console
            .reconfigure_logging(LoggingConfig { retention_days: 0 })
            .expect("logging should reconfigure");

        let entries = console.read_logs(0, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "line-1");
        assert_eq!(entries[1].message, "line-2");
    }

    #[test]
    fn prepared_logging_reconfigure_keeps_logs_written_before_commit() {
        let console =
            OperatorConsole::new(LoggingConfig::default()).expect("console should initialize");
        console.push_log(ConsoleLogLevel::Info, "line-1");

        let prepared = console
            .prepare_logging_reconfigure(LoggingConfig { retention_days: 0 })
            .expect("logging reconfigure should prepare")
            .expect("logging reconfigure should be needed");
        console.push_log(ConsoleLogLevel::Warn, "line-2");
        console.apply_logging_reconfigure(prepared);

        let entries = console.read_logs(0, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "line-1");
        assert_eq!(entries[1].message, "line-2");
    }
}
