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

use self::approvals::{PendingApproval, PendingApprovalGuard};
use self::log_store::LogStore;
use self::sanitize::sanitize_console_text;
use crate::application::execution_service::ConfirmationRequest;
use crate::config::LoggingConfig;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct ConsoleLogEntry {
    pub timestamp: String,
    pub level: ConsoleLogLevel,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    pub id: Uuid,
    pub request: ConfirmationRequest,
    pub created_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ConsoleSnapshot {
    pub interactive: bool,
    pub total_log_count: usize,
    pub log_file_path: String,
    pub pending_approvals: Vec<PendingApprovalView>,
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

struct ConsoleState {
    interactive: bool,
    log_store: LogStore,
    pending_approvals: Vec<PendingApproval>,
}

impl OperatorConsole {
    pub fn new(logging: LoggingConfig) -> io::Result<Self> {
        Ok(Self {
            state: Arc::new(Mutex::new(ConsoleState {
                interactive: false,
                log_store: LogStore::new(logging)?,
                pending_approvals: Vec::new(),
            })),
        })
    }

    pub fn set_interactive(&self, interactive: bool) {
        self.state
            .lock()
            .expect("console lock poisoned")
            .interactive = interactive;
    }

    pub fn is_interactive(&self) -> bool {
        self.state
            .lock()
            .expect("console lock poisoned")
            .interactive
    }

    pub fn push_log(&self, level: ConsoleLogLevel, message: impl Into<String>) {
        let raw_message = sanitize_console_text(&message.into());
        let mut state = self.state.lock().expect("console lock poisoned");
        for line in raw_message.lines() {
            state.log_store.append(level, line.to_string());
        }

        if raw_message.ends_with('\n') {
            state.log_store.append(level, String::new());
        }
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
                    request: approval.request.clone(),
                    created_at: approval.created_at,
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

    pub async fn request_confirmation(
        &self,
        request: ConfirmationRequest,
    ) -> Result<bool, ConsoleApprovalError> {
        if !self.is_interactive() {
            return Err(ConsoleApprovalError::Unavailable);
        }

        let approval_id = Uuid::new_v4();
        let request_preview = request.command_line.clone();
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self.state.lock().expect("console lock poisoned");
            state
                .pending_approvals
                .push(PendingApproval::new(approval_id, request, sender));
        }

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

    pub fn resolve_confirmation(&self, approval_id: Uuid, approved: bool) -> bool {
        let mut approval = {
            let mut state = self.state.lock().expect("console lock poisoned");
            let Some(index) = state
                .pending_approvals
                .iter()
                .position(|pending| pending.id == approval_id)
            else {
                return false;
            };
            state.pending_approvals.remove(index)
        };

        let decision = if approved { "approved" } else { "rejected" };
        self.push_log(
            ConsoleLogLevel::Info,
            format!(
                "Approval {decision} [{}]: {}",
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
        }
    }

    fn sample_logging() -> LoggingConfig {
        LoggingConfig {
            memory_buffer_lines: 2,
            file_path: None,
            persist_file: false,
        }
    }

    #[tokio::test]
    async fn request_confirmation_requires_interactive_console() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");

        let result = console.request_confirmation(sample_request()).await;
        assert!(matches!(result, Err(ConsoleApprovalError::Unavailable)));
    }

    #[tokio::test]
    async fn resolve_confirmation_wakes_waiter() {
        let console = OperatorConsole::new(sample_logging()).expect("console should initialize");
        console.set_interactive(true);

        let waiter_console = console.clone();
        let wait_task =
            tokio::spawn(
                async move { waiter_console.request_confirmation(sample_request()).await },
            );

        tokio::task::yield_now().await;
        let approval_id = console.snapshot().pending_approvals[0].id;
        assert!(console.resolve_confirmation(approval_id, true));

        let approved = wait_task.await.expect("wait task should complete");
        assert_eq!(approved.expect("approval should be delivered"), true);
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
    fn removes_temporary_log_file_on_drop() {
        let log_path =
            std::env::temp_dir().join(format!("host-bridge-mcp-test-{}.log", Uuid::new_v4()));
        {
            let console = OperatorConsole::new(LoggingConfig {
                memory_buffer_lines: 2,
                file_path: Some(log_path.display().to_string()),
                persist_file: false,
            })
            .expect("console should initialize");
            console.push_log(ConsoleLogLevel::Info, "line-1");
            assert!(log_path.exists());
        }

        assert!(!log_path.exists());
    }

    #[test]
    fn persistent_log_file_is_reused_without_truncation() {
        let log_path =
            std::env::temp_dir().join(format!("host-bridge-mcp-persist-{}.log", Uuid::new_v4()));
        let seed_line = "2026-03-09T16:16:21.751592Z  INFO line-0\n";
        fs::write(&log_path, seed_line).expect("seed log file should be written");

        {
            let console = OperatorConsole::new(LoggingConfig {
                memory_buffer_lines: 2,
                file_path: Some(log_path.display().to_string()),
                persist_file: true,
            })
            .expect("console should initialize");

            let initial_entries = console.read_logs(0, 1);
            assert_eq!(initial_entries.len(), 1);
            assert_eq!(initial_entries[0].timestamp, "2026-03-09T16:16:21.751592Z");
            assert_eq!(initial_entries[0].message, "line-0");

            console.push_log(ConsoleLogLevel::Warn, "line-1");

            let entries = console.read_logs(0, 2);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].message, "line-0");
            assert_eq!(entries[1].message, "line-1");
        }

        let contents = fs::read_to_string(&log_path).expect("log file should remain readable");
        assert!(contents.contains(seed_line));
        assert!(contents.contains(" WARN line-1\n"));

        let _ = fs::remove_file(log_path);
    }
}
