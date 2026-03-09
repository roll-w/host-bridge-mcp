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

const CHAR_TRUNCATION_MARKER: &str = "... [truncated] ...";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OutputRenderOptions {
    head_lines: Option<usize>,
    tail_lines: Option<usize>,
    max_chars: Option<usize>,
}

impl OutputRenderOptions {
    pub(super) fn new(
        head_lines: Option<u64>,
        tail_lines: Option<u64>,
        max_chars: Option<u64>,
    ) -> Self {
        Self {
            head_lines: normalize_line_limit(head_lines),
            tail_lines: normalize_line_limit(tail_lines),
            max_chars: max_chars
                .filter(|value| *value > 0)
                .map(saturating_u64_to_usize),
        }
    }

    pub(super) fn apply(&self, text: String) -> String {
        let text = self.apply_line_limits(text);
        self.apply_char_limit(text)
    }

    fn apply_line_limits(&self, text: String) -> String {
        let effective_head = self.head_lines.filter(|value| *value > 0);
        let effective_tail = self.tail_lines.filter(|value| *value > 0);

        if effective_head.is_none() && effective_tail.is_none() {
            return if self.head_lines.is_some() || self.tail_lines.is_some() {
                String::new()
            } else {
                text
            };
        }

        let lines = split_lines(&text);
        match (effective_head, effective_tail) {
            (Some(head_lines), Some(tail_lines)) => {
                if head_lines.saturating_add(tail_lines) >= lines.len() {
                    return text;
                }

                render_head_and_tail(&lines, head_lines, tail_lines)
            }
            (Some(head_lines), None) => lines.into_iter().take(head_lines).collect(),
            (None, Some(tail_lines)) => {
                let start = lines.len().saturating_sub(tail_lines);
                lines[start..].concat()
            }
            (None, None) => text,
        }
    }

    fn apply_char_limit(&self, text: String) -> String {
        let Some(max_chars) = self.max_chars else {
            return text;
        };

        if text.chars().count() <= max_chars {
            return text;
        }

        match (
            self.head_lines.filter(|value| *value > 0),
            self.tail_lines.filter(|value| *value > 0),
        ) {
            (None, Some(_)) => truncate_tail_chars(&text, max_chars),
            (Some(_), None) => truncate_head_chars(&text, max_chars),
            _ => truncate_middle_chars(&text, max_chars),
        }
    }
}

fn normalize_line_limit(limit: Option<u64>) -> Option<usize> {
    limit.map(saturating_u64_to_usize)
}

fn saturating_u64_to_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn render_head_and_tail(lines: &[&str], head_lines: usize, tail_lines: usize) -> String {
    let omitted_lines = lines
        .len()
        .saturating_sub(head_lines.saturating_add(tail_lines));
    if omitted_lines == 0 {
        return lines.concat();
    }

    let mut rendered = String::new();
    rendered.push_str(&lines[..head_lines].concat());
    rendered.push_str(&format!("... [{omitted_lines} lines omitted] ...\n"));
    rendered.push_str(&lines[lines.len() - tail_lines..].concat());
    rendered
}

fn truncate_head_chars(text: &str, max_chars: usize) -> String {
    let marker_chars = CHAR_TRUNCATION_MARKER.chars().count();
    if max_chars <= marker_chars {
        return take_first_chars(text, max_chars);
    }

    let keep_chars = max_chars - marker_chars;
    format!(
        "{}{}",
        take_first_chars(text, keep_chars),
        CHAR_TRUNCATION_MARKER
    )
}

fn truncate_tail_chars(text: &str, max_chars: usize) -> String {
    let marker_chars = CHAR_TRUNCATION_MARKER.chars().count();
    if max_chars <= marker_chars {
        return take_last_chars(text, max_chars);
    }

    let keep_chars = max_chars - marker_chars;
    format!(
        "{}{}",
        CHAR_TRUNCATION_MARKER,
        take_last_chars(text, keep_chars)
    )
}

fn truncate_middle_chars(text: &str, max_chars: usize) -> String {
    let marker_chars = CHAR_TRUNCATION_MARKER.chars().count();
    if max_chars <= marker_chars {
        return take_first_chars(text, max_chars);
    }

    let keep_chars = max_chars - marker_chars;
    let head_chars = keep_chars / 2;
    let tail_chars = keep_chars - head_chars;
    format!(
        "{}{}{}",
        take_first_chars(text, head_chars),
        CHAR_TRUNCATION_MARKER,
        take_last_chars(text, tail_chars)
    )
}

fn take_first_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn take_last_chars(text: &str, count: usize) -> String {
    let total_chars = text.chars().count();
    if count >= total_chars {
        return text.to_string();
    }

    text.chars().skip(total_chars - count).collect()
}

#[cfg(test)]
mod tests {
    use super::{OutputRenderOptions, CHAR_TRUNCATION_MARKER};

    #[test]
    fn keeps_output_unchanged_without_limits() {
        let options = OutputRenderOptions::default();
        let output = "line1\nline2\n".to_string();

        assert_eq!(options.apply(output.clone()), output);
    }

    #[test]
    fn returns_head_lines_when_requested() {
        let options = OutputRenderOptions::new(Some(2), None, None);

        assert_eq!(options.apply(sample_output()), "one\ntwo\n");
    }

    #[test]
    fn returns_tail_lines_when_requested() {
        let options = OutputRenderOptions::new(None, Some(2), None);

        assert_eq!(options.apply(sample_output()), "three\nfour\n");
    }

    #[test]
    fn combines_head_and_tail_with_gap_marker() {
        let options = OutputRenderOptions::new(Some(1), Some(1), None);

        assert_eq!(
            options.apply(sample_output()),
            "one\n... [2 lines omitted] ...\nfour\n"
        );
    }

    #[test]
    fn preserves_full_output_when_head_and_tail_overlap() {
        let options = OutputRenderOptions::new(Some(2), Some(2), None);
        let output = sample_output();

        assert_eq!(options.apply(output.clone()), output);
    }

    #[test]
    fn treats_zero_max_chars_as_unlimited() {
        let options = OutputRenderOptions::new(None, None, Some(0));
        let output = sample_output();

        assert_eq!(options.apply(output.clone()), output);
    }

    #[test]
    fn truncates_tail_output_by_characters() {
        let options = OutputRenderOptions::new(None, Some(1), Some(32));
        let output = "prefix\n0123456789abcdefghijklmnopqrstuvwxyz\n".to_string();
        let rendered = options.apply(output);

        assert!(rendered.starts_with(CHAR_TRUNCATION_MARKER));
        assert!(rendered.ends_with("opqrstuvwxyz\n"));
        assert_eq!(rendered.chars().count(), 32);
    }

    #[test]
    fn truncates_mixed_output_around_the_middle() {
        let options = OutputRenderOptions::new(None, None, Some(24));
        let output = "alpha-beta-gamma-delta-epsilon".to_string();
        let rendered = options.apply(output);

        assert!(rendered.contains(CHAR_TRUNCATION_MARKER));
        assert_eq!(rendered.chars().count(), 24);
    }

    #[test]
    fn zero_head_and_tail_lines_return_empty_output() {
        let options = OutputRenderOptions::new(Some(0), Some(0), None);

        assert!(options.apply(sample_output()).is_empty());
    }
    fn sample_output() -> String {
        "one\ntwo\nthree\nfour\n".to_string()
    }
}
