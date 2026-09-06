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
use crate::application::execution_service::ExecutionState;
use crate::config::HistoryConfig;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MILLIS_PER_DAY: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionHistoryEntry {
    pub execution_id: Uuid,
    pub command_line: String,
    pub server: String,
    pub state: ExecutionState,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub timed_out: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionHistoryPage {
    pub records: Vec<ExecutionHistoryEntry>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionHistoryError {
    #[error("failed to access execution history: {0}")]
    Io(#[from] io::Error),
    #[error("failed to parse execution history: {0}")]
    Json(#[from] serde_json::Error),
    #[error("execution '{0}' is not in history")]
    NotFound(Uuid),
    #[error("execution '{0}' is still running")]
    Running(Uuid),
}

#[derive(Clone)]
pub struct ExecutionHistoryStore {
    path: PathBuf,
    data_directory: DataDirectory,
    state: std::sync::Arc<Mutex<HistoryState>>,
}

struct HistoryState {
    entries: Vec<ExecutionHistoryEntry>,
    config: HistoryConfig,
}

impl ExecutionHistoryStore {
    pub fn new(
        config: HistoryConfig,
        data_directory: DataDirectory,
    ) -> Result<Self, ExecutionHistoryError> {
        let path = data_directory.execution_history_path()?;
        let entries = load_entries(&path)?;
        let store = Self {
            path,
            data_directory,
            state: std::sync::Arc::new(Mutex::new(HistoryState { entries, config })),
        };
        store.prune()?;
        Ok(store)
    }

    pub fn set_config(&self, config: HistoryConfig) -> Result<(), ExecutionHistoryError> {
        let mut state = self.state.lock().expect("execution history lock poisoned");
        state.config = config;
        let config = state.config.clone();
        let removed = prune_entries(&mut state.entries, &config);
        if !removed.is_empty() {
            write_entries(&self.path, &state.entries)?;
        }
        drop(state);
        remove_output_files(&self.data_directory, &removed);
        Ok(())
    }

    pub fn record_started(
        &self,
        execution_id: Uuid,
        command_line: String,
        server: String,
    ) -> Result<(), ExecutionHistoryError> {
        let mut state = self.state.lock().expect("execution history lock poisoned");
        state
            .entries
            .retain(|entry| entry.execution_id != execution_id);
        state.entries.push(ExecutionHistoryEntry {
            execution_id,
            command_line,
            server,
            state: ExecutionState::Running,
            started_at: unix_timestamp_ms(),
            finished_at: None,
            exit_code: None,
            success: None,
            timed_out: None,
        });
        write_entries(&self.path, &state.entries)?;
        Ok(())
    }

    pub fn record_finished(
        &self,
        execution_id: Uuid,
        state_value: ExecutionState,
        exit_code: Option<i32>,
        success: Option<bool>,
        timed_out: Option<bool>,
    ) -> Result<(), ExecutionHistoryError> {
        let mut state = self.state.lock().expect("execution history lock poisoned");
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.execution_id == execution_id)
            .ok_or(ExecutionHistoryError::NotFound(execution_id))?;
        entry.state = state_value;
        entry.finished_at = Some(unix_timestamp_ms());
        entry.exit_code = exit_code;
        entry.success = success;
        entry.timed_out = timed_out;

        let config = state.config.clone();
        let removed = prune_entries(&mut state.entries, &config);
        write_entries(&self.path, &state.entries)?;
        drop(state);
        remove_output_files(&self.data_directory, &removed);
        Ok(())
    }

    pub fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<ExecutionHistoryPage, ExecutionHistoryError> {
        let state = self.state.lock().expect("execution history lock poisoned");
        let total = state.entries.len();
        let records = state
            .entries
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        Ok(ExecutionHistoryPage {
            records,
            total,
            offset,
            limit,
        })
    }

    pub fn get(
        &self,
        execution_id: Uuid,
    ) -> Result<Option<ExecutionHistoryEntry>, ExecutionHistoryError> {
        let state = self.state.lock().expect("execution history lock poisoned");
        Ok(state
            .entries
            .iter()
            .find(|entry| entry.execution_id == execution_id)
            .cloned())
    }

    pub fn delete(&self, execution_id: Uuid) -> Result<bool, ExecutionHistoryError> {
        let mut state = self.state.lock().expect("execution history lock poisoned");
        let Some(index) = state
            .entries
            .iter()
            .position(|entry| entry.execution_id == execution_id)
        else {
            return Ok(false);
        };

        if state.entries[index].state == ExecutionState::Running {
            return Err(ExecutionHistoryError::Running(execution_id));
        }

        state.entries.remove(index);
        write_entries(&self.path, &state.entries)?;
        drop(state);
        remove_output_file(&self.data_directory, execution_id);
        Ok(true)
    }

    pub fn prune(&self) -> Result<(), ExecutionHistoryError> {
        let mut state = self.state.lock().expect("execution history lock poisoned");
        let config = state.config.clone();
        let removed = prune_entries(&mut state.entries, &config);
        if !removed.is_empty() {
            write_entries(&self.path, &state.entries)?;
        }
        drop(state);
        remove_output_files(&self.data_directory, &removed);
        Ok(())
    }
}

fn load_entries(path: &PathBuf) -> Result<Vec<ExecutionHistoryEntry>, ExecutionHistoryError> {
    match fs::read(path) {
        Ok(contents) if contents.is_empty() => Ok(Vec::new()),
        Ok(contents) => Ok(serde_json::from_slice(&contents)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_entries(
    path: &PathBuf,
    entries: &[ExecutionHistoryEntry],
) -> Result<(), ExecutionHistoryError> {
    let content = serde_json::to_vec_pretty(entries)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(&content)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn prune_entries(entries: &mut Vec<ExecutionHistoryEntry>, config: &HistoryConfig) -> Vec<Uuid> {
    let now = unix_timestamp_ms();
    let cutoff = now.saturating_sub(config.retention_days.saturating_mul(MILLIS_PER_DAY));
    let mut removed = Vec::new();

    entries.retain(|entry| {
        let keep = entry.state == ExecutionState::Running || entry.started_at >= cutoff;
        if !keep {
            removed.push(entry.execution_id);
        }
        keep
    });

    let completed_count = entries
        .iter()
        .filter(|entry| entry.state != ExecutionState::Running)
        .count();
    let mut completed_to_remove = completed_count.saturating_sub(config.max_records);
    let mut retained = Vec::with_capacity(entries.len());
    for entry in entries.drain(..) {
        if entry.state != ExecutionState::Running && completed_to_remove > 0 {
            removed.push(entry.execution_id);
            completed_to_remove -= 1;
        } else {
            retained.push(entry);
        }
    }
    *entries = retained;

    removed
}

fn remove_output_files(data_directory: &DataDirectory, execution_ids: &[Uuid]) {
    for execution_id in execution_ids {
        remove_output_file(data_directory, *execution_id);
    }
}

fn remove_output_file(data_directory: &DataDirectory, execution_id: Uuid) {
    let Ok(path) = data_directory.execution_output_path(execution_id) else {
        return;
    };

    if let Err(error) = fs::remove_file(&path)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(
            execution_id = %execution_id,
            path = %path.display(),
            error = %error,
            "Failed to remove execution output file"
        );
    }
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_config() -> HistoryConfig {
        HistoryConfig {
            retention_days: 30,
            max_records: 2,
        }
    }

    #[test]
    fn prune_keeps_running_entries_and_newest_completed_entries() {
        let id_one = Uuid::new_v4();
        let id_two = Uuid::new_v4();
        let id_three = Uuid::new_v4();
        let id_four = Uuid::new_v4();
        let mut entries = vec![
            ExecutionHistoryEntry {
                execution_id: id_one,
                command_line: "one".to_string(),
                server: "host".to_string(),
                state: ExecutionState::Completed,
                started_at: unix_timestamp_ms(),
                finished_at: Some(unix_timestamp_ms()),
                exit_code: Some(0),
                success: Some(true),
                timed_out: Some(false),
            },
            ExecutionHistoryEntry {
                execution_id: id_two,
                command_line: "two".to_string(),
                server: "host".to_string(),
                state: ExecutionState::Completed,
                started_at: unix_timestamp_ms(),
                finished_at: Some(unix_timestamp_ms()),
                exit_code: Some(0),
                success: Some(true),
                timed_out: Some(false),
            },
            ExecutionHistoryEntry {
                execution_id: id_three,
                command_line: "three".to_string(),
                server: "host".to_string(),
                state: ExecutionState::Completed,
                started_at: unix_timestamp_ms(),
                finished_at: Some(unix_timestamp_ms()),
                exit_code: Some(0),
                success: Some(true),
                timed_out: Some(false),
            },
            ExecutionHistoryEntry {
                execution_id: id_four,
                command_line: "three".to_string(),
                server: "host".to_string(),
                state: ExecutionState::Running,
                started_at: unix_timestamp_ms(),
                finished_at: None,
                exit_code: None,
                success: None,
                timed_out: None,
            },
        ];

        let removed = prune_entries(&mut entries, &history_config());

        assert_eq!(entries.len(), 3);
        assert!(removed.contains(&id_one));
        assert!(entries.iter().any(|entry| entry.execution_id == id_two));
        assert!(entries.iter().any(|entry| entry.execution_id == id_three));
        assert!(entries.iter().any(|entry| entry.execution_id == id_four));
    }
}
