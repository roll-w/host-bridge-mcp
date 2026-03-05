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
use std::collections::HashMap;
use std::path::Path;

const CONFIG_ENV_KEY: &str = "HOST_BRIDGE_CONFIG";
const DEFAULT_CONFIG_FILE: &str = "host-bridge.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub execution: ExecutionConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind_address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExecutionConfig {
    pub default_policy: PolicyAction,
    pub command_policies: HashMap<String, CommandPolicy>,
    pub default_working_directory: Option<String>,
    pub path_mappings: Vec<PathMappingRule>,
    pub target_platform: TargetPlatform,
    pub enable_builtin_wsl_mapping: bool,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CommandPolicy {
    pub action: Option<PolicyAction>,
    pub default_working_directory: Option<String>,
    pub subcommand_policies: Vec<SubcommandPolicy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubcommandPolicy {
    pub when: String,
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

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            default_policy: PolicyAction::Confirm,
            command_policies: HashMap::new(),
            default_working_directory: None,
            path_mappings: Vec::new(),
            target_platform: TargetPlatform::Auto,
            enable_builtin_wsl_mapping: true,
            default_timeout_ms: 30 * 60 * 1000,
            max_timeout_ms: 2 * 60 * 60 * 1000,
        }
    }
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self {
            action: None,
            default_working_directory: None,
            subcommand_policies: Vec::new(),
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

        for (command, _) in &self.execution.command_policies {
            if command.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "execution.command_policies key cannot be empty".to_string(),
                ));
            }
        }

        for (command, policy) in &self.execution.command_policies {
            for subcommand_policy in &policy.subcommand_policies {
                if subcommand_policy.when.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "execution.command_policies.{command}.subcommand_policies.when cannot be empty"
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
}
