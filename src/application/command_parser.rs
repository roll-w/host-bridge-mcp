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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandParseError {
    #[error("command cannot be empty")]
    Empty,
    #[error("unclosed quote in command")]
    UnclosedQuote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub contains_shell_operator: bool,
}

pub fn parse_command_line(
    input: &str,
    platform: crate::domain::platform::runtime::RuntimePlatform,
) -> Result<ParsedCommand, CommandParseError> {
    let windows = platform.is_windows();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut single_quote = false;
    let mut double_quote = false;
    let mut contains_shell_operator = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' && !single_quote {
            started = true;
            if windows {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    current.extend(std::iter::repeat_n('\\', count / 2));
                    chars.next();
                    if count % 2 == 0 {
                        double_quote = !double_quote;
                    } else {
                        current.push('"');
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', count));
                }
            } else if let Some(&next) = chars.peek() {
                if !double_quote || matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    chars.next();
                    if next != '\n' {
                        current.push(next);
                    }
                } else {
                    current.push('\\');
                }
            } else {
                current.push('\\');
            }
            continue;
        }
        if windows && ch == '^' && !double_quote {
            started = true;
            current.push(chars.next().unwrap_or('^'));
            continue;
        }
        if !windows && ch == '\'' && !double_quote {
            single_quote = !single_quote;
            started = true;
            continue;
        }
        if ch == '"' && !single_quote {
            double_quote = !double_quote;
            started = true;
            continue;
        }
        if !single_quote && !double_quote {
            if matches!(ch, ';' | '|' | '<' | '>' | '&' | '\n' | '\r') {
                contains_shell_operator = true;
            }
            if ch.is_whitespace() {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
                continue;
            }
        }
        started = true;
        current.push(ch);
    }

    if single_quote || double_quote {
        return Err(CommandParseError::UnclosedQuote);
    }
    if started {
        tokens.push(current);
    }
    let mut tokens = tokens.into_iter();
    let program = tokens
        .next()
        .filter(|value| !value.is_empty())
        .ok_or(CommandParseError::Empty)?;
    Ok(ParsedCommand {
        program,
        args: tokens.collect(),
        contains_shell_operator,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::platform::runtime::RuntimePlatform;

    fn parse_command_line(input: &str) -> Result<ParsedCommand, CommandParseError> {
        super::parse_command_line(input, RuntimePlatform::Linux)
    }

    #[test]
    fn windows_paths_and_empty_arguments_are_preserved() {
        let parsed = super::parse_command_line(
            r#"tool.exe "C:\Users\Public\file.txt" "" next"#,
            RuntimePlatform::Windows,
        )
        .unwrap();
        assert_eq!(parsed.args, [r"C:\Users\Public\file.txt", "", "next"]);
    }

    #[test]
    fn posix_empty_arguments_and_quoted_backslashes_are_preserved() {
        let parsed = parse_command_line(r#"tool '' "" "a\b" a\ b"#).unwrap();
        assert_eq!(parsed.args, ["", "", r"a\b", "a b"]);
    }

    #[test]
    fn windows_escaped_quotes_and_trailing_backslashes_are_preserved() {
        let parsed = super::parse_command_line(
            r#"tool "C:\Program Files\\" "a\"b""#,
            RuntimePlatform::Windows,
        )
        .unwrap();
        assert_eq!(parsed.args, ["C:\\Program Files\\", "a\"b"]);
    }

    #[test]
    fn parses_simple_command() {
        let parsed = parse_command_line("mvn clean compile").expect("should parse command");
        assert_eq!(parsed.program, "mvn");
        assert_eq!(parsed.args, vec!["clean", "compile"]);
    }

    #[test]
    fn parses_quoted_arguments() {
        let parsed =
            parse_command_line("npm run test -- --grep \"my case\"").expect("should parse command");
        assert_eq!(parsed.program, "npm");
        assert_eq!(parsed.args, vec!["run", "test", "--", "--grep", "my case"]);
    }

    #[test]
    fn fails_on_unclosed_quote() {
        let error = parse_command_line("mvn \"clean").expect_err("should fail");
        assert_eq!(error, CommandParseError::UnclosedQuote);
    }

    #[test]
    fn detects_shell_chaining() {
        let parsed =
            parse_command_line("cargo build && cargo test").expect("should parse chained command");
        assert!(parsed.contains_shell_operator);
        assert_eq!(parsed.program, "cargo");
    }

    #[test]
    fn detects_pipe_operator() {
        let parsed = parse_command_line("ls -la | grep foo").expect("should parse piped command");
        assert!(parsed.contains_shell_operator);
        assert_eq!(parsed.program, "ls");
    }

    #[test]
    fn detects_semicolon_operator() {
        let parsed = parse_command_line("cd /tmp; ls").expect("should parse semicolon command");
        assert!(parsed.contains_shell_operator);
        assert_eq!(parsed.program, "cd");
    }

    #[test]
    fn detects_redirection_operator() {
        let output =
            parse_command_line("echo hello > output.txt").expect("should parse output redirection");
        assert!(output.contains_shell_operator);

        let input = parse_command_line("sort < input.txt").expect("should parse input redirection");
        assert!(input.contains_shell_operator);
    }

    #[test]
    fn simple_command_has_no_shell_operator() {
        let parsed = parse_command_line("cargo build --release").expect("should parse");
        assert!(!parsed.contains_shell_operator);
    }

    #[test]
    fn allows_separator_inside_quotes() {
        let parsed = parse_command_line("python -c \"print('a && b')\"")
            .expect("quoted operator should be allowed");
        assert_eq!(parsed.program, "python");
        assert_eq!(parsed.args, vec!["-c", "print('a && b')"]);
    }

    #[test]
    fn allows_redirection_character_inside_quotes() {
        let parsed = parse_command_line("echo \"a > b\"")
            .expect("quoted redirection character should be allowed");
        assert!(!parsed.contains_shell_operator);
    }

    #[test]
    fn newline_inside_quotes_not_detected_as_shell_operator() {
        let parsed = parse_command_line("python -c \"line1\nline2\"")
            .expect("newline inside quotes should be allowed");
        assert!(!parsed.contains_shell_operator);
    }

    #[test]
    fn newline_outside_quotes_detected_as_shell_operator() {
        let parsed = parse_command_line("echo hello\necho world").expect("should parse");
        assert!(parsed.contains_shell_operator);
    }
}
