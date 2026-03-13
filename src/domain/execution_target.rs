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

use crate::config::{ExecutionConfig, ExecutionServerConfig, SshAuthConfig, SshAuthType};
use crate::domain::path_mapping::PathMapper;
use crate::domain::platform::runtime::{RuntimePlatform, resolve_target_platform};
use std::collections::HashMap;
use std::time::Duration;

const HOST_TARGET_NAME: &str = "host";

#[derive(Debug, Clone)]
pub struct ExecutionTargetRegistry {
    default_target: String,
    targets: HashMap<String, ExecutionTarget>,
}

#[derive(Debug, Clone)]
pub struct ExecutionTarget {
    pub name: String,
    pub transport: ExecutionTransport,
    pub target_platform: RuntimePlatform,
    pub path_mapper: PathMapper,
}

#[derive(Debug, Clone)]
pub enum ExecutionTransport {
    Host,
    Ssh(SshTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuthTarget,
    pub known_hosts_file: Option<String>,
    pub connection_idle_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SshAuthTarget {
    Agent,
    IdentityFile(String),
    PasswordEnv(String),
    PasswordFile(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEnvironmentSummary {
    pub name: String,
    pub platform: String,
}

impl ExecutionTargetRegistry {
    pub fn from_config(execution: &ExecutionConfig) -> Self {
        let mut targets = HashMap::from([(
            HOST_TARGET_NAME.to_string(),
            implicit_host_target(execution),
        )]);

        for server in &execution.servers {
            targets.insert(server.name().to_string(), build_target(server));
        }

        Self {
            default_target: execution.default_server.clone(),
            targets,
        }
    }

    pub fn resolve(&self, requested: Option<&str>) -> Option<&ExecutionTarget> {
        let name = requested.unwrap_or(&self.default_target);
        self.targets.get(name)
    }

    pub fn default_target(&self) -> &ExecutionTarget {
        self.targets
            .get(&self.default_target)
            .expect("validated config must resolve default execution target")
    }

    pub fn environments(&self) -> Vec<ExecutionEnvironmentSummary> {
        let mut targets = self.targets.values().collect::<Vec<_>>();
        targets.sort_by(|left, right| left.name.cmp(&right.name));
        targets
            .into_iter()
            .map(|target| ExecutionEnvironmentSummary {
                name: target.name.clone(),
                platform: target.target_platform.as_name().to_string(),
            })
            .collect()
    }
}

fn implicit_host_target(execution: &ExecutionConfig) -> ExecutionTarget {
    ExecutionTarget {
        name: HOST_TARGET_NAME.to_string(),
        transport: ExecutionTransport::Host,
        target_platform: resolve_target_platform(execution.target_platform),
        path_mapper: PathMapper::new(execution.path_mappings.clone(), execution.target_platform),
    }
}

fn build_target(server: &ExecutionServerConfig) -> ExecutionTarget {
    match server {
        ExecutionServerConfig::Host {
            name,
            target_platform,
            path_mappings,
        } => ExecutionTarget {
            name: name.clone(),
            transport: ExecutionTransport::Host,
            target_platform: resolve_target_platform(*target_platform),
            path_mapper: PathMapper::new(path_mappings.clone(), *target_platform),
        },
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
        } => ExecutionTarget {
            name: name.clone(),
            transport: ExecutionTransport::Ssh(SshTarget {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                auth: build_ssh_auth_target(auth),
                known_hosts_file: known_hosts_file.clone(),
                connection_idle_timeout: Duration::from_millis(*connection_idle_timeout_ms),
            }),
            target_platform: resolve_target_platform(*target_platform),
            path_mapper: PathMapper::new(path_mappings.clone(), *target_platform),
        },
    }
}

fn build_ssh_auth_target(auth: &SshAuthConfig) -> SshAuthTarget {
    match auth.kind {
        SshAuthType::Agent => SshAuthTarget::Agent,
        SshAuthType::IdentityFile => {
            SshAuthTarget::IdentityFile(auth.r#ref.clone().expect("validated auth ref"))
        }
        SshAuthType::PasswordEnv => {
            SshAuthTarget::PasswordEnv(auth.r#ref.clone().expect("validated auth ref"))
        }
        SshAuthType::PasswordFile => {
            SshAuthTarget::PasswordFile(auth.r#ref.clone().expect("validated auth ref"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutionServerConfig, PathMappingRule, TargetPlatform};

    #[test]
    fn registry_includes_implicit_host_target() {
        let registry = ExecutionTargetRegistry::from_config(&ExecutionConfig::default());

        assert_eq!(registry.default_target().name, "host");
        assert_eq!(
            registry.environments(),
            vec![ExecutionEnvironmentSummary {
                name: "host".to_string(),
                platform: RuntimePlatform::current().as_name().to_string(),
            }]
        );
    }

    #[test]
    fn explicit_host_target_overrides_implicit_host_settings() {
        let registry = ExecutionTargetRegistry::from_config(&ExecutionConfig {
            servers: vec![ExecutionServerConfig::Host {
                name: "host".to_string(),
                target_platform: TargetPlatform::Windows,
                path_mappings: vec![PathMappingRule {
                    from: "/workspace/mnt/d".to_string(),
                    to: "D:\\".to_string(),
                    platforms: Vec::new(),
                }],
            }],
            ..ExecutionConfig::default()
        });

        let target = registry
            .resolve(Some("host"))
            .expect("host target should exist");
        assert_eq!(target.target_platform, RuntimePlatform::Windows);
        assert_eq!(
            target.path_mapper.map_path("/workspace/mnt/d/repo"),
            "D:\\repo"
        );
    }

    #[test]
    fn registry_resolves_named_ssh_target() {
        let registry = ExecutionTargetRegistry::from_config(&ExecutionConfig {
            default_server: "prod".to_string(),
            servers: vec![ExecutionServerConfig::Ssh {
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "deploy".to_string(),
                target_platform: TargetPlatform::Linux,
                path_mappings: Vec::new(),
                auth: SshAuthConfig {
                    kind: SshAuthType::PasswordEnv,
                    r#ref: Some("SSH_PASSWORD".to_string()),
                },
                known_hosts_file: Some("/home/dev/.ssh/known_hosts".to_string()),
                connection_idle_timeout_ms: 45_000,
            }],
            ..ExecutionConfig::default()
        });

        let target = registry.default_target();
        match &target.transport {
            ExecutionTransport::Ssh(ssh) => {
                assert_eq!(ssh.host, "prod.example.com");
                assert_eq!(ssh.user, "deploy");
                match &ssh.auth {
                    SshAuthTarget::PasswordEnv(reference) => assert_eq!(reference, "SSH_PASSWORD"),
                    _ => panic!("expected password-env auth"),
                }
                assert_eq!(ssh.connection_idle_timeout, Duration::from_millis(45_000));
            }
            ExecutionTransport::Host => panic!("expected ssh target"),
        }
    }
}
