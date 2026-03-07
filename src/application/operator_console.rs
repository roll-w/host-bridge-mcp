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

use crate::application::execution_service::ConfirmationRequest;
use crate::config::LoggingConfig;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
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

struct PendingApproval {
    id: Uuid,
    request: ConfirmationRequest,
    created_at: SystemTime,
    responder: Option<oneshot::Sender<bool>>,
}

struct LogStore {
    buffer_limit: usize,
    buffered_logs: VecDeque<ConsoleLogEntry>,
    total_log_count: usize,
    storage: LogFileStorage,
}

struct LogFileStorage {
    path: PathBuf,
    writer: Option<File>,
    line_offsets: Vec<u64>,
    next_offset: u64,
    delete_on_drop: bool,
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
            total_log_count: state.log_store.total_log_count,
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
            state.pending_approvals.push(PendingApproval {
                id: approval_id,
                request,
                created_at: SystemTime::now(),
                responder: Some(sender),
            });
        }

        self.push_log(
            ConsoleLogLevel::Warn,
            format!("Approval pending [{approval_id}]: {request_preview}"),
        );

        let mut guard = PendingApprovalGuard {
            console: self.clone(),
            approval_id,
            active: true,
        };

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

        if let Some(sender) = approval.responder.take() {
            let _ = sender.send(approved);
        }

        true
    }

    pub fn shutdown(&self, reason: &str) {
        let pending_approvals = {
            let mut state = self.state.lock().expect("console lock poisoned");
            state.interactive = false;
            state.pending_approvals.drain(..).collect::<Vec<_>>()
        };

        for mut approval in pending_approvals {
            approval.responder.take();
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

impl LogStore {
    fn new(logging: LoggingConfig) -> io::Result<Self> {
        Ok(Self {
            buffer_limit: logging.memory_buffer_lines,
            buffered_logs: VecDeque::with_capacity(logging.memory_buffer_lines),
            total_log_count: 0,
            storage: LogFileStorage::new(logging)?,
        })
    }

    fn append(&mut self, level: ConsoleLogLevel, message: String) {
        let entry = ConsoleLogEntry { level, message };
        if self.buffered_logs.len() >= self.buffer_limit {
            self.buffered_logs.pop_front();
        }
        self.buffered_logs.push_back(entry.clone());
        self.storage.append(&entry);
        self.total_log_count += 1;
    }

    fn log_path(&self) -> &PathBuf {
        &self.storage.path
    }

    fn read_range(&mut self, start: usize, limit: usize) -> Vec<ConsoleLogEntry> {
        if limit == 0 || start >= self.total_log_count {
            return Vec::new();
        }

        let end = (start + limit).min(self.total_log_count);
        let buffer_start = self
            .total_log_count
            .saturating_sub(self.buffered_logs.len());

        if start >= buffer_start {
            return self
                .buffered_logs
                .iter()
                .skip(start - buffer_start)
                .take(end - start)
                .cloned()
                .collect();
        }

        let file_end = end.min(buffer_start);
        let mut entries = self.storage.read_range(start, file_end);
        if end > buffer_start {
            entries.extend(self.buffered_logs.iter().take(end - buffer_start).cloned());
        }
        entries
    }
}

impl LogFileStorage {
    fn new(logging: LoggingConfig) -> io::Result<Self> {
        let path = resolve_log_path(&logging);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let writer = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;

        Ok(Self {
            path,
            writer: Some(writer),
            line_offsets: Vec::new(),
            next_offset: 0,
            delete_on_drop: !logging.persist_file,
        })
    }

    fn append(&mut self, entry: &ConsoleLogEntry) {
        let serialized = format!("{}\t{}\n", entry.level.as_tag(), entry.message);
        self.line_offsets.push(self.next_offset);
        self.next_offset += serialized.len() as u64;

        if let Some(writer) = self.writer.as_mut() {
            if writer.write_all(serialized.as_bytes()).is_ok() {
                let _ = writer.flush();
            }
        }
    }

    fn read_range(&mut self, start: usize, end: usize) -> Vec<ConsoleLogEntry> {
        if start >= end {
            return Vec::new();
        }

        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
        let mut reader = match File::open(&self.path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };

        let mut entries = Vec::with_capacity(end - start);
        for index in start..end {
            let Some(offset) = self.line_offsets.get(index).copied() else {
                break;
            };
            let next_offset = self
                .line_offsets
                .get(index + 1)
                .copied()
                .unwrap_or(self.next_offset);
            let line_length = next_offset.saturating_sub(offset) as usize;
            if line_length == 0 {
                continue;
            }

            let mut buffer = vec![0_u8; line_length];
            if reader.seek(SeekFrom::Start(offset)).is_err() {
                break;
            }
            if reader.read_exact(&mut buffer).is_err() {
                break;
            }

            let raw_line = String::from_utf8_lossy(&buffer);
            if let Some(entry) = parse_log_line(raw_line.trim_end_matches(['\n', '\r'])) {
                entries.push(entry);
            }
        }

        entries
    }
}

impl Drop for LogFileStorage {
    fn drop(&mut self) {
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.flush();
            drop(writer);
        }
        if self.delete_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl ConsoleLogLevel {
    fn as_tag(self) -> &'static str {
        match self {
            ConsoleLogLevel::Info => "INFO",
            ConsoleLogLevel::Warn => "WARN",
            ConsoleLogLevel::Error => "ERROR",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "INFO" => Some(Self::Info),
            "WARN" => Some(Self::Warn),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }
}

fn resolve_log_path(logging: &LoggingConfig) -> PathBuf {
    if let Some(path) = &logging.file_path {
        return PathBuf::from(path);
    }

    if logging.persist_file {
        return PathBuf::from("host-bridge-mcp.log");
    }

    std::env::temp_dir().join(format!("host-bridge-mcp-{}.log", Uuid::new_v4()))
}

fn parse_log_line(raw: &str) -> Option<ConsoleLogEntry> {
    let (tag, message) = raw.split_once('\t')?;
    Some(ConsoleLogEntry {
        level: ConsoleLogLevel::from_tag(tag)?,
        message: message.to_string(),
    })
}

fn sanitize_console_text(input: &str) -> String {
    enum State {
        Normal,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut sanitized = String::with_capacity(input.len());
    let mut state = State::Normal;

    for ch in input.chars() {
        match state {
            State::Normal => {
                if ch == '\u{1b}' {
                    state = State::Escape;
                } else if ch == '\n' || ch == '\t' || !ch.is_control() {
                    sanitized.push(ch);
                }
            }
            State::Escape => {
                state = match ch {
                    '[' => State::Csi,
                    ']' => State::Osc,
                    _ => State::Normal,
                };
            }
            State::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = State::Normal;
                }
            }
            State::Osc => {
                if ch == '\u{7}' {
                    state = State::Normal;
                } else if ch == '\u{1b}' {
                    state = State::OscEscape;
                }
            }
            State::OscEscape => {
                state = if ch == '\\' {
                    State::Normal
                } else {
                    State::Osc
                };
            }
        }
    }

    sanitized
}

struct PendingApprovalGuard {
    console: OperatorConsole,
    approval_id: Uuid,
    active: bool,
}

impl PendingApprovalGuard {
    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PendingApprovalGuard {
    fn drop(&mut self) {
        if self.active {
            self.console.cancel_pending_confirmation(self.approval_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_request() -> ConfirmationRequest {
        ConfirmationRequest {
            command_line: "cargo build".to_string(),
            executable: "cargo".to_string(),
            args: vec!["build".to_string()],
            working_directory: "/workspace".to_string(),
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
}
