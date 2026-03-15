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
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(super) struct LogStore {
    buffer_limit: usize,
    buffered_logs: VecDeque<ConsoleLogEntry>,
    total_log_count: usize,
    storage: LogFileStorage,
}

pub(super) enum PreparedLogStore {
    UpdatePolicy {
        buffer_limit: usize,
        delete_on_drop: bool,
    },
    Replace {
        replacement: LogStore,
        source_total_log_count: usize,
    },
}

pub(super) struct LogStoreReconfigureSnapshot {
    buffer_limit: usize,
    buffered_logs: VecDeque<ConsoleLogEntry>,
    total_log_count: usize,
    current_path: PathBuf,
    delete_on_drop: bool,
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
        self.push_entry(ConsoleLogEntry {
            timestamp: super::current_console_timestamp(),
            level,
            message,
        });
    }

    pub(super) fn reconfigure_snapshot(&self) -> LogStoreReconfigureSnapshot {
        LogStoreReconfigureSnapshot {
            buffer_limit: self.buffer_limit,
            buffered_logs: self.buffered_logs.clone(),
            total_log_count: self.total_log_count,
            current_path: self.storage.path().to_path_buf(),
            delete_on_drop: self.storage.delete_on_drop(),
        }
    }

    pub(super) fn apply_reconfigure(&mut self, prepared: PreparedLogStore) {
        match prepared {
            PreparedLogStore::UpdatePolicy {
                buffer_limit,
                delete_on_drop,
            } => {
                self.buffer_limit = buffer_limit;
                self.trim_buffer_to_limit();
                self.storage.set_delete_on_drop(delete_on_drop);
            }
            PreparedLogStore::Replace {
                mut replacement,
                source_total_log_count,
            } => {
                if self.total_log_count > source_total_log_count {
                    let pending_entries = self.read_range(
                        source_total_log_count,
                        self.total_log_count - source_total_log_count,
                    );
                    for entry in pending_entries {
                        replacement.restore_entry(entry);
                    }
                }
                *self = replacement;
            }
        }
    }

    fn push_entry(&mut self, entry: ConsoleLogEntry) {
        if self.buffered_logs.len() >= self.buffer_limit {
            self.buffered_logs.pop_front();
        }
        self.buffered_logs.push_back(entry.clone());
        self.storage.append(&entry);
        self.total_log_count += 1;
    }

    fn restore_entry(&mut self, entry: ConsoleLogEntry) {
        self.push_entry(entry);
    }

    fn trim_buffer_to_limit(&mut self) {
        while self.buffered_logs.len() > self.buffer_limit {
            self.buffered_logs.pop_front();
        }
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

impl LogStoreReconfigureSnapshot {
    pub(super) fn prepare(self, logging: LoggingConfig) -> io::Result<Option<PreparedLogStore>> {
        let next_path = resolve_log_path(&logging)?;
        let next_delete_on_drop = !logging.persist_file;
        let next_buffer_limit = logging.memory_buffer_lines;

        if self.current_path == next_path {
            if self.buffer_limit == next_buffer_limit && self.delete_on_drop == next_delete_on_drop
            {
                return Ok(None);
            }

            return Ok(Some(PreparedLogStore::UpdatePolicy {
                buffer_limit: next_buffer_limit,
                delete_on_drop: next_delete_on_drop,
            }));
        }

        let mut replacement = LogStore::new(logging)?;
        for entry in self.buffered_logs {
            replacement.restore_entry(entry);
        }

        Ok(Some(PreparedLogStore::Replace {
            replacement,
            source_total_log_count: self.total_log_count,
        }))
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
        if append_mode {
            archive_existing_log_file(&path)?;
        }

        let (line_offsets, next_offset) = (Vec::new(), 0);
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

    fn path(&self) -> &Path {
        &self.path
    }

    fn delete_on_drop(&self) -> bool {
        self.delete_on_drop
    }

    fn set_delete_on_drop(&mut self, delete_on_drop: bool) {
        self.delete_on_drop = delete_on_drop;
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

fn archive_existing_log_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => fs::rename(path, archived_log_path(path)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn archived_log_path(path: &Path) -> io::Result<PathBuf> {
    let date = archive_date_label();
    let mut index = 1_usize;

    loop {
        let archived_path = path.with_file_name(archived_log_file_name(path, &date, index)?);
        match fs::symlink_metadata(&archived_path) {
            Ok(_) => index += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(archived_path),
            Err(error) => return Err(error),
        }
    }
}

fn archived_log_file_name(path: &Path, date: &str, index: usize) -> io::Result<OsString> {
    let Some(stem) = path.file_stem() else {
        let Some(file_name) = path.file_name() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("log path '{}' must include a file name", path.display()),
            ));
        };

        let mut archived_name = OsString::from(file_name);
        archived_name.push(format!(".{date}.{index}"));
        return Ok(archived_name);
    };

    let mut archived_name = OsString::from(stem);
    archived_name.push(format!(".{date}.{index}"));
    if let Some(extension) = path.extension() {
        archived_name.push(".");
        archived_name.push(extension);
    }

    Ok(archived_name)
}

fn archive_date_label() -> String {
    sanitize_archive_date(&super::current_console_timestamp())
}

fn sanitize_archive_date(timestamp: &str) -> String {
    timestamp
        .chars()
        .take_while(|character| *character != 'T')
        .filter(|character| character.is_ascii_digit() || *character == '-')
        .collect()
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
    fn current_console_timestamp_matches_expected_utc_shape() {
        let timestamp = super::super::current_console_timestamp();
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

    #[test]
    fn archived_log_file_name_preserves_extension() {
        let archived_name =
            archived_log_file_name(Path::new("/tmp/host-bridge-mcp.log"), "2026-03-15", 2)
                .expect("archived log file name should be generated");

        assert_eq!(
            archived_name,
            OsString::from("host-bridge-mcp.2026-03-15.2.log")
        );
    }

    #[test]
    fn sanitize_archive_date_keeps_only_year_month_day() {
        assert_eq!(
            sanitize_archive_date("2026-03-15T09:13:41.123456Z"),
            "2026-03-15"
        );
    }
}
