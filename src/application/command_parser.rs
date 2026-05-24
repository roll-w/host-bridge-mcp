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

pub fn parse_command_line(input: &str) -> Result<ParsedCommand, CommandParseError> {
    let has_shell_operator = detect_shell_operators(input);
    let tokens = split_command_line(input)?;
    let mut iter = tokens.into_iter();
    let program = iter.next().ok_or(CommandParseError::Empty)?;
    let args = iter.collect::<Vec<_>>();

    Ok(ParsedCommand {
        program,
        args,
        contains_shell_operator: has_shell_operator,
    })
}

fn split_command_line(input: &str) -> Result<Vec<String>, CommandParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }

    if escaped {
        current.push('\\');
    }

    if in_single_quote || in_double_quote {
        return Err(CommandParseError::UnclosedQuote);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    if tokens.is_empty() {
        return Err(CommandParseError::Empty);
    }

    Ok(tokens)
}

fn detect_shell_operators(input: &str) -> bool {
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if in_single_quote || in_double_quote {
            continue;
        }

        if ch == ';' || ch == '|' || ch == '\n' || ch == '\r' {
            return true;
        }

        if ch == '&' && matches!(chars.peek(), Some('&')) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let parsed =
            parse_command_line("ls -la | grep foo").expect("should parse piped command");
        assert!(parsed.contains_shell_operator);
        assert_eq!(parsed.program, "ls");
    }

    #[test]
    fn detects_semicolon_operator() {
        let parsed =
            parse_command_line("cd /tmp; ls").expect("should parse semicolon command");
        assert!(parsed.contains_shell_operator);
        assert_eq!(parsed.program, "cd");
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
    fn newline_inside_quotes_not_detected_as_shell_operator() {
        let parsed = parse_command_line("python -c \"line1\nline2\"")
            .expect("newline inside quotes should be allowed");
        assert!(!parsed.contains_shell_operator);
    }

    #[test]
    fn newline_outside_quotes_detected_as_shell_operator() {
        let parsed = parse_command_line("echo hello\necho world")
            .expect("should parse");
        assert!(parsed.contains_shell_operator);
    }
}
