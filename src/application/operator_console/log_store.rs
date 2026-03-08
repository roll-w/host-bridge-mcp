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

use crate::application::operator_console::{ConsoleLogEntry, ConsoleLogLevel};
use crate::config::LoggingConfig;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) struct LogStore {
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

impl LogStore {
    pub(super) fn new(logging: LoggingConfig) -> io::Result<Self> {
        Ok(Self {
            buffer_limit: logging.memory_buffer_lines,
            buffered_logs: VecDeque::with_capacity(logging.memory_buffer_lines),
            total_log_count: 0,
            storage: LogFileStorage::new(logging)?,
        })
    }

    pub(super) fn total_log_count(&self) -> usize {
        self.total_log_count
    }

    pub(super) fn log_path(&self) -> &Path {
        &self.storage.path
    }

    pub(super) fn append(&mut self, level: ConsoleLogLevel, message: String) {
        let entry = ConsoleLogEntry { level, message };
        if self.buffered_logs.len() >= self.buffer_limit {
            self.buffered_logs.pop_front();
        }
        self.buffered_logs.push_back(entry.clone());
        self.storage.append(&entry);
        self.total_log_count += 1;
    }

    pub(super) fn read_range(&mut self, start: usize, limit: usize) -> Vec<ConsoleLogEntry> {
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
        let serialized = format!("{}\t{}\n", log_level_tag(entry.level), entry.message);
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
        level: parse_log_level(tag)?,
        message: message.to_string(),
    })
}

fn log_level_tag(level: ConsoleLogLevel) -> &'static str {
    match level {
        ConsoleLogLevel::Info => "INFO",
        ConsoleLogLevel::Warn => "WARN",
        ConsoleLogLevel::Error => "ERROR",
    }
}

fn parse_log_level(tag: &str) -> Option<ConsoleLogLevel> {
    match tag {
        "INFO" => Some(ConsoleLogLevel::Info),
        "WARN" => Some(ConsoleLogLevel::Warn),
        "ERROR" => Some(ConsoleLogLevel::Error),
        _ => None,
    }
}
