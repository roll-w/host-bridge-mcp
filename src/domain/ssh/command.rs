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

use super::SshCommandRequest;
use crate::domain::platform::runtime::RuntimePlatform;
use std::collections::HashMap;
use std::time::Duration;

const MIN_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2);
const MAX_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn build_remote_command(
    platform: RuntimePlatform,
    request: &SshCommandRequest,
) -> String {
    match platform {
        RuntimePlatform::Windows => build_windows_remote_command(request),
        RuntimePlatform::Linux | RuntimePlatform::Macos => build_posix_remote_command(request),
    }
}

pub(super) fn keepalive_interval_for(idle_timeout: Duration) -> Option<Duration> {
    if idle_timeout < MIN_KEEPALIVE_INTERVAL.saturating_mul(2) {
        return None;
    }

    let candidate = idle_timeout / 3;
    Some(candidate.clamp(MIN_KEEPALIVE_INTERVAL, MAX_KEEPALIVE_INTERVAL))
}

fn build_posix_remote_command(request: &SshCommandRequest) -> String {
    let env_prefix = build_posix_env_prefix(&request.env);
    let command = std::iter::once(quote_posix(&request.executable))
        .chain(request.args.iter().map(|value| quote_posix(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let exec_command = if env_prefix.is_empty() {
        format!("exec {command}")
    } else {
        format!("exec env {env_prefix} {command}")
    };
    let script = match request.working_directory.as_deref() {
        Some(working_directory) => {
            format!("cd -- {} && {exec_command}", quote_posix(working_directory))
        }
        None => exec_command,
    };

    format!("sh -lc {}", quote_posix(&script))
}

fn build_windows_remote_command(request: &SshCommandRequest) -> String {
    let script = build_windows_script(request);
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
                .map(|value| quote_posix(&format!("{key}={value}")))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_windows_script(request: &SshCommandRequest) -> String {
    let mut lines = vec!["$ErrorActionPreference = 'Stop'".to_string()];
    if let Some(working_directory) = request.working_directory.as_deref() {
        lines.push(format!(
            "Set-Location -LiteralPath {}",
            quote_powershell(working_directory)
        ));
    }

    let mut keys = request.env.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if let Some(value) = request.env.get(key) {
            lines.push(format!(
                "[System.Environment]::SetEnvironmentVariable({}, {}, 'Process')",
                quote_powershell(key),
                quote_powershell(value)
            ));
        }
    }

    let command = std::iter::once(quote_powershell(&request.executable))
        .chain(request.args.iter().map(|value| quote_powershell(value)))
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

    fn request() -> SshCommandRequest {
        SshCommandRequest {
            executable: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            env: HashMap::from([("RUST_LOG".to_string(), "info debug".to_string())]),
            working_directory: Some("/srv/app".to_string()),
            timeout_ms: 3_000,
        }
    }

    #[test]
    fn posix_command_uses_shell_safe_wrapping() {
        let remote_command = build_remote_command(RuntimePlatform::Linux, &request());

        assert!(remote_command.contains("sh -lc"));
        assert!(remote_command.contains("/srv/app"));
        assert!(remote_command.contains("RUST_LOG=info debug"));
        assert!(remote_command.contains("cargo"));
        assert!(remote_command.contains("--release"));
    }

    #[test]
    fn windows_command_uses_encoded_powershell() {
        let mut request = request();
        request.executable = "cargo.exe".to_string();
        request.working_directory = Some("C:\\repo".to_string());
        let remote_command = build_remote_command(RuntimePlatform::Windows, &request);

        assert!(remote_command.starts_with(
            "powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "
        ));
    }

    #[test]
    fn base64_encoder_matches_known_value() {
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
    }
}
