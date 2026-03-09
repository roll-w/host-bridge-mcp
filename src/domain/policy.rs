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

use crate::config::{AppConfig, CommandPolicyConfig, CommandRuleConfig, PolicyAction};

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
    rules: Vec<CompiledExecutionRule>,
}

#[derive(Debug, Clone)]
struct CompiledExecutionRule {
    command: String,
    args_prefix: Vec<String>,
    action: PolicyAction,
    default_working_directory: Option<String>,
    order: usize,
}

impl PolicyEngine {
    pub fn new(config: AppConfig) -> Self {
        let execution = config.execution;
        let rules = compile_command_policies(&execution.commands);

        Self {
            default_action: execution.default_action,
            default_working_directory: execution.default_working_directory,
            rules,
        }
    }

    pub fn evaluate(&self, command_token: &str, command_arguments: &[String]) -> PolicyResult {
        let key = normalize_command(command_token);
        let normalized_arguments = command_arguments
            .iter()
            .map(|argument| normalize_subcommand_token(argument))
            .collect::<Vec<_>>();
        let matching_rule = self
            .rules
            .iter()
            .filter(|rule| {
                rule.command == key && prefix_match(&rule.args_prefix, &normalized_arguments)
            })
            .max_by_key(|rule| (rule.args_prefix.len(), rule.order));

        let action = matching_rule
            .map(|rule| rule.action)
            .unwrap_or(self.default_action);

        let decision = match action {
            PolicyAction::Allow => PolicyDecision::Allow,
            PolicyAction::Deny => PolicyDecision::Deny,
            PolicyAction::Confirm => PolicyDecision::RequireConfirmation,
        };

        let default_working_directory = matching_rule
            .and_then(extract_working_directory)
            .or_else(|| self.default_working_directory.clone());

        PolicyResult {
            decision,
            default_working_directory,
        }
    }
}

fn compile_command_policies(commands: &[CommandPolicyConfig]) -> Vec<CompiledExecutionRule> {
    let mut compiled_rules = Vec::new();
    let mut order = 0;

    for command_policy in commands {
        compiled_rules.push(CompiledExecutionRule {
            command: normalize_command(&command_policy.command),
            args_prefix: Vec::new(),
            action: command_policy.action,
            default_working_directory: command_policy.default_working_directory.clone(),
            order,
        });
        order += 1;

        for rule in &command_policy.rules {
            compiled_rules.push(compile_nested_rule(order, command_policy, rule));
            order += 1;
        }
    }

    compiled_rules
}

fn compile_nested_rule(
    order: usize,
    command_policy: &CommandPolicyConfig,
    rule: &CommandRuleConfig,
) -> CompiledExecutionRule {
    CompiledExecutionRule {
        command: normalize_command(&command_policy.command),
        args_prefix: rule
            .args_prefix
            .iter()
            .map(|token| normalize_subcommand_token(token))
            .collect(),
        action: rule.action,
        default_working_directory: rule
            .default_working_directory
            .clone()
            .or_else(|| command_policy.default_working_directory.clone()),
        order,
    }
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

fn extract_working_directory(policy: &CompiledExecutionRule) -> Option<String> {
    policy.default_working_directory.clone()
}

pub fn normalize_command(command_token: &str) -> String {
    let trimmed = command_token.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);

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
    use crate::config::{CommandPolicyConfig, CommandRuleConfig, ExecutionConfig, PolicyAction};

    fn test_config(execution: ExecutionConfig) -> AppConfig {
        let mut config = AppConfig::default();
        config.execution = execution;
        config
    }

    #[test]
    fn normalize_command_removes_windows_suffix() {
        assert_eq!(normalize_command("C:\\tools\\mvn.CMD"), "mvn");
    }

    #[test]
    fn confirm_policy_requires_confirmation() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            ..ExecutionConfig::default()
        });
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("mvn", &[]);
        assert_eq!(result.decision, PolicyDecision::RequireConfirmation);
    }

    #[test]
    fn subcommand_policy_overrides_command_policy() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Allow,
            commands: vec![CommandPolicyConfig {
                command: "mvn".to_string(),
                action: PolicyAction::Allow,
                default_working_directory: None,
                rules: vec![CommandRuleConfig {
                    args_prefix: vec!["clean".to_string(), "install".to_string()],
                    action: PolicyAction::Deny,
                    default_working_directory: None,
                }],
            }],
            ..ExecutionConfig::default()
        });
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("mvn", &["clean".to_string(), "install".to_string()]);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn longest_subcommand_pattern_wins() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            commands: vec![CommandPolicyConfig {
                command: "npm".to_string(),
                action: PolicyAction::Confirm,
                default_working_directory: None,
                rules: vec![
                    CommandRuleConfig {
                        args_prefix: vec!["run".to_string()],
                        action: PolicyAction::Allow,
                        default_working_directory: None,
                    },
                    CommandRuleConfig {
                        args_prefix: vec!["run".to_string(), "build".to_string()],
                        action: PolicyAction::Deny,
                        default_working_directory: None,
                    },
                ],
            }],
            ..ExecutionConfig::default()
        });
        let engine = PolicyEngine::new(config);

        let result = engine.evaluate("npm", &["run".to_string(), "build".to_string()]);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn later_rule_wins_when_specificity_matches() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            commands: vec![CommandPolicyConfig {
                command: "cargo".to_string(),
                action: PolicyAction::Confirm,
                default_working_directory: None,
                rules: vec![
                    CommandRuleConfig {
                        args_prefix: vec!["build".to_string()],
                        action: PolicyAction::Allow,
                        default_working_directory: None,
                    },
                    CommandRuleConfig {
                        args_prefix: vec!["build".to_string()],
                        action: PolicyAction::Deny,
                        default_working_directory: None,
                    },
                ],
            }],
            ..ExecutionConfig::default()
        });

        let engine = PolicyEngine::new(config);
        let result = engine.evaluate("cargo", &["build".to_string()]);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn grouped_command_policy_uses_command_action_and_nested_override() {
        let config = test_config(ExecutionConfig {
            default_action: PolicyAction::Confirm,
            commands: vec![CommandPolicyConfig {
                command: "npm".to_string(),
                action: PolicyAction::Allow,
                default_working_directory: Some("/workspace/frontend".to_string()),
                rules: vec![CommandRuleConfig {
                    args_prefix: vec!["publish".to_string()],
                    action: PolicyAction::Deny,
                    default_working_directory: None,
                }],
            }],
            ..ExecutionConfig::default()
        });

        let engine = PolicyEngine::new(config);

        let install_result = engine.evaluate("npm", &["install".to_string()]);
        assert_eq!(install_result.decision, PolicyDecision::Allow);
        assert_eq!(
            install_result.default_working_directory,
            Some("/workspace/frontend".to_string())
        );

        let publish_result = engine.evaluate("npm", &["publish".to_string()]);
        assert_eq!(publish_result.decision, PolicyDecision::Deny);
        assert_eq!(
            publish_result.default_working_directory,
            Some("/workspace/frontend".to_string())
        );
    }
}
