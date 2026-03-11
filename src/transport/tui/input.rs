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

use crate::application::operator_console::{ConsoleLogLevel, ConsoleSnapshot, OperatorConsole};
use crate::application::shutdown_controller::ShutdownController;
use crate::transport::tui::render::log_line_text;
use crate::transport::tui::state::TuiState;
use crate::transport::tui::terminal::write_terminal_clipboard;
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use std::io;

pub(super) fn handle_input(
    input: Event,
    console: &OperatorConsole,
    snapshot: &ConsoleSnapshot,
    state: &mut TuiState,
    shutdown_controller: &ShutdownController,
) -> bool {
    match input {
        Event::Key(key) => {
            if key.kind != KeyEventKind::Press {
                return false;
            }

            match key.code {
                KeyCode::Char(character) if character.eq_ignore_ascii_case(&'q') => {
                    if shutdown_controller.request_shutdown() {
                        console.push_log(ConsoleLogLevel::Warn, "Shutdown requested from TUI.");
                        return true;
                    }
                    false
                }
                KeyCode::Up => {
                    state.select_previous(snapshot.pending_approvals.len());
                    false
                }
                KeyCode::Down => {
                    state.select_next(snapshot.pending_approvals.len());
                    false
                }
                KeyCode::PageUp => {
                    state.page_up(snapshot);
                    false
                }
                KeyCode::PageDown => {
                    state.page_down(snapshot);
                    false
                }
                KeyCode::Home => {
                    state.jump_head();
                    false
                }
                KeyCode::End => {
                    state.follow_tail();
                    false
                }
                KeyCode::Left => {
                    state.scroll_logs_left(8);
                    state.clear_log_selection();
                    false
                }
                KeyCode::Right => {
                    state.scroll_logs_right(8);
                    state.clear_log_selection();
                    false
                }
                KeyCode::Char('a') => {
                    if let Some(approval) = state.selected_approval(snapshot) {
                        console.resolve_confirmation(approval.id, true);
                    }
                    false
                }
                KeyCode::Char('r') => {
                    if let Some(approval) = state.selected_approval(snapshot) {
                        console.resolve_confirmation(approval.id, false);
                    }
                    false
                }
                _ => false,
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                state.begin_log_selection(mouse.column, mouse.row);
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                state.extend_log_selection(mouse.column, mouse.row);
                false
            }
            MouseEventKind::Up(MouseButton::Left) => {
                state.extend_log_selection(mouse.column, mouse.row);
                if let Some((start, end)) = state.selected_log_range() {
                    match copy_logs_to_clipboard(console, start, end) {
                        Ok(_lines) => {}
                        Err(error) => console.push_log(
                            ConsoleLogLevel::Error,
                            format!("Failed to copy selected logs: {error}"),
                        ),
                    }
                }
                state.clear_log_selection();
                false
            }
            MouseEventKind::ScrollUp => {
                state.scroll_up(snapshot, 3);
                state.clear_log_selection();
                false
            }
            MouseEventKind::ScrollDown => {
                state.scroll_down(snapshot, 3);
                state.clear_log_selection();
                false
            }
            _ => false,
        },
        _ => false,
    }
}

fn copy_logs_to_clipboard(
    console: &OperatorConsole,
    start: usize,
    end: usize,
) -> io::Result<usize> {
    let entries = console.read_logs(start, end.saturating_sub(start).saturating_add(1));
    let copied_lines = entries.len();
    let text = entries
        .iter()
        .map(log_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    write_terminal_clipboard(&text)?;
    Ok(copied_lines)
}
