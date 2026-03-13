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

use crate::config::{PathMappingRule, Platform, TargetPlatform};
use crate::domain::platform::runtime::{RuntimePlatform, current_is_wsl, resolve_target_platform};

#[derive(Debug, Clone)]
pub struct PathMapper {
    rules: Vec<PathMappingRule>,
    target_platform: RuntimePlatform,
    running_inside_wsl: bool,
}

impl PathMapper {
    pub fn new(rules: Vec<PathMappingRule>, target: TargetPlatform) -> Self {
        Self {
            rules,
            target_platform: resolve_target_platform(target),
            running_inside_wsl: current_is_wsl(),
        }
    }

    pub fn map_path(&self, raw_path: &str) -> String {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        for rule in &self.rules {
            if !self.rule_applies(rule) {
                continue;
            }
            if let Some(mapped) = apply_rule(trimmed, rule) {
                return mapped;
            }
        }

        trimmed.to_string()
    }

    pub fn map_command_if_path(&self, command_token: &str) -> String {
        if command_token.contains('/') || command_token.contains('\\') {
            return self.map_path(command_token);
        }
        command_token.to_string()
    }

    pub fn map_argument_if_path(&self, argument: &str) -> String {
        if looks_like_path(argument) {
            return self.map_path(argument);
        }
        argument.to_string()
    }

    fn rule_applies(&self, rule: &PathMappingRule) -> bool {
        if rule.platforms.is_empty() {
            return true;
        }

        rule.platforms.iter().any(|platform| match platform {
            Platform::Windows => self.target_platform == RuntimePlatform::Windows,
            Platform::Linux => self.target_platform == RuntimePlatform::Linux,
            Platform::Macos => self.target_platform == RuntimePlatform::Macos,
            Platform::Wsl => self.running_inside_wsl,
        })
    }
}

pub fn looks_like_path(value: &str) -> bool {
    if value.starts_with('/') || value.starts_with("\\\\") {
        return true;
    }

    let bytes = value.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'\\' && bytes[0].is_ascii_alphabetic() {
        return true;
    }

    false
}

fn apply_rule(input: &str, rule: &PathMappingRule) -> Option<String> {
    if !input.starts_with(&rule.from) {
        return None;
    }

    let remainder = input.strip_prefix(&rule.from)?;
    let destination_is_windows = rule.to.contains('\\');
    let mut normalized_remainder = if destination_is_windows {
        remainder.replace('/', "\\")
    } else {
        remainder.replace('\\', "/")
    };

    if destination_is_windows {
        if rule.to.ends_with('\\') && normalized_remainder.starts_with('\\') {
            normalized_remainder = normalized_remainder
                .strip_prefix('\\')
                .unwrap_or(&normalized_remainder)
                .to_string();
        }
    } else if rule.to.ends_with('/') && normalized_remainder.starts_with('/') {
        normalized_remainder = normalized_remainder
            .strip_prefix('/')
            .unwrap_or(&normalized_remainder)
            .to_string();
    }

    if normalized_remainder.is_empty() {
        return Some(rule.to.clone());
    }

    let needs_separator = !rule.to.ends_with('/')
        && !rule.to.ends_with('\\')
        && !normalized_remainder.starts_with('/')
        && !normalized_remainder.starts_with('\\');

    if needs_separator {
        let separator = if destination_is_windows { '\\' } else { '/' };
        return Some(format!("{}{separator}{normalized_remainder}", rule.to));
    }

    Some(format!("{}{}", rule.to, normalized_remainder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_mapping_rule_applies() {
        let mapper = PathMapper::new(
            vec![PathMappingRule {
                from: "/workspace/mnt/d".to_string(),
                to: "D:\\".to_string(),
                platforms: Vec::new(),
            }],
            TargetPlatform::Windows,
        );

        assert_eq!(
            mapper.map_path("/workspace/mnt/d/Code/repo"),
            "D:\\Code\\repo"
        );
    }

    #[test]
    fn detects_windows_drive_path() {
        assert!(looks_like_path("C:\\Users\\dev"));
        assert!(!looks_like_path("mvn"));
    }
}
