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

use crate::domain::platform::runtime::RuntimePlatform;
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct SpawnPlan {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub windows_raw_tail: Option<OsString>,
}

impl SpawnPlan {
    fn windows_raw_tail(&self) -> Option<&OsString> {
        self.windows_raw_tail.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeEnvironment {
    platform: RuntimePlatform,
    system_environment: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SpawnPlanner {
    runtime_environment: RuntimeEnvironment,
}

impl RuntimeEnvironment {
    pub fn current() -> Self {
        Self::new(RuntimePlatform::current(), env::vars().collect())
    }

    pub fn new(platform: RuntimePlatform, system_environment: HashMap<String, String>) -> Self {
        Self {
            platform,
            system_environment,
        }
    }

    pub fn platform(&self) -> RuntimePlatform {
        self.platform
    }

    fn resolve_system_var(&self, name: &str) -> Option<OsString> {
        if self.platform.is_windows() {
            return self
                .system_environment
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| OsString::from(value));
        }

        self.system_environment.get(name).map(OsString::from)
    }
}

impl SpawnPlanner {
    pub fn current() -> Self {
        Self::new(RuntimeEnvironment::current())
    }

    pub fn new(runtime_environment: RuntimeEnvironment) -> Self {
        Self {
            runtime_environment,
        }
    }

    pub fn build(
        &self,
        executable: &str,
        args: &[String],
        environment: &HashMap<String, String>,
        working_directory: &Path,
    ) -> SpawnPlan {
        match self.runtime_environment.platform() {
            RuntimePlatform::Windows => build_windows_spawn_plan(
                executable,
                args,
                environment,
                working_directory,
                &self.runtime_environment,
            ),
            RuntimePlatform::Linux | RuntimePlatform::Macos => SpawnPlan {
                program: PathBuf::from(executable),
                args: args.iter().map(OsString::from).collect(),
                windows_raw_tail: None,
            },
        }
    }
}

pub fn apply_spawn_plan(_command: &mut Command, _spawn_plan: &SpawnPlan) {
    if let Some(raw_tail) = _spawn_plan.windows_raw_tail() {
        #[cfg(windows)]
        {
            _command.raw_arg(raw_tail);
        }

        #[cfg(not(windows))]
        {
            let _ = raw_tail;
        }
    }
}

fn build_windows_spawn_plan(
    executable: &str,
    args: &[String],
    environment: &HashMap<String, String>,
    working_directory: &Path,
    runtime_environment: &RuntimeEnvironment,
) -> SpawnPlan {
    let executable_path = Path::new(executable);
    let resolved_path = resolve_windows_executable_path(
        executable,
        environment,
        Some(working_directory),
        runtime_environment,
    );
    let shell_target = resolved_path
        .as_deref()
        .unwrap_or(executable_path)
        .to_string_lossy()
        .into_owned();

    match classify_windows_target_kind(resolved_path.as_deref().unwrap_or(executable_path)) {
        Some(WindowsTargetKind::DirectExecutable) => SpawnPlan {
            program: resolved_path.unwrap_or_else(|| PathBuf::from(executable)),
            args: args.iter().map(OsString::from).collect(),
            windows_raw_tail: None,
        },
        Some(WindowsTargetKind::PowerShellScript) => build_powershell_spawn_plan(
            resolved_path.unwrap_or_else(|| PathBuf::from(executable)),
            args,
            runtime_environment,
        ),
        Some(WindowsTargetKind::CmdShell) | Some(WindowsTargetKind::ShellAssociated) | None => {
            build_cmd_shell_spawn_plan(
                &shell_target,
                args,
                resolved_path
                    .as_deref()
                    .map(is_node_cmd_shim)
                    .unwrap_or(false),
                runtime_environment,
            )
        }
    }
}

fn build_powershell_spawn_plan(
    script_path: PathBuf,
    args: &[String],
    runtime_environment: &RuntimeEnvironment,
) -> SpawnPlan {
    let mut spawn_args = vec![
        OsString::from("-NoLogo"),
        OsString::from("-NoProfile"),
        OsString::from("-NonInteractive"),
        OsString::from("-File"),
        script_path.into_os_string(),
    ];
    spawn_args.extend(args.iter().map(OsString::from));

    SpawnPlan {
        program: resolve_windows_powershell_host(runtime_environment),
        args: spawn_args,
        windows_raw_tail: None,
    }
}

fn build_cmd_shell_spawn_plan(
    command: &str,
    args: &[String],
    double_escape_meta_chars: bool,
    runtime_environment: &RuntimeEnvironment,
) -> SpawnPlan {
    let escaped_command = escape_cmd_command(command);
    let escaped_arguments = args
        .iter()
        .map(|argument| escape_cmd_argument(argument, double_escape_meta_chars))
        .collect::<Vec<_>>();
    let shell_command = std::iter::once(escaped_command)
        .chain(escaped_arguments)
        .collect::<Vec<_>>()
        .join(" ");

    SpawnPlan {
        program: resolve_cmd_shell(runtime_environment),
        args: vec![
            OsString::from("/D"),
            OsString::from("/S"),
            OsString::from("/C"),
        ],
        windows_raw_tail: Some(OsString::from(format!("\"{shell_command}\""))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsTargetKind {
    DirectExecutable,
    CmdShell,
    PowerShellScript,
    ShellAssociated,
}

fn classify_windows_target_kind(path: &Path) -> Option<WindowsTargetKind> {
    let extension = path.extension()?.to_string_lossy();
    if extension.eq_ignore_ascii_case("com") || extension.eq_ignore_ascii_case("exe") {
        return Some(WindowsTargetKind::DirectExecutable);
    }
    if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
        return Some(WindowsTargetKind::CmdShell);
    }
    if extension.eq_ignore_ascii_case("ps1") {
        return Some(WindowsTargetKind::PowerShellScript);
    }

    Some(WindowsTargetKind::ShellAssociated)
}

fn resolve_cmd_shell(runtime_environment: &RuntimeEnvironment) -> PathBuf {
    runtime_environment
        .resolve_system_var("COMSPEC")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cmd.exe"))
}

fn resolve_windows_powershell_host(runtime_environment: &RuntimeEnvironment) -> PathBuf {
    ["powershell.exe", "powershell", "pwsh.exe", "pwsh"]
        .into_iter()
        .find_map(|candidate| {
            resolve_windows_executable_path(candidate, &HashMap::new(), None, runtime_environment)
        })
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

fn escape_cmd_command(command: &str) -> String {
    escape_cmd_meta(command, false)
}

fn escape_cmd_argument(argument: &str, double_escape_meta_chars: bool) -> String {
    let quoted_argument = quote_windows_argument(argument);
    escape_cmd_meta(&quoted_argument, double_escape_meta_chars)
}

fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut pending_backslashes = 0_usize;

    for character in argument.chars() {
        if character == '\\' {
            pending_backslashes += 1;
            continue;
        }

        if character == '"' {
            append_backslashes(&mut quoted, pending_backslashes * 2 + 1);
            quoted.push('"');
            pending_backslashes = 0;
            continue;
        }

        append_backslashes(&mut quoted, pending_backslashes);
        pending_backslashes = 0;
        quoted.push(character);
    }

    append_backslashes(&mut quoted, pending_backslashes * 2);
    quoted.push('"');
    quoted
}

fn append_backslashes(target: &mut String, count: usize) {
    for _ in 0..count {
        target.push('\\');
    }
}

fn escape_cmd_meta(value: &str, double_escape: bool) -> String {
    let mut escaped = String::with_capacity(value.len() * if double_escape { 3 } else { 2 });
    for character in value.chars() {
        if is_cmd_meta_character(character) {
            escaped.push('^');
            if double_escape {
                escaped.push('^');
            }
        }
        escaped.push(character);
    }
    escaped
}

fn is_cmd_meta_character(character: char) -> bool {
    matches!(
        character,
        '(' | ')'
            | '['
            | ']'
            | '%'
            | '!'
            | '^'
            | '"'
            | '`'
            | '<'
            | '>'
            | '&'
            | '|'
            | ';'
            | ','
            | ' '
            | '*'
            | '?'
    )
}

fn is_node_cmd_shim(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    if !extension.to_string_lossy().eq_ignore_ascii_case("cmd") {
        return false;
    }

    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();

    components
        .windows(2)
        .any(|window| window[0] == "node_modules" && window[1] == ".bin")
}

fn resolve_windows_executable_path(
    executable: &str,
    environment: &HashMap<String, String>,
    working_directory: Option<&Path>,
    runtime_environment: &RuntimeEnvironment,
) -> Option<PathBuf> {
    let executable_path = Path::new(executable);
    if executable_path.is_absolute() || executable.contains('/') || executable.contains('\\') {
        let candidate = resolve_windows_path_candidate(executable, working_directory);
        return resolve_path_candidate(
            &normalize_path(&candidate),
            &windows_path_extensions(environment, runtime_environment),
        );
    }

    if executable_path.extension().is_some() {
        return Some(PathBuf::from(executable));
    }

    let path_value = resolved_env_var(environment, runtime_environment, "PATH")?;
    let extensions = windows_path_extensions(environment, runtime_environment);

    for directory in env::split_paths(&path_value) {
        let candidate = directory.join(executable);
        if let Some(resolved) = resolve_path_candidate(&candidate, &extensions) {
            return Some(resolved);
        }
    }

    None
}

fn resolve_windows_path_candidate(executable: &str, working_directory: Option<&Path>) -> PathBuf {
    let executable_path = Path::new(executable);
    if executable_path.is_absolute() || looks_like_windows_absolute_path(executable) {
        return executable_path.to_path_buf();
    }

    let relative_path = windows_relative_path(executable);
    working_directory
        .unwrap_or_else(|| Path::new("."))
        .join(relative_path)
}

fn looks_like_windows_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn windows_relative_path(path: &str) -> PathBuf {
    let mut candidate = PathBuf::new();
    for segment in path.split(['\\', '/']) {
        if segment.is_empty() {
            continue;
        }
        candidate.push(segment);
    }
    candidate
}

fn resolve_path_candidate(path: &Path, extensions: &[String]) -> Option<PathBuf> {
    if path.extension().is_some() {
        return path.is_file().then(|| normalize_path(path));
    }

    for extension in extensions {
        for candidate in extension_candidates(path, extension) {
            if candidate.is_file() {
                return Some(normalize_path(&candidate));
            }
        }
    }

    path.is_file().then(|| normalize_path(path))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

fn extension_candidates(path: &Path, extension: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut suffixes = vec![extension.to_string()];
    let lowercase = extension.to_ascii_lowercase();
    if lowercase != extension {
        suffixes.push(lowercase);
    }
    let uppercase = extension.to_ascii_uppercase();
    if uppercase != extension && uppercase != suffixes[0] {
        suffixes.push(uppercase);
    }

    for suffix in suffixes {
        candidates.push(PathBuf::from(format!("{}{}", path.display(), suffix)));
    }

    candidates
}

fn windows_path_extensions(
    environment: &HashMap<String, String>,
    runtime_environment: &RuntimeEnvironment,
) -> Vec<String> {
    let raw_extensions = resolved_env_var(environment, runtime_environment, "PATHEXT")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD;.PS1"));

    raw_extensions
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            if extension.starts_with('.') {
                extension.to_string()
            } else {
                format!(".{extension}")
            }
        })
        .collect()
}

fn resolved_env_var(
    environment: &HashMap<String, String>,
    runtime_environment: &RuntimeEnvironment,
    name: &str,
) -> Option<OsString> {
    if runtime_environment.platform().is_windows() {
        if let Some((_, value)) = environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
        {
            return Some(OsString::from(value));
        }
    } else if let Some(value) = environment.get(name) {
        return Some(OsString::from(value));
    }

    runtime_environment.resolve_system_var(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn resolve_windows_executable_uses_pathext_for_bare_command() {
        let sandbox = temp_sandbox("bare");
        let npm_cmd = sandbox.join("npm.cmd");
        fs::write(&npm_cmd, "").expect("test command file should be created");

        let environment = HashMap::from([
            ("PATH".to_string(), sandbox.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);
        let runtime_environment = RuntimeEnvironment::new(RuntimePlatform::Windows, HashMap::new());

        let resolved =
            resolve_windows_executable_path("npm", &environment, None, &runtime_environment)
                .expect("resolver should find npm.cmd");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            npm_cmd.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn resolve_windows_executable_uses_pathext_for_explicit_path_without_extension() {
        let sandbox = temp_sandbox("path");
        let tool_prefix = sandbox.join("tool");
        let tool_cmd = sandbox.join("tool.cmd");
        fs::write(&tool_cmd, "").expect("test command file should be created");

        let environment = HashMap::from([("PATHEXT".to_string(), ".CMD".to_string())]);
        let runtime_environment = RuntimeEnvironment::new(RuntimePlatform::Windows, HashMap::new());
        let resolved = resolve_windows_executable_path(
            &tool_prefix.display().to_string(),
            &environment,
            None,
            &runtime_environment,
        )
        .expect("resolver should use PATHEXT for explicit path");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            tool_cmd.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn resolve_windows_executable_prefers_windows_shim_over_bare_file() {
        let sandbox = temp_sandbox("prefer-shim");
        let bare_tool = sandbox.join("npm");
        let tool_cmd = sandbox.join("npm.cmd");
        fs::write(&bare_tool, "#!/bin/sh\nexit 0\n").expect("bare file should be created");
        fs::write(&tool_cmd, "").expect("cmd shim should be created");

        let environment = HashMap::from([
            ("PATH".to_string(), sandbox.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);
        let runtime_environment = RuntimeEnvironment::new(RuntimePlatform::Windows, HashMap::new());

        let resolved =
            resolve_windows_executable_path("npm", &environment, None, &runtime_environment)
                .expect("resolver should prefer npm.cmd on Windows");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            tool_cmd.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn build_windows_spawn_plan_uses_cmd_for_batch_shims() {
        let sandbox = temp_sandbox("cmd-shim");
        let shim = sandbox.join("npm.cmd");
        fs::write(&shim, "").expect("test command file should be created");

        let environment = HashMap::from([
            ("PATH".to_string(), sandbox.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);
        let runtime_environment = RuntimeEnvironment::new(
            RuntimePlatform::Windows,
            HashMap::from([(
                "COMSPEC".to_string(),
                r"C:\Windows\System32\cmd.exe".to_string(),
            )]),
        );
        let planner = SpawnPlanner::new(runtime_environment);

        let plan = planner.build("npm", &["-v".to_string()], &environment, Path::new("."));
        assert_eq!(
            plan.program.to_string_lossy().to_ascii_lowercase(),
            r"c:\windows\system32\cmd.exe"
        );
        assert_eq!(
            plan.args,
            vec![
                OsString::from("/D"),
                OsString::from("/S"),
                OsString::from("/C")
            ]
        );
        let expected_tail = format!(
            "\"{} {}\"",
            escape_cmd_command(&shim.display().to_string()),
            escape_cmd_argument("-v", false)
        );
        assert_eq!(
            plan.windows_raw_tail
                .as_ref()
                .map(|value| value.to_string_lossy().to_ascii_lowercase()),
            Some(expected_tail.to_ascii_lowercase())
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn build_windows_spawn_plan_uses_direct_spawn_for_executables() {
        let sandbox = temp_sandbox("exe");
        let tool = sandbox.join("cargo.exe");
        fs::write(&tool, "").expect("test executable should be created");

        let environment = HashMap::from([
            ("PATH".to_string(), sandbox.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);
        let planner = SpawnPlanner::new(RuntimeEnvironment::new(
            RuntimePlatform::Windows,
            HashMap::new(),
        ));

        let plan = planner.build(
            "cargo",
            &["build".to_string()],
            &environment,
            Path::new("."),
        );
        assert_eq!(
            plan.program.to_string_lossy().to_ascii_lowercase(),
            tool.to_string_lossy().to_ascii_lowercase()
        );
        assert_eq!(plan.args, vec![OsString::from("build")]);
        assert!(plan.windows_raw_tail.is_none());

        let _ = fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn windows_path_extensions_falls_back_to_default_when_missing() {
        let runtime_environment = RuntimeEnvironment::new(RuntimePlatform::Windows, HashMap::new());
        let extensions = windows_path_extensions(&HashMap::new(), &runtime_environment);
        assert!(
            extensions
                .iter()
                .any(|extension| extension.eq_ignore_ascii_case(".cmd"))
        );
        assert!(
            extensions
                .iter()
                .any(|extension| extension.eq_ignore_ascii_case(".exe"))
        );
    }

    #[test]
    fn resolve_windows_executable_uses_working_directory_for_relative_paths() {
        let sandbox = temp_sandbox("relative");
        let project = sandbox.join("project");
        let shim_dir = project.join("node_modules").join(".bin");
        let shim = shim_dir.join("tool.cmd");
        fs::create_dir_all(&shim_dir).expect("shim directory should be created");
        fs::write(&shim, "").expect("relative cmd shim should be created");

        let environment = HashMap::from([("PATHEXT".to_string(), ".CMD".to_string())]);
        let runtime_environment = RuntimeEnvironment::new(RuntimePlatform::Windows, HashMap::new());
        let resolved = resolve_windows_executable_path(
            r".\node_modules\.bin\tool",
            &environment,
            Some(&project),
            &runtime_environment,
        )
        .expect("resolver should honor working directory for relative paths");
        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            shim.to_string_lossy().to_ascii_lowercase()
        );

        let _ = fs::remove_dir_all(&sandbox);
    }

    fn temp_sandbox(label: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "host-bridge-mcp-spawn-planner-{label}-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("temporary sandbox should be created");
        path
    }
}
