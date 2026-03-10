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

use std::fs::DirBuilder;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const APP_DIR_NAME: &str = ".host-bridge-mcp";
const EXECUTIONS_DIR_NAME: &str = "executions";
const LOGS_DIR_NAME: &str = "logs";
const DEFAULT_LOG_FILE_NAME: &str = "host-bridge-mcp.log";

pub(crate) fn execution_output_path(execution_id: Uuid) -> io::Result<PathBuf> {
    let executions_dir = resolve_data_subdir(EXECUTIONS_DIR_NAME)?;
    Ok(executions_dir.join(format!("{execution_id}.log")))
}

pub(crate) fn default_persisted_log_path() -> io::Result<PathBuf> {
    let logs_dir = resolve_data_subdir(LOGS_DIR_NAME)?;
    Ok(logs_dir.join(DEFAULT_LOG_FILE_NAME))
}

fn resolve_data_subdir(name: &str) -> io::Result<PathBuf> {
    let candidates = resolve_base_data_dir_candidates()?;
    resolve_data_subdir_from_candidates(&candidates, name)
}

fn resolve_data_subdir_from_candidates(candidates: &[PathBuf], name: &str) -> io::Result<PathBuf> {
    let mut failures = Vec::new();

    for base_dir in candidates {
        match ensure_data_subdir(base_dir, name) {
            Ok(path) => return Ok(path),
            Err(error) => failures.push(format!("{}: {error}", base_dir.display())),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "failed to initialize host-bridge data directory candidates: {}",
            failures.join("; ")
        ),
    ))
}

fn ensure_data_subdir(base_dir: &Path, name: &str) -> io::Result<PathBuf> {
    ensure_directory(&base_dir)?;

    let subdir = base_dir.join(name);
    ensure_directory(&subdir)?;

    Ok(subdir)
}

fn resolve_base_data_dir_candidates() -> io::Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();

    if let Some(home_dir) = resolve_home_dir() {
        push_unique_path(&mut candidates, home_dir.join(APP_DIR_NAME));
    }

    if let Ok(executable_path) = std::env::current_exe() {
        if let Some(parent) = executable_path.parent() {
            push_unique_path(&mut candidates, parent.join(APP_DIR_NAME));
        }
    }

    push_unique_path(&mut candidates, std::env::current_dir()?.join(APP_DIR_NAME));
    Ok(candidates)
}

#[cfg(not(windows))]
fn resolve_home_dir() -> Option<PathBuf> {
    resolve_home_env_dir()
}

#[cfg(windows)]
fn resolve_home_dir() -> Option<PathBuf> {
    resolve_windows_home_dir().or_else(resolve_home_env_dir)
}

fn resolve_home_env_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

#[cfg(windows)]
fn resolve_windows_home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let home_drive = std::env::var_os("HOMEDRIVE")?;
            let home_path = std::env::var_os("HOMEPATH")?;
            if home_drive.is_empty() || home_path.is_empty() {
                return None;
            }

            let mut combined = PathBuf::from(home_drive);
            combined.push(home_path);
            Some(combined)
        })
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("path '{}' must not be a symbolic link", path.display()),
                ));
            }

            if metadata.is_dir() {
                return Ok(());
            }

            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("path '{}' exists but is not a directory", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut builder = DirBuilder::new();
    builder.recursive(true);

    #[cfg(unix)]
    builder.mode(0o700);

    builder.create(path)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if paths.iter().all(|existing| existing != &candidate) {
        paths.push(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("host-bridge-mcp-{label}-{unique}"))
    }

    #[test]
    fn execution_output_file_name_uses_execution_id() {
        let execution_id =
            Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").expect("uuid should parse");

        let file_name = format!("{execution_id}.log");

        assert_eq!(file_name, "123e4567-e89b-12d3-a456-426614174000.log");
    }

    #[test]
    fn default_log_file_name_is_stable() {
        assert_eq!(DEFAULT_LOG_FILE_NAME, "host-bridge-mcp.log");
    }

    #[test]
    fn data_subdir_falls_back_to_next_candidate_after_failure() {
        let blocked_path = unique_temp_path("blocked");
        let valid_base = unique_temp_path("valid");
        fs::write(&blocked_path, "blocked").expect("blocked path should be created as file");

        let resolved = resolve_data_subdir_from_candidates(
            &[blocked_path.clone(), valid_base.clone()],
            EXECUTIONS_DIR_NAME,
        )
            .expect("fallback candidate should succeed");

        assert_eq!(resolved, valid_base.join(EXECUTIONS_DIR_NAME));

        let _ = fs::remove_file(blocked_path);
        let _ = fs::remove_dir_all(valid_base);
    }
}
