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

mod input;
mod render;
mod state;
mod terminal;

use self::input::handle_input;
use self::render::render;
use self::state::TuiState;
use self::terminal::{setup_terminal, TerminalGuard};
use crate::application::operator_console::OperatorConsole;
use crate::application::shutdown_controller::ShutdownController;
use crossterm::event;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

pub fn start(console: OperatorConsole, shutdown_controller: ShutdownController) -> bool {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    console.set_interactive(interactive);
    if !interactive {
        return false;
    }

    let tui_console = console.clone();
    let tui_shutdown = shutdown_controller.clone();
    let spawn_result = std::thread::Builder::new()
        .name("host-bridge-tui".to_string())
        .spawn(move || {
            if let Err(error) = run(tui_console.clone(), tui_shutdown.clone()) {
                tui_console.shutdown(&format!("TUI stopped: {error}"));
            }
        });

    if spawn_result.is_err() {
        console.set_interactive(false);
        return false;
    }

    true
}

fn run(console: OperatorConsole, shutdown_controller: ShutdownController) -> io::Result<()> {
    let mut state = TuiState::default();
    let mut terminal = setup_terminal()?;
    let _guard = TerminalGuard;

    loop {
        let snapshot = console.snapshot();
        state.sync(&snapshot);

        if snapshot.pending_approvals.len() > state.last_pending_count {
            print!("\x07");
            io::stdout().flush()?;
        }
        state.last_pending_count = snapshot.pending_approvals.len();

        terminal.draw(|frame| render(frame, &snapshot, &mut state, &console))?;

        if event::poll(Duration::from_millis(150))? {
            let input = event::read()?;
            if handle_input(input, &console, &snapshot, &mut state, &shutdown_controller) {
                break;
            }
        }
    }

    console.shutdown("TUI stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::operator_console::ConsoleSnapshot;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn snapshot(total_log_count: usize) -> ConsoleSnapshot {
        ConsoleSnapshot {
            interactive: true,
            total_log_count,
            log_file_path: "test.log".to_string(),
            pending_approvals: Vec::new(),
        }
    }

    #[test]
    fn page_up_from_tail_moves_one_page_back() {
        let mut state = TuiState {
            log_page_size: 10,
            follow_logs: true,
            ..TuiState::default()
        };

        state.page_up(&snapshot(100));
        assert_eq!(state.log_start_index, 81);
        assert!(!state.follow_logs);
    }

    #[test]
    fn page_down_reaches_tail_and_restores_follow_mode() {
        let mut state = TuiState {
            log_page_size: 10,
            log_start_index: 81,
            follow_logs: false,
            ..TuiState::default()
        };

        state.page_down(&snapshot(100));
        assert_eq!(state.log_start_index, 90);
        assert!(state.follow_logs);
    }

    #[test]
    fn modified_q_does_not_request_shutdown() {
        let console = OperatorConsole::default();
        let shutdown_controller = ShutdownController::default();
        let snapshot = snapshot(0);
        let mut state = TuiState::default();

        let should_quit = handle_input(
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::SHIFT)),
            &console,
            &snapshot,
            &mut state,
            &shutdown_controller,
        );

        assert!(!should_quit);
        assert!(shutdown_controller.request_shutdown());
    }

    #[test]
    fn drag_selection_tracks_visible_log_range() {
        let mut state = TuiState::default();
        state.set_visible_logs(ratatui::layout::Rect::new(0, 0, 40, 6), 10, 4);

        state.begin_log_selection(2, 2);
        state.extend_log_selection(2, 4);

        assert_eq!(state.selected_log_range(), Some((11, 13)));
        assert!(state.is_log_line_selected(12));
        assert!(!state.is_log_line_selected(14));
    }
}
