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
use crate::config::{
    CommandPolicyConfig, CommandRuleConfig, ExecutionConfig, ExecutionServerConfig,
    PathMappingRule, PolicyAction, SshAuthConfig, SshAuthType, TargetPlatform,
};
use crate::domain::execution_target::SshAuthTarget;

fn test_config(execution: ExecutionConfig) -> Arc<AppConfig> {
    let mut config = AppConfig::default();
    config.execution = execution;
    Arc::new(config)
}

#[tokio::test]
async fn reject_zero_timeout() {
    let config = test_config(ExecutionConfig {
        default_action: PolicyAction::Allow,
        ..ExecutionConfig::default()
    });
    let service = ExecutionService::new(config);
    let result = service
        .prepare_command(ExecuteCommandInput {
            command: "cargo build".to_string(),
            server: None,
            working_directory: None,
            env: HashMap::new(),
            timeout_ms: Some(0),
        })
        .await;
    assert!(matches!(result, Err(ExecutionError::InvalidTimeout)));
}

#[tokio::test]
async fn prepare_command_marks_confirmation_when_policy_requires_it() {
    let config = test_config(ExecutionConfig {
        default_action: PolicyAction::Confirm,
        commands: vec![CommandPolicyConfig {
            command: "cargo".to_string(),
            default_working_directory: None,
            action: PolicyAction::Confirm,
            rules: vec![CommandRuleConfig {
                args_prefix: vec!["build".to_string()],
                action: PolicyAction::Confirm,
                default_working_directory: None,
            }],
        }],
        ..ExecutionConfig::default()
    });

    let service = ExecutionService::new(config);
    let prepared = service
        .prepare_command(ExecuteCommandInput {
            command: "cargo build".to_string(),
            server: None,
            working_directory: None,
            env: HashMap::new(),
            timeout_ms: None,
        })
        .await
        .expect("command should prepare");

    let confirmation = prepared
        .confirmation_request()
        .expect("confirmation should exist");
    assert_eq!(confirmation.server, "host");
    assert!(matches!(
        confirmation.platform.as_str(),
        "windows" | "linux" | "macos"
    ));
}

#[tokio::test]
async fn prepare_command_rejects_unknown_server() {
    let service = ExecutionService::new(test_config(ExecutionConfig {
        default_action: PolicyAction::Allow,
        ..ExecutionConfig::default()
    }));

    let error = service
        .prepare_command(ExecuteCommandInput {
            command: "cargo build".to_string(),
            server: Some("missing".to_string()),
            working_directory: None,
            env: HashMap::new(),
            timeout_ms: None,
        })
        .await
        .expect_err("unknown server should fail");

    assert!(matches!(error, ExecutionError::UnknownServer(name) if name == "missing"));
}

#[tokio::test]
async fn prepare_command_builds_ssh_invocation_for_remote_server() {
    let service = ExecutionService::new(test_config(ExecutionConfig {
        default_action: PolicyAction::Confirm,
        servers: vec![ExecutionServerConfig::Ssh {
            name: "prod".to_string(),
            host: "prod.example.com".to_string(),
            port: 2222,
            user: "deploy".to_string(),
            target_platform: TargetPlatform::Linux,
            path_mappings: vec![PathMappingRule {
                from: "/workspace".to_string(),
                to: "/srv/workspace".to_string(),
                platforms: Vec::new(),
            }],
            auth: SshAuthConfig {
                kind: SshAuthType::PasswordEnv,
                r#ref: Some("SSH_PASSWORD".to_string()),
            },
            known_hosts_file: Some("/home/dev/.ssh/known_hosts".to_string()),
            connection_idle_timeout_ms: 90_000,
        }],
        ..ExecutionConfig::default()
    }));

    let prepared = service
        .prepare_command(ExecuteCommandInput {
            command: "cargo build".to_string(),
            server: Some("prod".to_string()),
            working_directory: Some("/workspace/app".to_string()),
            env: HashMap::from([("RUST_LOG".to_string(), "debug".to_string())]),
            timeout_ms: Some(5_000),
        })
        .await
        .expect("remote command should prepare");

    let confirmation = prepared
        .confirmation_request()
        .expect("confirmation should exist");
    assert_eq!(confirmation.server, "prod");
    assert_eq!(confirmation.platform, "linux");
    assert_eq!(
        confirmation.working_directory.as_deref(),
        Some("/srv/workspace/app")
    );
    match &prepared.run.backend {
        RunExecutionBackend::Ssh(remote_run) => {
            assert_eq!(remote_run.target.host, "prod.example.com");
            assert_eq!(remote_run.target.port, 2222);
            assert_eq!(remote_run.target.user, "deploy");
            assert_eq!(
                remote_run.target.connection_idle_timeout,
                Duration::from_millis(90_000)
            );
            match &remote_run.target.auth {
                SshAuthTarget::PasswordEnv(reference) => assert_eq!(reference, "SSH_PASSWORD"),
                _ => panic!("expected password-env auth"),
            }
            assert_eq!(
                remote_run.request.working_directory.as_deref(),
                Some("/srv/workspace/app")
            );
            assert_eq!(remote_run.request.executable, "cargo");
            assert_eq!(remote_run.request.args, vec!["build".to_string()]);
            assert_eq!(
                remote_run.request.env.get("RUST_LOG").map(String::as_str),
                Some("debug")
            );
        }
        RunExecutionBackend::Host(_) => panic!("expected ssh backend"),
    }
}

#[test]
fn execution_record_preserves_merged_output_order() {
    let path = std::env::temp_dir().join(format!("host-bridge-mcp-test-{}.log", Uuid::new_v4()));
    let record = ExecutionRecord::with_output_path(path.clone()).expect("record should initialize");

    record
        .append_output("first\n")
        .expect("first write should succeed");
    record
        .append_output("second\n")
        .expect("second write should succeed");

    assert_eq!(
        record.read_output().expect("output should be readable"),
        "first\nsecond\n"
    );

    drop(record);
    let _ = fs::remove_file(path);
}

#[test]
fn execution_output_file_is_named_from_execution_id() {
    let execution_id =
        Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").expect("uuid should parse");
    let path = execution_output_path(execution_id).expect("path should resolve");

    assert_eq!(
        path.file_name().and_then(|value| value.to_str()),
        Some("123e4567-e89b-12d3-a456-426614174000.log")
    );
}

#[test]
fn execution_output_file_is_retained_after_record_drop() {
    let path = std::env::temp_dir().join(format!("host-bridge-mcp-output-{}.log", Uuid::new_v4()));
    {
        let record =
            ExecutionRecord::with_output_path(path.clone()).expect("record should initialize");
        record
            .append_output("persisted\n")
            .expect("output should be written");
    }

    assert!(path.exists());
    let _ = fs::remove_file(path);
}
