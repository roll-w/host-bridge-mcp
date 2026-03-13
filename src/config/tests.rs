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
    assert!(
        error
            .to_string()
            .contains("execution.commands[0].rules[0].args-prefix must contain at least one token")
    );
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
    assert_eq!(config.execution.default_server, "host");
    assert!(config.execution.servers.is_empty());
}

#[test]
fn reject_default_server_when_missing_from_configured_servers() {
    let config = AppConfig {
        execution: ExecutionConfig {
            default_server: "prod".to_string(),
            servers: vec![ExecutionServerConfig::Ssh {
                name: "staging".to_string(),
                host: "staging.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig::default(),
                known_hosts_file: None,
                connection_idle_timeout_ms: 30_000,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(
        error
            .to_string()
            .contains("execution.default-server 'prod'")
    );
}

#[test]
fn reject_duplicate_server_names() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![
                ExecutionServerConfig::Host {
                    name: "build".to_string(),
                    target_platform: TargetPlatform::Auto,
                    path_mappings: Vec::new(),
                },
                ExecutionServerConfig::Host {
                    name: "build".to_string(),
                    target_platform: TargetPlatform::Auto,
                    path_mappings: Vec::new(),
                },
            ],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(error.to_string().contains("duplicates server 'build'"));
}

#[test]
fn reject_ssh_server_with_auto_target_platform() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Auto,
                path_mappings: Vec::new(),
                auth: SshAuthConfig::default(),
                known_hosts_file: None,
                connection_idle_timeout_ms: 30_000,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(
        error
            .to_string()
            .contains("target-platform must be explicit for SSH servers")
    );
}

#[test]
fn reject_ssh_server_named_host() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![ExecutionServerConfig::Ssh {
                name: "host".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig::default(),
                known_hosts_file: None,
                connection_idle_timeout_ms: 30_000,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(error.to_string().contains("'host' is reserved"));
}

#[test]
fn reject_missing_auth_ref_for_password_file() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig {
                    kind: SshAuthType::PasswordFile,
                    r#ref: None,
                },
                known_hosts_file: None,
                connection_idle_timeout_ms: 30_000,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(error.to_string().contains("auth.ref is required"));
}

#[test]
fn reject_auth_ref_for_agent_auth() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig {
                    kind: SshAuthType::Agent,
                    r#ref: Some("unexpected".to_string()),
                },
                known_hosts_file: None,
                connection_idle_timeout_ms: 30_000,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(
        error
            .to_string()
            .contains("must be omitted when auth.type is agent")
    );
}

#[test]
fn reject_zero_ssh_connection_idle_timeout() {
    let config = AppConfig {
        execution: ExecutionConfig {
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig::default(),
                known_hosts_file: None,
                connection_idle_timeout_ms: 0,
            }],
            ..ExecutionConfig::default()
        },
        ..AppConfig::default()
    };

    let error = config.validate().expect_err("config should be invalid");
    assert!(
        error
            .to_string()
            .contains("connection-idle-timeout-ms must be greater than zero")
    );
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
