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

use crate::config::{AppConfig, CommandPolicy, PolicyAction, SubcommandPolicy};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireConfirmation,
    Deny,
}

#[derive(Debug, Clone)]
pub struct PolicyResult {
    pub decision: PolicyDecision,
    pub default_working_directory: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    default_action: PolicyAction,
    default_working_directory: Option<String>,
    command_policies: HashMap<String, CommandPolicy>,
}

impl PolicyEngine {
    pub fn new(config: AppConfig) -> Self {
        let command_policies = config
            .execution
            .command_policies
            .into_iter()
            .map(|(key, policy)| (normalize_command(&key), policy))
            .collect::<HashMap<_, _>>();

        Self {
            default_action: config.execution.default_policy,
            default_working_directory: config.execution.default_working_directory,
            command_policies,
        }
    }

    pub fn evaluate(&self, command_token: &str, command_arguments: &[String]) -> PolicyResult {
        let key = normalize_command(command_token);
        let command_policy = self.command_policies.get(&key);
        let subcommand_policy =
            command_policy.and_then(|policy| best_subcommand_policy(policy, command_arguments));

        let action = subcommand_policy
            .map(|policy| policy.action)
            .or_else(|| command_policy.and_then(|policy| policy.action))
            .unwrap_or(self.default_action);

        let decision = match action {
            PolicyAction::Allow => PolicyDecision::Allow,
            PolicyAction::Deny => PolicyDecision::Deny,
            PolicyAction::Confirm => PolicyDecision::RequireConfirmation,
        };

        let default_working_directory = subcommand_policy
            .and_then(extract_subcommand_working_directory)
            .or_else(|| command_policy.and_then(extract_working_directory))
            .or_else(|| self.default_working_directory.clone());

        PolicyResult {
            decision,
            default_working_directory,
        }
    }
}

fn best_subcommand_policy<'a>(
    command_policy: &'a CommandPolicy,
    command_arguments: &[String],
) -> Option<&'a SubcommandPolicy> {
    let normalized_arguments = command_arguments
        .iter()
        .map(|argument| normalize_subcommand_token(argument))
        .collect::<Vec<_>>();

    let mut best_match: Option<(&SubcommandPolicy, usize)> = None;
    for subcommand_policy in &command_policy.subcommand_policies {
        let pattern_tokens = tokenize_subcommand_pattern(&subcommand_policy.when);
        if pattern_tokens.is_empty() || !prefix_match(&pattern_tokens, &normalized_arguments) {
            continue;
        }

        let should_replace = match best_match {
            Some((_, current_len)) => pattern_tokens.len() > current_len,
            None => true,
        };
        if should_replace {
            best_match = Some((subcommand_policy, pattern_tokens.len()));
        }
    }

    best_match.map(|(policy, _)| policy)
}

fn tokenize_subcommand_pattern(pattern: &str) -> Vec<String> {
    pattern
        .split_whitespace()
        .map(normalize_subcommand_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
}

fn normalize_subcommand_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn prefix_match(pattern_tokens: &[String], input_tokens: &[String]) -> bool {
    if pattern_tokens.len() > input_tokens.len() {
        return false;
    }

    for (index, pattern_token) in pattern_tokens.iter().enumerate() {
        if input_tokens[index] != *pattern_token {
            return false;
        }
    }

    true
}

fn extract_working_directory(policy: &CommandPolicy) -> Option<String> {
    policy.default_working_directory.clone()
}

fn extract_subcommand_working_directory(policy: &SubcommandPolicy) -> Option<String> {
    policy.default_working_directory.clone()
}

pub fn normalize_command(command_token: &str) -> String {
    let trimmed = command_token.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let name = Path::new(trimmed)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(trimmed);

    strip_windows_suffix(name).to_ascii_lowercase()
}

fn strip_windows_suffix(value: &str) -> &str {
    for suffix in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stripped) = strip_ascii_case_suffix(value, suffix) {
            return stripped;
        }
    }
    value
}

fn strip_ascii_case_suffix<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    if value.len() < suffix.len() {
        return None;
    }

    let split_point = value.len() - suffix.len();
    let (head, tail) = value.split_at(split_point);
    if tail.eq_ignore_ascii_case(suffix) {
        Some(head)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutionConfig, PolicyAction, ServerConfig, SubcommandPolicy};
    use std::collections::HashMap;

    #[test]
    fn normalize_command_removes_windows_suffix() {
        assert_eq!(normalize_command("C:\\tools\\mvn.CMD"), "mvn");
    }

    #[test]
    fn confirm_policy_requires_confirmation() {
        let config = AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                default_policy: PolicyAction::Confirm,
                command_policies: HashMap::new(),
                ..ExecutionConfig::default()
            },
        };
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("mvn", &[]);
        assert_eq!(result.decision, PolicyDecision::RequireConfirmation);
    }

    #[test]
    fn subcommand_policy_overrides_command_policy() {
        let mut command_policies = HashMap::new();
        command_policies.insert(
            "mvn".to_string(),
            CommandPolicy {
                action: Some(PolicyAction::Allow),
                default_working_directory: None,
                subcommand_policies: vec![SubcommandPolicy {
                    when: "clean install".to_string(),
                    action: PolicyAction::Deny,
                    default_working_directory: None,
                }],
            },
        );

        let config = AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                default_policy: PolicyAction::Allow,
                command_policies,
                ..ExecutionConfig::default()
            },
        };
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("mvn", &["clean".to_string(), "install".to_string()]);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn longest_subcommand_pattern_wins() {
        let mut command_policies = HashMap::new();
        command_policies.insert(
            "npm".to_string(),
            CommandPolicy {
                action: Some(PolicyAction::Confirm),
                default_working_directory: None,
                subcommand_policies: vec![
                    SubcommandPolicy {
                        when: "run".to_string(),
                        action: PolicyAction::Allow,
                        default_working_directory: None,
                    },
                    SubcommandPolicy {
                        when: "run build".to_string(),
                        action: PolicyAction::Deny,
                        default_working_directory: None,
                    },
                ],
            },
        );

        let config = AppConfig {
            server: ServerConfig::default(),
            execution: ExecutionConfig {
                default_policy: PolicyAction::Confirm,
                command_policies,
                ..ExecutionConfig::default()
            },
        };
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("npm", &["run".to_string(), "build".to_string()]);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }
}
