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

use crate::application::data_dir::DataDirectory;
use crate::application::operator_console::{ConsoleLogEntry, ConsoleLogLevel};
use crate::config::LoggingConfig;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const RUNTIME_LOG_BUFFER_LIMIT: usize = 2_000;

pub(super) struct LogStore {
    buffered_logs: VecDeque<ConsoleLogEntry>,
    total_log_count: usize,
    storage: LogFileStorage,
}

pub(super) enum PreparedLogStore {
    UpdatePolicy { retention_days: u64 },
}

pub(super) struct LogStoreReconfigureSnapshot {
    retention_days: u64,
}

struct LogFileSegment {
    path: PathBuf,
    start_line: usize,
    line_offsets: Vec<u64>,
    next_offset: u64,
}

struct LogFileStorage {
    path: PathBuf,
    writer: Option<File>,
    line_offsets: Vec<u64>,
    next_offset: u64,
    active_start_line: usize,
    archived_segments: Vec<LogFileSegment>,
    retention_days: u64,
    active_date: String,
}

impl LogStore {
    pub(super) fn new(logging: LoggingConfig, data_directory: &DataDirectory) -> io::Result<Self> {
        let storage = LogFileStorage::new(logging, data_directory)?;

        Ok(Self {
            buffered_logs: VecDeque::with_capacity(RUNTIME_LOG_BUFFER_LIMIT),
            total_log_count: storage.line_count(),
            storage,
        })
    }

    pub(super) fn total_log_count(&self) -> usize {
        self.total_log_count
    }

    pub(super) fn buffer_limit(&self) -> usize {
        RUNTIME_LOG_BUFFER_LIMIT
    }

    pub(super) fn log_path(&self) -> &Path {
        &self.storage.path
    }

    pub(super) fn append(&mut self, level: ConsoleLogLevel, message: String) -> ConsoleLogEntry {
        let entry = ConsoleLogEntry {
            timestamp: super::current_console_timestamp(),
            level,
            message,
        };
        self.push_entry(entry.clone());
        entry
    }

    pub(super) fn reconfigure_snapshot(&self) -> LogStoreReconfigureSnapshot {
        LogStoreReconfigureSnapshot {
            retention_days: self.storage.retention_days,
        }
    }

    pub(super) fn apply_reconfigure(&mut self, prepared: PreparedLogStore) {
        match prepared {
            PreparedLogStore::UpdatePolicy { retention_days } => {
                self.storage.set_retention_days(retention_days);
            }
        }
    }

    fn push_entry(&mut self, entry: ConsoleLogEntry) {
        if self.buffered_logs.len() >= RUNTIME_LOG_BUFFER_LIMIT {
            self.buffered_logs.pop_front();
        }
        self.buffered_logs.push_back(entry.clone());
        self.storage.append(&entry, self.total_log_count);
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

impl LogStoreReconfigureSnapshot {
    pub(super) fn prepare(self, logging: LoggingConfig) -> io::Result<Option<PreparedLogStore>> {
        if self.retention_days == logging.retention_days {
            return Ok(None);
        }

        Ok(Some(PreparedLogStore::UpdatePolicy {
            retention_days: logging.retention_days,
        }))
    }
}

impl LogFileStorage {
    fn new(logging: LoggingConfig, data_directory: &DataDirectory) -> io::Result<Self> {
        let path = data_directory.runtime_log_path()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        archive_existing_log_file(&path)?;
        cleanup_archived_log_files(&path, logging.retention_days)?;

        let (line_offsets, next_offset) = (Vec::new(), 0);
        let writer = open_private_write_file(&path, false)?;

        Ok(Self {
            path,
            writer: Some(writer),
            line_offsets,
            next_offset,
            active_start_line: 0,
            archived_segments: Vec::new(),
            retention_days: logging.retention_days,
            active_date: current_log_date(),
        })
    }

    fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    fn set_retention_days(&mut self, retention_days: u64) {
        self.retention_days = retention_days;
        if let Err(error) = cleanup_archived_log_files(&self.path, retention_days) {
            tracing::warn!(
                path = %self.path.display(),
                error = %error,
                "Failed to apply runtime log retention"
            );
        }
    }

    fn append(&mut self, entry: &ConsoleLogEntry, global_line_index: usize) {
        let serialized = serialize_log_entry(entry);

        if self.should_rotate() {
            let _ = self.rotate(global_line_index);
        }

        if let Some(writer) = self.writer.as_mut() {
            if writer.write_all(serialized.as_bytes()).is_ok() && writer.flush().is_ok() {
                self.line_offsets.push(self.next_offset);
                self.next_offset += serialized.len() as u64;
            }
        }
    }

    fn should_rotate(&self) -> bool {
        !self.line_offsets.is_empty() && self.active_date != current_log_date()
    }

    fn rotate(&mut self, next_line_index: usize) -> io::Result<()> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        writer.flush()?;
        drop(writer);

        let archived_path = archived_log_path(&self.path)?;
        fs::rename(&self.path, &archived_path)?;
        self.archived_segments.push(LogFileSegment {
            path: archived_path,
            start_line: self.active_start_line,
            line_offsets: std::mem::take(&mut self.line_offsets),
            next_offset: self.next_offset,
        });
        self.next_offset = 0;
        self.active_start_line = next_line_index;
        self.writer = Some(open_private_write_file(&self.path, false)?);
        self.active_date = current_log_date();

        cleanup_archived_log_files(&self.path, self.retention_days)?;
        self.archived_segments.retain(|segment| {
            fs::symlink_metadata(&segment.path)
                .map(|metadata| metadata.is_file())
                .unwrap_or(false)
        });
        Ok(())
    }

    fn read_range(&mut self, start: usize, end: usize) -> Vec<ConsoleLogEntry> {
        if start >= end {
            return Vec::new();
        }

        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }

        let mut entries = Vec::with_capacity(end - start);
        for segment in &self.archived_segments {
            read_segment_range(segment, start, end, &mut entries);
        }
        let current_segment = LogFileSegment {
            path: self.path.clone(),
            start_line: self.active_start_line,
            line_offsets: self.line_offsets.clone(),
            next_offset: self.next_offset,
        };
        read_segment_range(&current_segment, start, end, &mut entries);
        entries
    }
}

fn archive_existing_log_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => match fs::rename(path, archived_log_path(path)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn read_segment_range(
    segment: &LogFileSegment,
    start: usize,
    end: usize,
    entries: &mut Vec<ConsoleLogEntry>,
) {
    let segment_start = segment.start_line;
    let segment_end = segment_start + segment.line_offsets.len();
    let range_start = start.max(segment_start);
    let range_end = end.min(segment_end);
    if range_start >= range_end {
        return;
    }

    let Ok(mut reader) = File::open(&segment.path) else {
        return;
    };

    for global_index in range_start..range_end {
        let local_index = global_index - segment_start;
        let Some(offset) = segment.line_offsets.get(local_index).copied() else {
            break;
        };
        let next_offset = segment
            .line_offsets
            .get(local_index + 1)
            .copied()
            .unwrap_or(segment.next_offset);
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
}

fn cleanup_archived_log_files(path: &Path, retention_days: u64) -> io::Result<()> {
    if retention_days == 0 {
        return Ok(());
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            retention_days.saturating_mul(24 * 60 * 60),
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let candidate = entry.path();
        if !is_archived_log_path(path, &candidate) {
            continue;
        }
        let modified = entry
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified < cutoff {
            fs::remove_file(candidate)?;
        }
    }
    Ok(())
}

fn is_archived_log_path(active_path: &Path, candidate: &Path) -> bool {
    let Some(active_stem) = active_path
        .file_stem()
        .or_else(|| active_path.file_name())
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    let Some(candidate_name) = candidate.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let prefix = format!("{active_stem}.");
    let Some(mut archived_name) = candidate_name.strip_prefix(&prefix) else {
        return false;
    };
    if let Some(extension) = active_path.extension().and_then(|value| value.to_str()) {
        let suffix = format!(".{extension}");
        let Some(without_extension) = archived_name.strip_suffix(&suffix) else {
            return false;
        };
        archived_name = without_extension;
    }

    let mut parts = archived_name.split('.');
    let Some(date) = parts.next() else {
        return false;
    };
    let Some(index) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !date.is_empty()
        && date
        .chars()
        .all(|character| character.is_ascii_digit() || character == '-')
        && !index.is_empty()
        && index.chars().all(|character| character.is_ascii_digit())
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

fn current_log_date() -> String {
    archive_date_label()
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
    }
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

    #[test]
    fn rotates_when_calendar_date_changes() {
        let directory = std::env::temp_dir().join(format!(
            "host-bridge-mcp-log-rotation-{}",
            uuid::Uuid::new_v4()
        ));
        let data_directory =
            DataDirectory::from_root(directory.clone()).expect("data directory should initialize");
        let path = directory.join("logs/host-bridge.log");
        let mut storage = LogFileStorage::new(LoggingConfig::default(), &data_directory)
            .expect("log storage should initialize");
        let first = ConsoleLogEntry {
            timestamp: "2026-03-15T09:13:41.123456Z".to_string(),
            level: ConsoleLogLevel::Info,
            message: "first".to_string(),
        };
        let second = ConsoleLogEntry {
            message: "second".to_string(),
            ..first.clone()
        };

        storage.append(&first, 0);
        storage.active_date = "2000-01-01".to_string();
        storage.append(&second, 1);

        let archives = fs::read_dir(path.parent().expect("log path should have a parent"))
            .expect("log directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| is_archived_log_path(&path, candidate))
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert!(
            fs::read_to_string(&archives[0])
                .expect("archived log should be readable")
                .contains("first")
        );
        assert!(
            fs::read_to_string(&path)
                .expect("active log should be readable")
                .contains("second")
        );

        drop(storage);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn zero_retention_keeps_archived_logs() {
        let directory = std::env::temp_dir().join(format!(
            "host-bridge-mcp-log-time-rotation-{}",
            uuid::Uuid::new_v4()
        ));
        let data_directory =
            DataDirectory::from_root(directory.clone()).expect("data directory should initialize");
        let mut storage = LogFileStorage::new(LoggingConfig { retention_days: 0 }, &data_directory)
            .expect("log storage should initialize");
        let entry = ConsoleLogEntry {
            timestamp: "2026-03-15T09:13:41.123456Z".to_string(),
            level: ConsoleLogLevel::Info,
            message: "line".to_string(),
        };

        storage.append(&entry, 0);
        storage.active_date = "2000-01-01".to_string();
        storage.append(&entry, 1);

        assert_eq!(storage.archived_segments.len(), 1);
        let archived_path = storage.archived_segments[0].path.clone();

        drop(storage);
        assert!(archived_path.exists());
        let _ = fs::remove_dir_all(directory);
    }
}
