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

const APP_DIR_NAME: &str = ".host-bridge";
const EXECUTIONS_DIR_NAME: &str = "executions";
const EXECUTION_HISTORY_FILE_NAME: &str = "history.json";
const LOGS_DIR_NAME: &str = "logs";
const PASSWORDS_DIR_NAME: &str = "passwords";
const DEFAULT_LOG_FILE_NAME: &str = "host-bridge.log";
const TEMP_LOG_FILE_PREFIX: &str = "host-bridge-mcp-";
const SSH_PASSWORD_FILE_PREFIX: &str = "ssh-password-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataDirectory {
    root: PathBuf,
}

impl DataDirectory {
    pub(crate) fn new(configured_path: Option<&str>) -> io::Result<Self> {
        let root = configured_path
            .map(resolve_configured_path)
            .unwrap_or_else(default_data_dir);
        ensure_directory(&root)?;
        Ok(Self { root })
    }

    #[cfg(test)]
    pub(crate) fn from_root(root: PathBuf) -> io::Result<Self> {
        ensure_directory(&root)?;
        Ok(Self { root })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn execution_output_path(&self, execution_id: Uuid) -> io::Result<PathBuf> {
        Ok(self
            .subdir(EXECUTIONS_DIR_NAME)?
            .join(format!("{execution_id}.log")))
    }

    pub(crate) fn execution_history_path(&self) -> io::Result<PathBuf> {
        Ok(self
            .subdir(EXECUTIONS_DIR_NAME)?
            .join(EXECUTION_HISTORY_FILE_NAME))
    }

    pub(crate) fn runtime_log_path(&self) -> io::Result<PathBuf> {
        Ok(self.subdir(LOGS_DIR_NAME)?.join(DEFAULT_LOG_FILE_NAME))
    }

    pub(crate) fn temporary_log_path(&self) -> io::Result<PathBuf> {
        Ok(self
            .subdir(LOGS_DIR_NAME)?
            .join(format!("{TEMP_LOG_FILE_PREFIX}{}.log", Uuid::new_v4())))
    }

    pub(crate) fn ssh_password_file_path(&self, server_name: &str) -> io::Result<PathBuf> {
        Ok(self.subdir(PASSWORDS_DIR_NAME)?.join(format!(
            "{SSH_PASSWORD_FILE_PREFIX}{}.txt",
            encode_file_name_component(server_name)
        )))
    }

    fn subdir(&self, name: &str) -> io::Result<PathBuf> {
        let path = self.root.join(name);
        ensure_directory(&path)?;
        Ok(path)
    }
}

fn default_data_dir() -> PathBuf {
    resolve_home_dir()
        .map(|home| home.join(APP_DIR_NAME))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(APP_DIR_NAME)
        })
}

fn resolve_configured_path(value: &str) -> PathBuf {
    let value = value.trim();
    let path = if value == "~" {
        resolve_home_dir().unwrap_or_else(|| PathBuf::from(value))
    } else if let Some(relative) = value.strip_prefix("~/") {
        resolve_home_dir()
            .map(|home| home.join(relative))
            .unwrap_or_else(|| PathBuf::from(value))
    } else {
        PathBuf::from(value)
    };

    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn encode_file_name_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("-{byte:02x}"));
        }
    }
    if encoded.is_empty() {
        "target".to_string()
    } else {
        encoded
    }
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
        assert_eq!(DEFAULT_LOG_FILE_NAME, "host-bridge.log");
    }

    #[test]
    fn runtime_log_path_uses_logs_subdir() {
        let root = unique_temp_path("runtime-log");
        let directory = DataDirectory { root: root.clone() };

        assert_eq!(
            directory.runtime_log_path().expect("runtime log path"),
            root.join("logs/host-bridge.log")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn temporary_log_path_uses_logs_subdir_with_unique_name() {
        let root = unique_temp_path("temporary-log");
        let directory = DataDirectory { root: root.clone() };
        let path = directory.temporary_log_path().expect("temporary log path");

        let logs_dir = root.join("logs");
        assert_eq!(path.parent(), Some(logs_dir.as_path()));
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("temporary log path should have a file name");
        assert!(file_name.starts_with(TEMP_LOG_FILE_PREFIX));
        assert!(file_name.ends_with(".log"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn configured_data_directory_uses_exact_root() {
        let root = unique_temp_path("configured");
        let directory = DataDirectory::from_root(root.clone())
            .expect("configured data directory should initialize");

        assert_eq!(directory.root(), root.as_path());
        assert_eq!(
            directory.execution_history_path().expect("history path"),
            root.join("executions/history.json")
        );

        let _ = fs::remove_dir_all(root);
    }
}
