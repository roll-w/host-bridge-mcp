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

pub(super) fn sanitize_console_text(input: &str) -> String {
    enum State {
        Normal,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut sanitized = String::with_capacity(input.len());
    let mut state = State::Normal;

    for ch in input.chars() {
        match state {
            State::Normal => {
                if ch == '\u{1b}' {
                    state = State::Escape;
                } else if ch == '\n' || ch == '\t' || !ch.is_control() {
                    sanitized.push(ch);
                }
            }
            State::Escape => {
                state = match ch {
                    '[' => State::Csi,
                    ']' => State::Osc,
                    _ => State::Normal,
                };
            }
            State::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = State::Normal;
                }
            }
            State::Osc => {
                if ch == '\u{7}' {
                    state = State::Normal;
                } else if ch == '\u{1b}' {
                    state = State::OscEscape;
                }
            }
            State::OscEscape => {
                state = if ch == '\\' {
                    State::Normal
                } else {
                    State::Osc
                };
            }
        }
    }

    sanitized
}
