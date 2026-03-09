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
use std::path::Path;

const CONFIG_ENV_KEY: &str = "HOST_BRIDGE_CONFIG";
const DEFAULT_CONFIG_FILE: &str = "host-bridge.yaml";

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
    pub path_mappings: Vec<PathMappingRule>,
    pub target_platform: TargetPlatform,
    pub enable_builtin_wsl_mapping: bool,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
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
        source: serde_yaml::Error,
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
            path_mappings: Vec::new(),
            target_platform: TargetPlatform::Auto,
            enable_builtin_wsl_mapping: true,
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
        let value = serde_yaml::from_str::<serde_yaml::Value>(&raw).map_err(|source| {
            ConfigError::Parse {
                path: path.clone(),
                source,
            }
        })?;
        reject_legacy_execution_keys(&value)?;
        let config = serde_yaml::from_str::<Self>(&raw)
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

        for rule in &self.execution.path_mappings {
            if rule.from.trim().is_empty() || rule.to.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "execution.path-mappings entries require non-empty from/to".to_string(),
                ));
            }
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

fn reject_legacy_execution_keys(value: &serde_yaml::Value) -> Result<(), ConfigError> {
    let Some(execution) = value
        .as_mapping()
        .and_then(|root| root.get(serde_yaml::Value::String("execution".to_string())))
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Ok(());
    };

    if execution.contains_key(&serde_yaml::Value::String("default_policy".to_string())) {
        return Err(ConfigError::Validation(
            "execution.default-policy is no longer supported; use execution.default-action"
                .to_string(),
        ));
    }

    if execution.contains_key(&serde_yaml::Value::String("rules".to_string())) {
        return Err(ConfigError::Validation(
            "execution.rules is no longer supported; migrate to execution.commands".to_string(),
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_config_is_valid() {
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn reject_command_policy_with_empty_command() {
        let config = AppConfig {
            execution: ExecutionConfig {
                commands: vec![CommandPolicyConfig {
                    command: "   ".to_string(),
                    action: PolicyAction::Allow,
                    default_working_directory: None,
                    rules: Vec::new(),
                }],
                ..ExecutionConfig::default()
            },
            ..AppConfig::default()
        };

        let error = config.validate().expect_err("config should be invalid");
        assert!(error.to_string().contains("execution.commands[0].command"));
    }

    #[test]
    fn reject_nested_command_rule_without_args_prefix() {
        let config = AppConfig {
            execution: ExecutionConfig {
                commands: vec![CommandPolicyConfig {
                    command: "cargo".to_string(),
                    action: PolicyAction::Allow,
                    default_working_directory: None,
                    rules: vec![CommandRuleConfig {
                        args_prefix: Vec::new(),
                        action: PolicyAction::Confirm,
                        default_working_directory: None,
                    }],
                }],
                ..ExecutionConfig::default()
            },
            ..AppConfig::default()
        };

        let error = config.validate().expect_err("config should be invalid");
        assert!(error.to_string().contains(
            "execution.commands[0].rules[0].args-prefix must contain at least one token"
        ));
    }

    #[test]
    fn reject_empty_nested_command_rule_token() {
        let config = AppConfig {
            execution: ExecutionConfig {
                commands: vec![CommandPolicyConfig {
                    command: "cargo".to_string(),
                    action: PolicyAction::Allow,
                    default_working_directory: None,
                    rules: vec![CommandRuleConfig {
                        args_prefix: vec!["build".to_string(), "   ".to_string()],
                        action: PolicyAction::Confirm,
                        default_working_directory: None,
                    }],
                }],
                ..ExecutionConfig::default()
            },
            ..AppConfig::default()
        };

        let error = config.validate().expect_err("config should be invalid");
        assert!(
            error
                .to_string()
                .contains("execution.commands[0].rules[0].args-prefix[1]")
        );
    }

    #[test]
    fn reject_legacy_default_policy_key_when_loading() {
        let path = write_temp_config(
            r#"execution:
  default_policy: allow
"#,
        );

        let error = AppConfig::load_with_path(Some(path.to_str().expect("valid temp path")))
            .expect_err("legacy default_policy should be rejected");
        assert!(
            error
                .to_string()
                .contains("execution.default-policy is no longer supported")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reject_legacy_rules_key_when_loading() {
        let path = write_temp_config(
            r#"execution:
  default-action: confirm
  rules:
    - command: cargo
      action: deny
      args-prefix:
        - publish
"#,
        );

        let error = AppConfig::load_with_path(Some(path.to_str().expect("valid temp path")))
            .expect_err("legacy execution.rules should be rejected");
        assert!(
            error
                .to_string()
                .contains("execution.rules is no longer supported")
        );

        let _ = fs::remove_file(path);
    }

    fn write_temp_config(contents: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("host-bridge-config-{unique}.yaml"));
        fs::write(&path, contents).expect("temp config should be written");
        path
    }

    #[test]
    fn default_config_uses_single_server() {
        let config = AppConfig::default();

        assert_eq!(config.server.bind_address, "127.0.0.1:8787");
        assert_eq!(config.server.access, AccessConfig::default());
    }

    #[test]
    fn explicit_missing_config_path_fails_to_load() {
        let error = AppConfig::load_with_path(Some("definitely-missing-config.yaml"))
            .expect_err("missing explicit config should fail");

        assert!(matches!(error, ConfigError::Read { .. }));
    }

    #[test]
    fn unknown_yaml_fields_are_rejected() {
        let path = write_temp_config(
            r#"server:
  bind-process: 127.0.0.1:8787
    unexpected: true
"#,
        );

        let error = AppConfig::load_with_path(Some(path.to_str().expect("valid temp path")))
            .expect_err("unknown fields should be rejected");

        assert!(matches!(error, ConfigError::Parse { .. }));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_access_header_fields_are_rejected() {
        let path = write_temp_config(
            r#"server:
  bind-process: 127.0.0.1:8787
  access:
    api-key-env: HOST_BRIDGE_API_KEY
    header-name: Authorization
"#,
        );

        let error = AppConfig::load_with_path(Some(path.to_str().expect("valid temp path")))
            .expect_err("legacy auth header config should be rejected");

        assert!(matches!(error, ConfigError::Parse { .. }));
        let _ = fs::remove_file(path);
    }
}
