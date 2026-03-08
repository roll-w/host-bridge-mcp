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

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout, Write};

pub(super) fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    set_mouse_capture(true)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

pub(super) fn write_terminal_clipboard(text: &str) -> io::Result<()> {
    let encoded = base64_encode(text.as_bytes());
    print!("\x1b]52;c;{encoded}\x07");
    io::stdout().flush()
}

pub(super) struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

fn set_mouse_capture(enabled: bool) -> io::Result<()> {
    let mut stdout = io::stdout();
    if enabled {
        execute!(stdout, EnableMouseCapture)
    } else {
        execute!(stdout, DisableMouseCapture)
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut index = 0;

    while index < bytes.len() {
        let first = bytes[index];
        let second = bytes.get(index + 1).copied();
        let third = bytes.get(index + 2).copied();

        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(
            ALPHABET[((first & 0b0000_0011) << 4 | second.unwrap_or(0) >> 4) as usize] as char,
        );

        match second {
            Some(second) => encoded.push(
                ALPHABET[((second & 0b0000_1111) << 2 | third.unwrap_or(0) >> 6) as usize] as char,
            ),
            None => encoded.push('='),
        }

        match third {
            Some(third) => encoded.push(ALPHABET[(third & 0b0011_1111) as usize] as char),
            None => encoded.push('='),
        }

        index += 3;
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn base64_encoder_matches_expected_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }
}
