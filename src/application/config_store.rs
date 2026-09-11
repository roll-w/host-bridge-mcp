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

use crate::config::{
    AccessConfig, AppConfig, CommandPolicyConfig, CommandRuleConfig, ConfigError, ExecutionConfig,
    ExecutionServerConfig, HistoryConfig, LoggingConfig, PolicyAction, ResolvedConfigPath,
    ServerConfig, SshAuthConfig, SshAuthType, TargetPlatform,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub path: String,
    pub raw: String,
    pub config: AppConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigStoreError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("failed to serialize default configuration: {0}")]
    Serialize(String),
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VisualConfigPatch {
    pub data_dir: Option<String>,
    pub tui: Option<bool>,
    pub web: Option<bool>,
    pub server_address: Option<String>,
    pub api_key_env: Option<String>,
    pub log_retention_days: Option<u64>,
    pub default_action: Option<String>,
    pub default_working_directory: Option<String>,
    pub default_server: Option<String>,
    pub target_platform: Option<String>,
    pub default_timeout_ms: Option<u64>,
    pub max_timeout_ms: Option<u64>,
    pub history_retention_days: Option<u64>,
    pub history_max_records: Option<usize>,
    pub commands: Option<Vec<CommandPolicyConfig>>,
    pub servers: Option<Vec<ExecutionServerConfig>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlAppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    data_dir: Option<String>,
    tui: bool,
    web: bool,
    server: YamlServerConfig,
    execution: YamlExecutionConfig,
    logging: LoggingConfig,
    history: HistoryConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlServerConfig {
    #[serde(rename = "address")]
    bind_address: String,
    #[serde(skip_serializing_if = "YamlAccessConfig::is_empty")]
    access: YamlAccessConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlAccessConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key_env: Option<String>,
}

impl YamlAccessConfig {
    fn is_empty(&self) -> bool {
        self.api_key_env.is_none()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlExecutionConfig {
    default_action: PolicyAction,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commands: Vec<YamlCommandPolicyConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_working_directory: Option<String>,
    default_server: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    servers: Vec<YamlExecutionServerConfig>,
    target_platform: TargetPlatform,
    default_timeout_ms: u64,
    max_timeout_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "transport", rename_all_fields = "kebab-case")]
enum YamlExecutionServerConfig {
    #[serde(rename = "host")]
    Host {
        name: String,
        target_platform: TargetPlatform,
    },
    #[serde(rename = "ssh")]
    Ssh {
        name: String,
        host: String,
        port: u16,
        user: String,
        target_platform: TargetPlatform,
        auth: YamlSshAuthConfig,
        #[serde(skip_serializing_if = "Option::is_none")]
        known_hosts_file: Option<String>,
        verify_host_key: bool,
        connection_idle_timeout_ms: u64,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlSshAuthConfig {
    #[serde(rename = "type")]
    kind: SshAuthType,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#ref: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlCommandPolicyConfig {
    command: String,
    action: PolicyAction,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_working_directory: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rules: Vec<YamlCommandRuleConfig>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct YamlCommandRuleConfig {
    args_prefix: Vec<String>,
    action: PolicyAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_working_directory: Option<String>,
}

impl From<&AppConfig> for YamlAppConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            data_dir: config.data_dir.clone(),
            tui: config.tui,
            web: config.web,
            server: YamlServerConfig::from(&config.server),
            execution: YamlExecutionConfig::from(&config.execution),
            logging: config.logging.clone(),
            history: config.history.clone(),
        }
    }
}

impl From<&ServerConfig> for YamlServerConfig {
    fn from(server: &ServerConfig) -> Self {
        Self {
            bind_address: server.bind_address.clone(),
            access: YamlAccessConfig::from(&server.access),
        }
    }
}

impl From<&AccessConfig> for YamlAccessConfig {
    fn from(access: &AccessConfig) -> Self {
        Self {
            api_key_env: access.api_key_env.clone(),
        }
    }
}

impl From<&ExecutionConfig> for YamlExecutionConfig {
    fn from(execution: &ExecutionConfig) -> Self {
        Self {
            default_action: execution.default_action,
            commands: execution
                .commands
                .iter()
                .map(YamlCommandPolicyConfig::from)
                .collect(),
            default_working_directory: execution.default_working_directory.clone(),
            default_server: execution.default_server.clone(),
            servers: execution
                .servers
                .iter()
                .map(YamlExecutionServerConfig::from)
                .collect(),
            target_platform: execution.target_platform,
            default_timeout_ms: execution.default_timeout_ms,
            max_timeout_ms: execution.max_timeout_ms,
        }
    }
}

impl From<&ExecutionServerConfig> for YamlExecutionServerConfig {
    fn from(server: &ExecutionServerConfig) -> Self {
        match server {
            ExecutionServerConfig::Host {
                name,
                target_platform,
            } => Self::Host {
                name: name.clone(),
                target_platform: *target_platform,
            },
            ExecutionServerConfig::Ssh {
                name,
                host,
                port,
                user,
                target_platform,
                auth,
                known_hosts_file,
                verify_host_key,
                connection_idle_timeout_ms,
            } => Self::Ssh {
                name: name.clone(),
                host: host.clone(),
                port: *port,
                user: user.clone(),
                target_platform: *target_platform,
                auth: YamlSshAuthConfig::from(auth),
                known_hosts_file: known_hosts_file.clone(),
                verify_host_key: *verify_host_key,
                connection_idle_timeout_ms: *connection_idle_timeout_ms,
            },
        }
    }
}

impl From<&SshAuthConfig> for YamlSshAuthConfig {
    fn from(auth: &SshAuthConfig) -> Self {
        Self {
            kind: auth.kind,
            r#ref: auth.r#ref.clone(),
        }
    }
}

impl From<&CommandPolicyConfig> for YamlCommandPolicyConfig {
    fn from(command: &CommandPolicyConfig) -> Self {
        Self {
            command: command.command.clone(),
            action: command.action,
            targets: command.targets.clone(),
            default_working_directory: command.default_working_directory.clone(),
            rules: command
                .rules
                .iter()
                .map(YamlCommandRuleConfig::from)
                .collect(),
        }
    }
}

impl From<&CommandRuleConfig> for YamlCommandRuleConfig {
    fn from(rule: &CommandRuleConfig) -> Self {
        Self {
            args_prefix: rule.args_prefix.clone(),
            action: rule.action,
            default_working_directory: rule.default_working_directory.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ConfigStore {
    path: String,
    write_lock: Arc<Mutex<()>>,
}

impl ConfigStore {
    pub fn new(path: ResolvedConfigPath) -> Self {
        Self {
            path: path.path,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn snapshot(&self, fallback: &AppConfig) -> Result<ConfigSnapshot, ConfigStoreError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                serialize_config_yaml(fallback)?
            }
            Err(error) => return Err(error.into()),
        };
        let config = AppConfig::parse_raw(&self.path, &raw)?;
        Ok(ConfigSnapshot {
            path: self.path.clone(),
            raw,
            config,
        })
    }

    pub fn save_raw(&self, raw: String) -> Result<ConfigSnapshot, ConfigStoreError> {
        let config = AppConfig::parse_raw(&self.path, &raw)?;
        self.write(&raw)?;
        Ok(ConfigSnapshot {
            path: self.path.clone(),
            raw,
            config,
        })
    }

    pub fn save_visual(
        &self,
        patch: VisualConfigPatch,
        fallback: &AppConfig,
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        let current = self.snapshot(fallback)?;
        let updated_raw = apply_visual_patch(&current.raw, &patch)?;
        self.save_raw(updated_raw)
    }

    pub fn write_password_file(&self, path: &str, password: &str) -> Result<(), ConfigStoreError> {
        let _guard = self.write_lock.lock().expect("config write lock poisoned");
        let path = Path::new(path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path)?;
        file.write_all(password.as_bytes())?;
        file.flush()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    fn write(&self, raw: &str) -> Result<(), ConfigStoreError> {
        let _guard = self.write_lock.lock().expect("config write lock poisoned");
        let path = Path::new(&self.path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path)?;
        file.write_all(raw.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

fn apply_visual_patch(raw: &str, patch: &VisualConfigPatch) -> Result<String, ConfigStoreError> {
    let mut lines = raw
        .split_inclusive('\n')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let updates = [
        (
            "data-dir",
            patch
                .data_dir
                .as_ref()
                .map(|value| yaml_optional_string(value)),
        ),
        ("tui", patch.tui.map(|value| value.to_string())),
        ("web", patch.web.map(|value| value.to_string())),
        (
            "server.address",
            patch
                .server_address
                .as_ref()
                .map(|value| yaml_string(value)),
        ),
        (
            "server.access.api-key-env",
            patch
                .api_key_env
                .as_ref()
                .map(|value| yaml_optional_string(value)),
        ),
        (
            "logging.retention-days",
            patch.log_retention_days.map(|value| value.to_string()),
        ),
        (
            "execution.default-action",
            patch
                .default_action
                .as_ref()
                .map(|value| yaml_string(value)),
        ),
        (
            "execution.default-working-directory",
            patch
                .default_working_directory
                .as_ref()
                .map(|value| yaml_optional_string(value)),
        ),
        (
            "execution.default-server",
            patch
                .default_server
                .as_ref()
                .map(|value| yaml_string(value)),
        ),
        (
            "execution.target-platform",
            patch
                .target_platform
                .as_ref()
                .map(|value| yaml_string(value)),
        ),
        (
            "execution.default-timeout-ms",
            patch.default_timeout_ms.map(|value| value.to_string()),
        ),
        (
            "execution.max-timeout-ms",
            patch.max_timeout_ms.map(|value| value.to_string()),
        ),
        (
            "history.retention-days",
            patch.history_retention_days.map(|value| value.to_string()),
        ),
        (
            "history.max-records",
            patch.history_max_records.map(|value| value.to_string()),
        ),
    ];

    for (path, value) in updates {
        if let Some(value) = value {
            if value == "null" {
                let _ = remove_yaml_scalar(&mut lines, path);
            } else if !replace_yaml_scalar(&mut lines, path, &value) {
                let _ = insert_yaml_scalar(&mut lines, path, &value);
            }
        }
    }

    if let Some(commands) = patch.commands.as_ref() {
        let value = serialize_command_policies(commands)?;
        replace_or_insert_yaml_block(&mut lines, "execution.commands", &value);
    }

    if let Some(servers) = patch.servers.as_ref() {
        let value = serialize_execution_servers(servers)?;
        replace_or_insert_yaml_block(&mut lines, "execution.servers", &value);
    }

    for path in [
        "data-dir",
        "server.access.api-key-env",
        "execution.default-working-directory",
    ] {
        let _ = remove_empty_yaml_scalar(&mut lines, path);
    }
    for path in ["server.access", "execution.commands", "execution.servers"] {
        let _ = remove_empty_yaml_mapping(&mut lines, path);
    }

    Ok(lines.concat())
}

fn serialize_yaml_value<T: Serialize>(value: &T) -> Result<String, ConfigStoreError> {
    serde_saphyr::to_string(value).map_err(|error| ConfigStoreError::Serialize(error.to_string()))
}

fn serialize_config_yaml(config: &AppConfig) -> Result<String, ConfigStoreError> {
    serialize_yaml_value(&YamlAppConfig::from(config))
}

fn serialize_command_policies(
    commands: &[CommandPolicyConfig],
) -> Result<String, ConfigStoreError> {
    let commands = commands
        .iter()
        .map(YamlCommandPolicyConfig::from)
        .collect::<Vec<_>>();
    serialize_yaml_value(&commands)
}

fn serialize_execution_servers(
    servers: &[ExecutionServerConfig],
) -> Result<String, ConfigStoreError> {
    let servers = servers
        .iter()
        .map(YamlExecutionServerConfig::from)
        .collect::<Vec<_>>();
    serialize_yaml_value(&servers)
}

fn replace_or_insert_yaml_block(lines: &mut Vec<String>, path: &str, value: &str) {
    if value.trim() == "[]" {
        let _ = remove_yaml_block(lines, path);
        return;
    }

    if !replace_yaml_block(lines, path, value) {
        let _ = insert_yaml_block(lines, path, value);
    }
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "\\\"")))
}

fn yaml_optional_string(value: &str) -> String {
    if value.trim().is_empty() {
        "null".to_string()
    } else {
        yaml_string(value)
    }
}

fn replace_yaml_scalar(lines: &mut [String], path: &str, value: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, _, colon_index)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    replace_scalar_value(&mut lines[line_index], colon_index, value);
    true
}

fn remove_yaml_scalar(lines: &mut Vec<String>, path: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, _, _)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    lines.remove(line_index);
    true
}

fn remove_empty_yaml_scalar(lines: &mut Vec<String>, path: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, _, colon_index)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    if !is_yaml_null_value(yaml_scalar_value(&lines[line_index], colon_index)) {
        return false;
    }

    lines.remove(line_index);
    true
}

fn replace_yaml_block(lines: &mut Vec<String>, path: &str, value: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, indent, colon_index)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    let block_end = mapping_block_end(lines, line_index, indent);
    let newline = line_ending(lines);
    let value = value.trim_end_matches(['\r', '\n']);
    let mut replacement = Vec::new();

    if value.trim() == "[]" {
        let mut line = lines[line_index].clone();
        replace_scalar_value(&mut line, colon_index, "[]");
        replacement.push(line);
    } else {
        replacement.push(yaml_mapping_header(
            &lines[line_index],
            colon_index,
            newline,
        ));
        let value_indent = " ".repeat(indent + 2);
        replacement.extend(
            value
                .lines()
                .map(|line| format!("{value_indent}{line}{newline}")),
        );
    }

    lines.splice(line_index..block_end, replacement);
    true
}

fn remove_yaml_block(lines: &mut Vec<String>, path: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, indent, _)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    let block_end = mapping_block_end(lines, line_index, indent);
    lines.drain(line_index..block_end);
    true
}

fn remove_empty_yaml_mapping(lines: &mut Vec<String>, path: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    let Some((line_index, indent, colon_index)) = find_yaml_mapping(lines, &wanted) else {
        return false;
    };

    let value = yaml_scalar_value(&lines[line_index], colon_index);
    if !value.is_empty()
        && !is_yaml_null_value(value)
        && !matches!(value, "[]" | "[ ]" | "{}" | "{ }")
    {
        return false;
    }

    let block_end = mapping_block_end(lines, line_index, indent);
    let has_content = lines
        .iter()
        .skip(line_index + 1)
        .take(block_end.saturating_sub(line_index + 1))
        .any(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        });
    if has_content {
        return false;
    }

    lines.drain(line_index..block_end);
    true
}

fn yaml_scalar_value(line: &str, colon_index: usize) -> &str {
    let content = line.trim_end_matches(['\r', '\n']);
    let comment_index = find_inline_comment(content, colon_index + 1).unwrap_or(content.len());
    content[colon_index + 1..comment_index].trim()
}

fn is_yaml_null_value(value: &str) -> bool {
    matches!(value, "" | "~" | "null" | "Null" | "NULL")
}

fn yaml_mapping_header(line: &str, colon_index: usize, newline: &str) -> String {
    let content = line.trim_end_matches(['\r', '\n']);
    let prefix = &content[..=colon_index];
    let Some(comment_index) = find_inline_comment(content, colon_index + 1) else {
        return format!("{prefix}{newline}");
    };

    format!("{prefix} {}{newline}", &content[comment_index..])
}

fn insert_yaml_scalar(lines: &mut Vec<String>, path: &str, value: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    if wanted.is_empty() {
        return false;
    }

    let parent_path = &wanted[..wanted.len() - 1];
    if !ensure_yaml_mapping(lines, parent_path) {
        return false;
    }

    insert_yaml_line(lines, parent_path, wanted[wanted.len() - 1], Some(value))
}

fn insert_yaml_block(lines: &mut Vec<String>, path: &str, value: &str) -> bool {
    let wanted = path.split('.').collect::<Vec<_>>();
    if wanted.is_empty() {
        return false;
    }

    let parent_path = &wanted[..wanted.len() - 1];
    if !ensure_yaml_mapping(lines, parent_path) {
        return false;
    }

    let newline = line_ending(lines);
    let (parent_indent, insertion_index) = if parent_path.is_empty() {
        if lines.len() == 1 && lines[0].is_empty() {
            lines.clear();
        }
        (0, lines.len())
    } else {
        let Some((parent_index, parent_indent, _)) = find_yaml_mapping(lines, parent_path) else {
            return false;
        };
        (
            child_indent(lines, parent_index, parent_indent),
            mapping_block_end(lines, parent_index, parent_indent),
        )
    };

    if insertion_index > 0 && !has_line_ending(&lines[insertion_index - 1]) {
        lines[insertion_index - 1].push_str(newline);
    }

    let value = value.trim_end_matches(['\r', '\n']);
    let key = wanted[wanted.len() - 1];
    let replacement = if value.trim() == "[]" {
        vec![format!("{}{key}: []{newline}", " ".repeat(parent_indent))]
    } else {
        let mut replacement = vec![format!("{}{key}:{newline}", " ".repeat(parent_indent))];
        let value_indent = " ".repeat(parent_indent + 2);
        replacement.extend(
            value
                .lines()
                .map(|line| format!("{value_indent}{line}{newline}")),
        );
        replacement
    };

    lines.splice(insertion_index..insertion_index, replacement);
    true
}

fn ensure_yaml_mapping(lines: &mut Vec<String>, path: &[&str]) -> bool {
    if path.is_empty() || find_yaml_mapping(lines, path).is_some() {
        return true;
    }

    let parent_path = &path[..path.len() - 1];
    if !ensure_yaml_mapping(lines, parent_path) {
        return false;
    }

    insert_yaml_line(lines, parent_path, path[path.len() - 1], None)
}

fn insert_yaml_line(
    lines: &mut Vec<String>,
    parent_path: &[&str],
    key: &str,
    value: Option<&str>,
) -> bool {
    let newline = line_ending(lines);
    let (parent_indent, insertion_index) = if parent_path.is_empty() {
        if lines.len() == 1 && lines[0].is_empty() {
            lines.clear();
        }
        (0, lines.len())
    } else {
        let Some((parent_index, parent_indent, _)) = find_yaml_mapping(lines, parent_path) else {
            return false;
        };
        (
            child_indent(lines, parent_index, parent_indent),
            mapping_block_end(lines, parent_index, parent_indent),
        )
    };

    if insertion_index > 0 && !has_line_ending(&lines[insertion_index - 1]) {
        lines[insertion_index - 1].push_str(newline);
    }

    let line = match value {
        Some(value) => format!("{}{key}: {value}{newline}", " ".repeat(parent_indent)),
        None => format!("{}{key}:{newline}", " ".repeat(parent_indent)),
    };
    lines.insert(insertion_index, line);
    true
}

fn find_yaml_mapping(lines: &[String], wanted: &[&str]) -> Option<(usize, usize, usize)> {
    if wanted.is_empty() {
        return None;
    }

    let mut stack: Vec<(usize, String)> = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let Some((indent, key, colon_index)) = mapping_key(line) else {
            continue;
        };

        while stack
            .last()
            .is_some_and(|(parent_indent, _)| *parent_indent >= indent)
        {
            stack.pop();
        }
        stack.push((indent, key.to_string()));

        if stack.len() == wanted.len()
            && stack
                .iter()
                .map(|(_, key)| key.as_str())
                .eq(wanted.iter().copied())
        {
            return Some((line_index, indent, colon_index));
        }
    }

    None
}

fn mapping_block_end(lines: &[String], parent_index: usize, parent_indent: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(parent_index + 1)
        .find_map(|(line_index, line)| {
            mapping_key(line)
                .filter(|(indent, _, _)| *indent <= parent_indent)
                .map(|_| line_index)
        })
        .unwrap_or(lines.len())
}

fn child_indent(lines: &[String], parent_index: usize, parent_indent: usize) -> usize {
    let block_end = mapping_block_end(lines, parent_index, parent_indent);
    lines
        .iter()
        .skip(parent_index + 1)
        .take(block_end.saturating_sub(parent_index + 1))
        .filter_map(|line| mapping_key(line).map(|(indent, _, _)| indent))
        .filter(|indent| *indent > parent_indent)
        .min()
        .unwrap_or(parent_indent + 2)
}

fn line_ending(lines: &[String]) -> &'static str {
    if lines.iter().any(|line| line.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    }
}

fn has_line_ending(line: &str) -> bool {
    line.ends_with('\n') || line.ends_with('\r')
}

fn mapping_key(line: &str) -> Option<(usize, &str, usize)> {
    let content = line.trim_end_matches(['\r', '\n']);
    let trimmed = content.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }

    let indent = content.len() - trimmed.len();
    let colon_index = trimmed.find(':')?;
    let key = trimmed[..colon_index].trim();
    if key.is_empty() || key.contains(['"', '\'']) {
        return None;
    }

    Some((indent, key, indent + colon_index))
}

fn replace_scalar_value(line: &mut String, colon_index: usize, value: &str) {
    let newline = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let content_end = line.len() - newline.len();
    let content = &line[..content_end];
    let comment_index = find_inline_comment(content, colon_index + 1);
    let (value_end, comment) = match comment_index {
        Some(index) => (index, &content[index..]),
        None => (content.len(), ""),
    };
    let prefix = &content[..colon_index + 1];
    let value_region = &content[colon_index + 1..value_end];
    let leading_end = value_region.len() - value_region.trim_start().len();
    let leading_whitespace = &value_region[..leading_end];
    let trailing_start = value_region.trim_end().len();
    let trailing_whitespace = if comment_index.is_some() {
        &value_region[trailing_start..]
    } else {
        ""
    };
    let leading_whitespace = if leading_whitespace.is_empty() {
        " ".to_string()
    } else {
        leading_whitespace.to_string()
    };

    *line = format!("{prefix}{leading_whitespace}{value}{trailing_whitespace}{comment}{newline}");
}

fn find_inline_comment(content: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in content
        .char_indices()
        .skip_while(|(index, _)| *index < start)
    {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('"'), '\\') => escaped = true,
            (Some(current), value) if current == value => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '#')
                if index == start
                    || content[..index]
                        .chars()
                        .last()
                        .is_some_and(char::is_whitespace) =>
            {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_scalar_patch_preserves_comments_and_layout() {
        let raw = "# keep this\nserver:\n  # keep address note\n  address: 127.0.0.1:8787 # keep inline\nlogging:\n  retention-days: 30\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                server_address: Some("0.0.0.0:9000".to_string()),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual patch should serialize");

        assert!(patched.contains("# keep this"));
        assert!(patched.contains("# keep address note"));
        assert!(patched.contains("address: \"0.0.0.0:9000\" # keep inline"));
        assert!(patched.contains("retention-days: 30"));
    }

    #[test]
    fn visual_scalar_patch_inserts_missing_fields_without_reformatting_existing_sections() {
        let raw = "# keep this\nserver:\n  address: 127.0.0.1:8787\nlogging:\n  retention-days: 30\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                data_dir: Some("/tmp/host-bridge-data".to_string()),
                tui: Some(false),
                web: Some(false),
                api_key_env: Some("HOST_BRIDGE_API_KEY".to_string()),
                log_retention_days: Some(14),
                default_working_directory: Some("/workspace".to_string()),
                history_retention_days: Some(14),
                history_max_records: Some(250),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual patch should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("inserted visual fields should remain valid configuration");
        assert_eq!(
            config.server.access.api_key_env.as_deref(),
            Some("HOST_BRIDGE_API_KEY")
        );
        assert_eq!(config.data_dir.as_deref(), Some("/tmp/host-bridge-data"));
        assert!(!config.tui);
        assert!(!config.web);
        assert_eq!(config.logging.retention_days, 14);
        assert_eq!(
            config.execution.default_working_directory.as_deref(),
            Some("/workspace")
        );
        assert_eq!(config.history.retention_days, 14);
        assert_eq!(config.history.max_records, 250);
        assert!(patched.starts_with("# keep this\nserver:\n"));
        assert!(patched.contains("retention-days: 14\n"));
    }

    #[test]
    fn visual_patch_updates_command_policies_and_servers() {
        let raw = "server:\n  address: 127.0.0.1:8787\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                commands: Some(vec![CommandPolicyConfig {
                    command: "cargo".to_string(),
                    action: PolicyAction::Allow,
                    targets: Vec::new(),
                    default_working_directory: None,
                    rules: vec![],
                }]),
                servers: Some(vec![ExecutionServerConfig::Host {
                    name: "builder".to_string(),
                    target_platform: TargetPlatform::Linux,
                }]),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual collections should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("visual collections should remain valid configuration");
        assert_eq!(config.execution.commands.len(), 1);
        assert_eq!(config.execution.commands[0].command, "cargo");
        assert_eq!(config.execution.servers.len(), 1);
        assert_eq!(config.execution.servers[0].name(), "builder");
        assert!(patched.contains("  commands:\n    - command: cargo\n      action: allow\n"));
        assert!(!patched.contains("targets: []"));
        assert!(!patched.contains("default-working-directory: null"));
        assert!(!patched.contains("rules: []"));
    }

    #[test]
    fn visual_patch_preserves_non_empty_command_policy_fields() {
        let raw = "server:\n  address: 127.0.0.1:8787\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                commands: Some(vec![CommandPolicyConfig {
                    command: "cargo".to_string(),
                    action: PolicyAction::Allow,
                    targets: vec!["host".to_string()],
                    default_working_directory: Some("/workspace".to_string()),
                    rules: vec![CommandRuleConfig {
                        args_prefix: vec!["publish".to_string()],
                        action: PolicyAction::Confirm,
                        default_working_directory: None,
                    }],
                }]),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual command policies should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("serialized command policies should remain valid configuration");
        let command = &config.execution.commands[0];
        assert_eq!(command.targets, vec!["host"]);
        assert_eq!(
            command.default_working_directory.as_deref(),
            Some("/workspace")
        );
        assert_eq!(command.rules.len(), 1);
        assert_eq!(command.rules[0].args_prefix, vec!["publish"]);
        assert!(patched.contains("targets:\n"));
        assert!(patched.contains("default-working-directory: /workspace\n"));
        assert!(patched.contains("rules:\n"));
        assert!(!patched.contains("default-working-directory: null"));
    }

    #[test]
    fn visual_patch_replaces_existing_collections_with_yaml_blocks() {
        let raw = "server:\n  address: 127.0.0.1:8787\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n  commands:\n    - command: old\n      action: confirm\n      targets: []\n      default-working-directory: null\n      rules: []\n  servers:\n    - transport: host\n      name: old\n      target-platform: auto\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                commands: Some(vec![CommandPolicyConfig {
                    command: "cargo".to_string(),
                    action: PolicyAction::Allow,
                    targets: Vec::new(),
                    default_working_directory: None,
                    rules: vec![],
                }]),
                servers: Some(vec![ExecutionServerConfig::Host {
                    name: "builder".to_string(),
                    target_platform: TargetPlatform::Linux,
                }]),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual collections should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("replaced visual collections should remain valid configuration");
        assert_eq!(config.execution.commands[0].command, "cargo");
        assert_eq!(config.execution.servers[0].name(), "builder");
        assert!(patched.contains("  commands:\n    - command: cargo\n"));
        assert!(patched.contains("  servers:\n    - transport: host\n"));
        assert!(!patched.contains("command: old"));
        assert!(!patched.contains("name: old"));
        assert!(!patched.contains("commands: [{"));
        assert!(!patched.contains("servers: [{"));
    }

    #[test]
    fn visual_patch_omits_empty_optional_fields_from_the_entire_configuration() {
        let raw = "data-dir: null\nserver:\n  address: 127.0.0.1:8787\n  access:\n    api-key-env: null\nlogging:\n  retention-days: 30\nhistory:\n  retention-days: 30\n  max-records: 1000\nexecution:\n  default-action: confirm\n  commands: []\n  default-working-directory: null\n  default-server: host\n  servers: []\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                data_dir: Some(String::new()),
                api_key_env: Some(String::new()),
                default_working_directory: Some(String::new()),
                commands: Some(vec![CommandPolicyConfig {
                    command: "npm".to_string(),
                    action: PolicyAction::Allow,
                    targets: Vec::new(),
                    default_working_directory: None,
                    rules: Vec::new(),
                }]),
                servers: Some(Vec::new()),
                ..VisualConfigPatch::default()
            },
        )
        .expect("visual configuration should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("compact visual configuration should remain valid");
        assert!(config.data_dir.is_none());
        assert!(config.server.access.api_key_env.is_none());
        assert!(config.execution.default_working_directory.is_none());
        assert!(config.execution.servers.is_empty());
        assert_eq!(config.execution.commands[0].command, "npm");
        assert!(patched.contains("  commands:\n    - command: npm\n      action: allow\n"));
        assert!(!patched.contains("data-dir:"));
        assert!(!patched.contains("access:"));
        assert!(!patched.contains("api-key-env:"));
        assert!(!patched.contains("default-working-directory:"));
        assert!(!patched.contains("targets: []"));
        assert!(!patched.contains("rules: []"));
        assert!(!patched.contains("servers:"));
        assert!(!patched.contains("null"));
        assert!(!patched.contains("[]"));
    }

    #[test]
    fn missing_config_snapshot_uses_compact_yaml_defaults() {
        let path = std::env::temp_dir().join(format!(
            "host-bridge-config-store-default-{}.yaml",
            uuid::Uuid::new_v4()
        ));
        let store = ConfigStore::new(ResolvedConfigPath {
            path: path.display().to_string(),
            explicit: true,
        });

        let snapshot = store
            .snapshot(&AppConfig::default())
            .expect("default configuration should serialize");

        assert!(!snapshot.raw.contains("null"));
        assert!(!snapshot.raw.contains("[]"));
        assert!(!snapshot.raw.contains("access:"));
        assert!(!snapshot.raw.contains("commands:"));
        assert!(!snapshot.raw.contains("servers:"));
    }

    #[test]
    fn visual_patch_preserves_non_empty_ssh_optional_fields() {
        let raw = "server:\n  address: 127.0.0.1:8787\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                servers: Some(vec![ExecutionServerConfig::Ssh {
                    name: "builder".to_string(),
                    host: "build.example.com".to_string(),
                    port: 22,
                    user: "builder".to_string(),
                    target_platform: TargetPlatform::Linux,
                    auth: SshAuthConfig {
                        kind: SshAuthType::PasswordEnv,
                        r#ref: Some("BUILD_SSH_PASSWORD".to_string()),
                    },
                    known_hosts_file: Some("/home/builder/.ssh/known_hosts".to_string()),
                    verify_host_key: true,
                    connection_idle_timeout_ms: 300000,
                }]),
                ..VisualConfigPatch::default()
            },
        )
        .expect("SSH server configuration should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("serialized SSH server should remain valid configuration");
        let ExecutionServerConfig::Ssh {
            auth,
            known_hosts_file,
            ..
        } = &config.execution.servers[0]
        else {
            panic!("expected an SSH server");
        };
        assert_eq!(auth.r#ref.as_deref(), Some("BUILD_SSH_PASSWORD"));
        assert_eq!(
            known_hosts_file.as_deref(),
            Some("/home/builder/.ssh/known_hosts")
        );
        assert!(patched.contains("ref: BUILD_SSH_PASSWORD\n"));
        assert!(patched.contains("known-hosts-file: /home/builder/.ssh/known_hosts\n"));
    }

    #[test]
    fn visual_patch_omits_empty_ssh_optional_fields() {
        let raw = "server:\n  address: 127.0.0.1:8787\nexecution:\n  default-action: confirm\n  default-server: host\n  target-platform: auto\n  default-timeout-ms: 1800000\n  max-timeout-ms: 7200000\n";
        let patched = apply_visual_patch(
            raw,
            &VisualConfigPatch {
                servers: Some(vec![ExecutionServerConfig::Ssh {
                    name: "builder".to_string(),
                    host: "build.example.com".to_string(),
                    port: 22,
                    user: "builder".to_string(),
                    target_platform: TargetPlatform::Linux,
                    auth: SshAuthConfig {
                        kind: SshAuthType::Agent,
                        r#ref: None,
                    },
                    known_hosts_file: None,
                    verify_host_key: true,
                    connection_idle_timeout_ms: 300000,
                }]),
                ..VisualConfigPatch::default()
            },
        )
        .expect("SSH server configuration should serialize");

        let config = AppConfig::parse_raw("test.yaml", &patched)
            .expect("compact SSH server should remain valid configuration");
        assert_eq!(config.execution.servers.len(), 1);
        assert!(!patched.contains("ref:"));
        assert!(!patched.contains("known-hosts-file:"));
        assert!(!patched.contains("null"));
    }

    #[test]
    fn save_raw_writes_to_the_resolved_config_path() {
        let path = std::env::temp_dir().join(format!(
            "host-bridge-config-store-{}.yaml",
            uuid::Uuid::new_v4()
        ));
        let store = ConfigStore::new(ResolvedConfigPath {
            path: path.display().to_string(),
            explicit: true,
        });

        store
            .save_raw("server:\n  address: 127.0.0.1:8810\n".to_string())
            .expect("resolved config path should be writable");

        assert_eq!(
            fs::read_to_string(&path).expect("resolved config file should be readable"),
            "server:\n  address: 127.0.0.1:8810\n"
        );
        let _ = fs::remove_file(path);
    }
}
