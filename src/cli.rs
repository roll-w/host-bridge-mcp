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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub config_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}\n\n{usage}")]
pub struct CliError {
    pub message: String,
    pub usage: String,
}

pub fn parse_args<I>(args: I) -> Result<CliOptions, CliError>
where
    I: IntoIterator<Item=String>,
{
    let mut iterator = args.into_iter();
    let program_name = iterator
        .next()
        .unwrap_or_else(|| "host-bridge-mcp".to_string());
    let usage = usage_text(&program_name);

    let mut config_path: Option<String> = None;

    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "-c" | "--config" => {
                let Some(value) = iterator.next() else {
                    return Err(CliError {
                        message: "missing value for --config".to_string(),
                        usage,
                    });
                };
                config_path = Some(value);
            }
            _ => {
                if let Some(value) = argument.strip_prefix("--config=") {
                    if value.trim().is_empty() {
                        return Err(CliError {
                            message: "--config requires a non-empty value".to_string(),
                            usage,
                        });
                    }
                    config_path = Some(value.to_string());
                } else {
                    return Err(CliError {
                        message: format!("unknown argument: {argument}"),
                        usage,
                    });
                }
            }
        }
    }

    Ok(CliOptions { config_path })
}

fn usage_text(program_name: &str) -> String {
    format!(
        "Usage:\n  {program_name} [OPTIONS]\n\nOptions:\n  -c, --config <PATH>   Set configuration file path"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_argument() {
        let outcome = parse_args(vec![
            "host-bridge-mcp".to_string(),
            "--config".to_string(),
            "custom.toml".to_string(),
        ])
            .expect("should parse");

        assert_eq!(
            outcome,
            CliOptions {
                config_path: Some("custom.toml".to_string()),
            }
        );
    }

    #[test]
    fn reject_help_argument() {
        let error = parse_args(vec!["host-bridge-mcp".to_string(), "--help".to_string()])
            .expect_err("--help is not supported");
        assert!(error.message.contains("unknown argument"));
    }
}
