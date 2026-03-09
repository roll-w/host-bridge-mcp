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

use crate::domain::execution_target::SshTarget;
use crate::domain::platform::runtime::RuntimePlatform;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SshInvocation {
    pub program: String,
    pub args: Vec<String>,
}

pub fn build_ssh_invocation(
    target: &SshTarget,
    platform: RuntimePlatform,
    executable: &str,
    args: &[String],
    env: &HashMap<String, String>,
    working_directory: Option<&str>,
) -> SshInvocation {
    let mut ssh_args = vec![
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
    ];

    if target.port != 22 {
        ssh_args.push("-p".to_string());
        ssh_args.push(target.port.to_string());
    }

    if let Some(identity_file) = target.identity_file.as_deref() {
        ssh_args.push("-i".to_string());
        ssh_args.push(identity_file.to_string());
        ssh_args.push("-o".to_string());
        ssh_args.push("IdentitiesOnly=yes".to_string());
    }

    if let Some(known_hosts_file) = target.known_hosts_file.as_deref() {
        ssh_args.push("-o".to_string());
        ssh_args.push(format!("UserKnownHostsFile={known_hosts_file}"));
    }

    ssh_args.push(destination(target));
    ssh_args.push(match platform {
        RuntimePlatform::Windows => {
            build_windows_remote_command(executable, args, env, working_directory)
        }
        RuntimePlatform::Linux | RuntimePlatform::Macos => {
            build_posix_remote_command(executable, args, env, working_directory)
        }
    });

    SshInvocation {
        program: "ssh".to_string(),
        args: ssh_args,
    }
}

fn destination(target: &SshTarget) -> String {
    match target.user.as_deref() {
        Some(user) => format!("{user}@{}", target.host),
        None => target.host.clone(),
    }
}

fn build_posix_remote_command(
    executable: &str,
    args: &[String],
    env: &HashMap<String, String>,
    working_directory: Option<&str>,
) -> String {
    let env_prefix = build_posix_env_prefix(env);
    let command = std::iter::once(quote_posix(executable))
        .chain(args.iter().map(|value| quote_posix(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let exec_command = if env_prefix.is_empty() {
        format!("exec {command}")
    } else {
        format!("exec env {env_prefix} {command}")
    };
    let script = match working_directory {
        Some(working_directory) => {
            format!("cd -- {} && {exec_command}", quote_posix(working_directory))
        }
        None => exec_command,
    };

    format!("sh -lc {}", quote_posix(&script))
}

fn build_windows_remote_command(
    executable: &str,
    args: &[String],
    env: &HashMap<String, String>,
    working_directory: Option<&str>,
) -> String {
    let script = build_windows_script(executable, args, env, working_directory);
    let encoded = encode_powershell_command(&script);
    format!(
        "powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}"
    )
}

fn build_posix_env_prefix(env: &HashMap<String, String>) -> String {
    let mut keys = env.keys().collect::<Vec<_>>();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| {
            env.get(key)
                .map(|value| format!("{}", quote_posix(&format!("{key}={value}"))))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_windows_script(
    executable: &str,
    args: &[String],
    env: &HashMap<String, String>,
    working_directory: Option<&str>,
) -> String {
    let mut lines = vec!["$ErrorActionPreference = 'Stop'".to_string()];
    if let Some(working_directory) = working_directory {
        lines.push(format!(
            "Set-Location -LiteralPath {}",
            quote_powershell(working_directory)
        ));
    }

    let mut keys = env.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if let Some(value) = env.get(key) {
            lines.push(format!(
                "[System.Environment]::SetEnvironmentVariable({}, {}, 'Process')",
                quote_powershell(key),
                quote_powershell(value)
            ));
        }
    }

    let command = std::iter::once(quote_powershell(executable))
        .chain(args.iter().map(|value| quote_powershell(value)))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(format!("& {command}"));
    lines.push("exit $LASTEXITCODE".to_string());
    lines.join("\n")
}

fn quote_posix(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

fn quote_powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encode_powershell_command(script: &str) -> String {
    let utf16 = script.encode_utf16().collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(utf16.len() * 2);
    for unit in utf16 {
        let [low, high] = unit.to_le_bytes();
        bytes.push(low);
        bytes.push(high);
    }

    encode_base64(&bytes)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        let combined = (u32::from(first) << 16) | (u32::from(second) << 8) | u32::from(third);

        encoded.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[((combined >> 6) & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(combined & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_target() -> SshTarget {
        SshTarget {
            host: "server.example.com".to_string(),
            port: 22,
            user: Some("deploy".to_string()),
            identity_file: Some("/home/dev/.ssh/id_ed25519".to_string()),
            known_hosts_file: Some("/home/dev/.ssh/known_hosts".to_string()),
        }
    }

    #[test]
    fn posix_invocation_uses_batch_mode_and_shell_safe_command() {
        let invocation = build_ssh_invocation(
            &ssh_target(),
            RuntimePlatform::Linux,
            "cargo",
            &["build".to_string(), "--release".to_string()],
            &HashMap::from([("RUST_LOG".to_string(), "info debug".to_string())]),
            Some("/srv/app"),
        );

        assert_eq!(invocation.program, "ssh");
        assert!(invocation.args.contains(&"BatchMode=yes".to_string()));
        assert!(
            invocation
                .args
                .contains(&"deploy@server.example.com".to_string())
        );
        let remote_command = invocation.args.last().expect("remote command should exist");
        assert!(remote_command.contains("sh -lc"));
        assert!(remote_command.contains("/srv/app"));
        assert!(remote_command.contains("RUST_LOG=info debug"));
        assert!(remote_command.contains("cargo"));
        assert!(remote_command.contains("--release"));
    }

    #[test]
    fn windows_invocation_uses_encoded_powershell() {
        let invocation = build_ssh_invocation(
            &ssh_target(),
            RuntimePlatform::Windows,
            "cargo.exe",
            &["build".to_string()],
            &HashMap::new(),
            Some("C:\\repo"),
        );

        let remote_command = invocation.args.last().expect("remote command should exist");
        assert!(remote_command.starts_with(
            "powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "
        ));
    }

    #[test]
    fn base64_encoder_matches_known_value() {
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
    }
}
