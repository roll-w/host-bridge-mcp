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

use crate::application::data_dir::{default_persisted_log_path, default_temporary_log_path};
use crate::application::operator_console::{ConsoleLogEntry, ConsoleLogLevel};
use crate::config::LoggingConfig;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime as TracingSystemTime};

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
        let buffer_limit = logging.memory_buffer_lines;
        let storage = LogFileStorage::new(logging)?;

        Ok(Self {
            buffer_limit,
            buffered_logs: VecDeque::with_capacity(buffer_limit),
            total_log_count: storage.line_count(),
            storage,
        })
    }

    pub(super) fn total_log_count(&self) -> usize {
        self.total_log_count
    }

    pub(super) fn log_path(&self) -> &Path {
        &self.storage.path
    }

    pub(super) fn append(&mut self, level: ConsoleLogLevel, message: String) {
        let entry = ConsoleLogEntry {
            timestamp: current_log_timestamp(),
            level,
            message,
        };
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
        let path = resolve_log_path(&logging)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let append_mode = logging.persist_file;
        let (line_offsets, next_offset) = if append_mode {
            index_log_file(&path)?
        } else {
            (Vec::new(), 0)
        };
        let writer = open_private_write_file(&path, append_mode)?;

        Ok(Self {
            path,
            writer: Some(writer),
            line_offsets,
            next_offset,
            delete_on_drop: !logging.persist_file,
        })
    }

    fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    fn append(&mut self, entry: &ConsoleLogEntry) {
        let serialized = serialize_log_entry(entry);

        if let Some(writer) = self.writer.as_mut() {
            if writer.write_all(serialized.as_bytes()).is_ok() && writer.flush().is_ok() {
                self.line_offsets.push(self.next_offset);
                self.next_offset += serialized.len() as u64;
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

fn resolve_log_path(logging: &LoggingConfig) -> io::Result<PathBuf> {
    if let Some(path) = &logging.file_path {
        return Ok(PathBuf::from(path));
    }

    if logging.persist_file {
        return default_persisted_log_path();
    }

    default_temporary_log_path()
}

fn open_private_write_file(path: &Path, append_mode: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true);

    if append_mode {
        options.append(true);
    } else {
        options.truncate(true);
    }

    #[cfg(unix)]
    options.mode(0o600);

    options.open(path)
}

fn index_log_file(path: &Path) -> io::Result<(Vec<u64>, u64)> {
    if !path.exists() {
        return Ok((Vec::new(), 0));
    }

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line_offsets = Vec::new();
    let mut next_offset = 0_u64;
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }

        line_offsets.push(next_offset);
        next_offset += read as u64;
    }

    Ok((line_offsets, next_offset))
}

fn parse_log_line(raw: &str) -> Option<ConsoleLogEntry> {
    let (timestamp, remainder) = raw.split_once(' ')?;
    if remainder.len() < 6 {
        return None;
    }

    let (level_field, message_field) = remainder.split_at(5);
    let message = message_field.strip_prefix(' ')?;

    Some(ConsoleLogEntry {
        timestamp: timestamp.to_string(),
        level: parse_log_level(level_field.trim())?,
        message: message.to_string(),
    })
}

fn current_log_timestamp() -> String {
    let timer = TracingSystemTime::default();
    let mut output = String::new();
    let mut writer = Writer::new(&mut output);

    if timer.format_time(&mut writer).is_err() {
        return "1970-01-01T00:00:00.000000Z".to_string();
    }

    output.truncate(output.trim_end().len());
    output
}

fn serialize_log_entry(entry: &ConsoleLogEntry) -> String {
    format!(
        "{} {:>5} {}\n",
        entry.timestamp,
        log_level_tag(entry.level),
        entry.message
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_log_timestamp_matches_expected_utc_shape() {
        let timestamp = current_log_timestamp();
        let bytes = timestamp.as_bytes();

        assert_eq!(timestamp.len(), 27);
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[7], b'-');
        assert_eq!(bytes[10], b'T');
        assert_eq!(bytes[13], b':');
        assert_eq!(bytes[16], b':');
        assert_eq!(bytes[19], b'.');
        assert_eq!(bytes[26], b'Z');
    }

    #[test]
    fn serialize_log_entry_aligns_info_and_error_levels() {
        let info_entry = ConsoleLogEntry {
            timestamp: "2026-03-09T16:16:21.751592Z".to_string(),
            level: ConsoleLogLevel::Info,
            message: "submitted".to_string(),
        };
        let error_entry = ConsoleLogEntry {
            timestamp: "2026-03-09T16:16:21.751592Z".to_string(),
            level: ConsoleLogLevel::Error,
            message: "failed".to_string(),
        };

        assert_eq!(
            serialize_log_entry(&info_entry),
            "2026-03-09T16:16:21.751592Z  INFO submitted\n"
        );
        assert_eq!(
            serialize_log_entry(&error_entry),
            "2026-03-09T16:16:21.751592Z ERROR failed\n"
        );
    }
}
