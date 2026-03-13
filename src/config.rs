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

use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

const CONFIG_ENV_KEY: &str = "HOST_BRIDGE_CONFIG";
const DEFAULT_CONFIG_FILE: &str = "host-bridge.yaml";
const DEFAULT_EXECUTION_SERVER: &str = "host";
const DEFAULT_SSH_PORT: u16 = 22;
const DEFAULT_SSH_CONNECTION_IDLE_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct AppConfig {
    pub server: ServerConfig,
    pub execution: ExecutionConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ServerConfig {
    #[serde(rename = "bind-process")]
    pub bind_address: String,
    pub access: AccessConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct AccessConfig {
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct LoggingConfig {
    pub memory_buffer_lines: usize,
    pub file_path: Option<String>,
    pub persist_file: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ExecutionConfig {
    pub default_action: PolicyAction,
    #[serde(default)]
    pub commands: Vec<CommandPolicyConfig>,
    pub default_working_directory: Option<String>,
    pub default_server: String,
    #[serde(default)]
    pub servers: Vec<ExecutionServerConfig>,
    pub path_mappings: Vec<PathMappingRule>,
    pub target_platform: TargetPlatform,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "transport",
    rename_all_fields = "kebab-case",
    deny_unknown_fields
)]
pub enum ExecutionServerConfig {
    #[serde(rename = "host")]
    Host {
        name: String,
        #[serde(default)]
        target_platform: TargetPlatform,
        #[serde(default)]
        path_mappings: Vec<PathMappingRule>,
    },
    #[serde(rename = "ssh")]
    Ssh {
        name: String,
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        user: String,
        target_platform: TargetPlatform,
        #[serde(default)]
        path_mappings: Vec<PathMappingRule>,
        #[serde(default)]
        auth: SshAuthConfig,
        #[serde(default)]
        known_hosts_file: Option<String>,
        #[serde(default = "default_ssh_connection_idle_timeout_ms")]
        connection_idle_timeout_ms: u64,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SshAuthConfig {
    #[serde(rename = "type")]
    pub kind: SshAuthType,
    pub r#ref: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SshAuthType {
    #[default]
    Agent,
    IdentityFile,
    PasswordEnv,
    PasswordFile,
}

impl SshAuthType {
    fn as_config_value(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::IdentityFile => "identity-file",
            Self::PasswordEnv => "password-env",
            Self::PasswordFile => "password-file",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CommandPolicyConfig {
    pub command: String,
    pub action: PolicyAction,
    pub default_working_directory: Option<String>,
    #[serde(default)]
    pub rules: Vec<CommandRuleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CommandRuleConfig {
    #[serde(default)]
    pub args_prefix: Vec<String>,
    pub action: PolicyAction,
    pub default_working_directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct PathMappingRule {
    pub from: String,
    pub to: String,
    pub platforms: Vec<Platform>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    Allow,
    #[default]
    Confirm,
    Deny,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TargetPlatform {
    #[default]
    Auto,
    Windows,
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    Wsl,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_saphyr::Error,
    },
    #[error("invalid config: {0}")]
    Validation(String),
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            execution: ExecutionConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8787".to_string(),
            access: AccessConfig::default(),
        }
    }
}

impl Default for AccessConfig {
    fn default() -> Self {
        Self { api_key_env: None }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            memory_buffer_lines: 2_000,
            file_path: None,
            persist_file: false,
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            default_action: PolicyAction::Confirm,
            commands: Vec::new(),
            default_working_directory: None,
            default_server: DEFAULT_EXECUTION_SERVER.to_string(),
            servers: Vec::new(),
            path_mappings: Vec::new(),
            target_platform: TargetPlatform::Auto,
            default_timeout_ms: 30 * 60 * 1000,
            max_timeout_ms: 2 * 60 * 60 * 1000,
        }
    }
}

impl Default for PathMappingRule {
    fn default() -> Self {
        Self {
            from: String::new(),
            to: String::new(),
            platforms: Vec::new(),
        }
    }
}

impl ExecutionServerConfig {
    pub fn name(&self) -> &str {
        match self {
            Self::Host { name, .. } | Self::Ssh { name, .. } => name,
        }
    }
}

fn default_ssh_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn default_ssh_connection_idle_timeout_ms() -> u64 {
    DEFAULT_SSH_CONNECTION_IDLE_TIMEOUT_MS
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_with_path(None)
    }

    pub fn load_with_path(config_path: Option<&str>) -> Result<Self, ConfigError> {
        let explicit_path = config_path
            .map(|value| value.to_string())
            .or_else(|| std::env::var(CONFIG_ENV_KEY).ok());
        let path = explicit_path
            .clone()
            .unwrap_or_else(|| DEFAULT_CONFIG_FILE.to_string());
        if !Path::new(&path).exists() {
            return if explicit_path.is_some() {
                Err(ConfigError::Read {
                    path,
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "configuration file was not found",
                    ),
                })
            } else {
                Ok(Self::default())
            };
        }

        let raw = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let config = serde_saphyr::from_str::<Self>(&raw)
            .map_err(|source| ConfigError::Parse { path, source })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.server.bind_address.trim().is_empty() {
            return Err(ConfigError::Validation(
                "server.bind-process cannot be empty".to_string(),
            ));
        }

        validate_server_access(&self.server.access, "server.access")?;

        validate_server_name(&self.execution.default_server, "execution.default-server")?;

        if self.execution.default_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "execution.default-timeout-ms must be greater than zero".to_string(),
            ));
        }

        if self.execution.max_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "execution.max-timeout-ms must be greater than zero".to_string(),
            ));
        }

        if self.execution.max_timeout_ms < self.execution.default_timeout_ms {
            return Err(ConfigError::Validation(
                "execution.max-timeout-ms must be greater than or equal to execution.default-timeout-ms"
                    .to_string(),
            ));
        }

        for (index, command) in self.execution.commands.iter().enumerate() {
            validate_command_name(
                &command.command,
                &format!("execution.commands[{index}].command"),
            )?;
            validate_working_directory(
                command.default_working_directory.as_deref(),
                &format!("execution.commands[{index}].default-working-directory"),
            )?;

            for (rule_index, rule) in command.rules.iter().enumerate() {
                if rule.args_prefix.is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "execution.commands[{index}].rules[{rule_index}].args-prefix must contain at least one token"
                    )));
                }

                validate_args_prefix(
                    &rule.args_prefix,
                    &format!("execution.commands[{index}].rules[{rule_index}].args-prefix"),
                )?;
                validate_working_directory(
                    rule.default_working_directory.as_deref(),
                    &format!(
                        "execution.commands[{index}].rules[{rule_index}].default-working-directory"
                    ),
                )?;
            }
        }

        validate_path_mappings(&self.execution.path_mappings, "execution.path-mappings")?;

        let mut server_names = HashSet::new();
        for (index, server) in self.execution.servers.iter().enumerate() {
            let name = server.name();
            validate_server_name(name, &format!("execution.servers[{index}].name"))?;
            if !server_names.insert(name.to_string()) {
                return Err(ConfigError::Validation(format!(
                    "execution.servers[{index}].name duplicates server '{name}'"
                )));
            }

            match server {
                ExecutionServerConfig::Host { path_mappings, .. } => {
                    validate_path_mappings(
                        path_mappings,
                        &format!("execution.servers[{index}].path-mappings"),
                    )?;
                }
                ExecutionServerConfig::Ssh {
                    name,
                    host,
                    port,
                    user,
                    target_platform,
                    path_mappings,
                    auth,
                    known_hosts_file,
                    connection_idle_timeout_ms,
                    ..
                } => {
                    if name == DEFAULT_EXECUTION_SERVER {
                        return Err(ConfigError::Validation(
                            "execution.servers[*].name 'host' is reserved for the local host transport"
                                .to_string(),
                        ));
                    }
                    if host.trim().is_empty() {
                        return Err(ConfigError::Validation(format!(
                            "execution.servers[{index}].host cannot be empty"
                        )));
                    }
                    if *port == 0 {
                        return Err(ConfigError::Validation(format!(
                            "execution.servers[{index}].port must be greater than zero"
                        )));
                    }
                    if *connection_idle_timeout_ms == 0 {
                        return Err(ConfigError::Validation(format!(
                            "execution.servers[{index}].connection-idle-timeout-ms must be greater than zero"
                        )));
                    }
                    if *target_platform == TargetPlatform::Auto {
                        return Err(ConfigError::Validation(format!(
                            "execution.servers[{index}].target-platform must be explicit for SSH servers"
                        )));
                    }
                    validate_non_empty(user, &format!("execution.servers[{index}].user"))?;
                    validate_ssh_auth(auth, &format!("execution.servers[{index}].auth"))?;
                    validate_optional_non_empty(
                        known_hosts_file.as_deref(),
                        &format!("execution.servers[{index}].known-hosts-file"),
                    )?;
                    validate_path_mappings(
                        path_mappings,
                        &format!("execution.servers[{index}].path-mappings"),
                    )?;
                }
            }
        }

        if self.execution.default_server != DEFAULT_EXECUTION_SERVER
            && !server_names.contains(&self.execution.default_server)
        {
            return Err(ConfigError::Validation(format!(
                "execution.default-server '{}' must reference a configured server",
                self.execution.default_server
            )));
        }

        if self.logging.memory_buffer_lines == 0 {
            return Err(ConfigError::Validation(
                "logging.memory-buffer-lines must be greater than zero".to_string(),
            ));
        }

        validate_working_directory(self.logging.file_path.as_deref(), "logging.file-path")?;

        Ok(())
    }
}

fn validate_command_name(command: &str, location: &str) -> Result<(), ConfigError> {
    if command.trim().is_empty() {
        return Err(ConfigError::Validation(format!(
            "{location} cannot be empty"
        )));
    }

    Ok(())
}

fn validate_server_name(name: &str, location: &str) -> Result<(), ConfigError> {
    validate_non_empty(name, location)
}

fn validate_non_empty(value: &str, location: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::Validation(format!(
            "{location} cannot be empty"
        )));
    }

    Ok(())
}

fn validate_server_access(access: &AccessConfig, location: &str) -> Result<(), ConfigError> {
    if let Some(api_key_env) = access.api_key_env.as_deref() {
        if api_key_env.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "{location}.api-key-env cannot be empty when provided"
            )));
        }
    }

    Ok(())
}

fn validate_args_prefix(args_prefix: &[String], location: &str) -> Result<(), ConfigError> {
    for (index, token) in args_prefix.iter().enumerate() {
        if token.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "{location}[{index}] cannot be empty"
            )));
        }
    }

    Ok(())
}

fn validate_working_directory(path: Option<&str>, location: &str) -> Result<(), ConfigError> {
    if let Some(path) = path {
        if path.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "{location} cannot be empty when provided"
            )));
        }
    }

    Ok(())
}

fn validate_optional_non_empty(value: Option<&str>, location: &str) -> Result<(), ConfigError> {
    if let Some(value) = value {
        if value.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "{location} cannot be empty when provided"
            )));
        }
    }

    Ok(())
}

fn validate_ssh_auth(auth: &SshAuthConfig, location: &str) -> Result<(), ConfigError> {
    validate_optional_non_empty(auth.r#ref.as_deref(), &format!("{location}.ref"))?;

    match auth.kind {
        SshAuthType::Agent => {
            if auth.r#ref.is_some() {
                Err(ConfigError::Validation(format!(
                    "{location}.ref must be omitted when auth.type is agent"
                )))
            } else {
                Ok(())
            }
        }
        SshAuthType::IdentityFile | SshAuthType::PasswordEnv | SshAuthType::PasswordFile => {
            if auth.r#ref.is_none() {
                Err(ConfigError::Validation(format!(
                    "{location}.ref is required when auth.type is {}",
                    auth.kind.as_config_value()
                )))
            } else {
                Ok(())
            }
        }
    }
}

fn validate_path_mappings(
    path_mappings: &[PathMappingRule],
    location: &str,
) -> Result<(), ConfigError> {
    for (index, rule) in path_mappings.iter().enumerate() {
        if rule.from.trim().is_empty() || rule.to.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "{location}[{index}] entries require non-empty from/to"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
