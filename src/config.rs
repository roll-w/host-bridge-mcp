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
const DEFAULT_CONFIG_FILE: &str = "host-bridge.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub execution: ExecutionConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind_address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub memory_buffer_lines: usize,
    pub file_path: Option<String>,
    pub persist_file: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExecutionConfig {
    #[serde(alias = "default_policy")]
    pub default_action: PolicyAction,
    pub rules: Vec<ExecutionRule>,
    pub default_working_directory: Option<String>,
    pub path_mappings: Vec<PathMappingRule>,
    pub target_platform: TargetPlatform,
    pub enable_builtin_wsl_mapping: bool,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionRule {
    pub command: String,
    #[serde(default)]
    pub args_prefix: Vec<String>,
    pub action: PolicyAction,
    pub default_working_directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
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
        source: toml::de::Error,
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
            bind_address: "0.0.0.0:8787".to_string()
        }
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
            rules: Vec::new(),
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
        let path = config_path.map(|value| value.to_string()).unwrap_or_else(|| {
            std::env::var(CONFIG_ENV_KEY).unwrap_or_else(|_| DEFAULT_CONFIG_FILE.to_string())
        });
        if !Path::new(&path).exists() {
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let config =
            toml::from_str::<Self>(&raw).map_err(|source| ConfigError::Parse { path, source })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.execution.default_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "execution.default_timeout_ms must be greater than zero".to_string(),
            ));
        }

        if self.execution.max_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "execution.max_timeout_ms must be greater than zero".to_string(),
            ));
        }

        if self.execution.max_timeout_ms < self.execution.default_timeout_ms {
            return Err(ConfigError::Validation(
                "execution.max_timeout_ms must be greater than or equal to execution.default_timeout_ms"
                    .to_string(),
            ));
        }

        for (index, rule) in self.execution.rules.iter().enumerate() {
            if rule.command.trim().is_empty() {
                return Err(ConfigError::Validation(
                    format!("execution.rules[{index}].command cannot be empty"),
                ));
            }

            for (arg_index, token) in rule.args_prefix.iter().enumerate() {
                if token.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "execution.rules[{index}].args_prefix[{arg_index}] cannot be empty"
                    )));
                }
            }
        }

        for rule in &self.execution.path_mappings {
            if rule.from.trim().is_empty() || rule.to.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "execution.path_mappings entries require non-empty from/to".to_string(),
                ));
            }
        }

        if self.logging.memory_buffer_lines == 0 {
            return Err(ConfigError::Validation(
                "logging.memory_buffer_lines must be greater than zero".to_string(),
            ));
        }

        if let Some(path) = &self.logging.file_path {
            if path.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "logging.file_path cannot be empty when provided".to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn reject_rule_with_empty_command() {
        let config = AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                rules: vec![ExecutionRule {
                    command: "   ".to_string(),
                    args_prefix: Vec::new(),
                    action: PolicyAction::Allow,
                    default_working_directory: None,
                }],
                ..ExecutionConfig::default()
            },
            logging: LoggingConfig::default(),
        };

        let error = config.validate().expect_err("config should be invalid");
        assert!(error.to_string().contains("execution.rules[0].command"));
    }
}
