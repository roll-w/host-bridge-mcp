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

use crate::application::operator_console::{
    ConsoleLogEntry, ConsoleLogLevel, ConsoleSnapshot, OperatorConsole, PendingApprovalView,
};
use crate::application::shutdown_controller::ShutdownController;
use arboard::Clipboard;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, IsTerminal, Stdout, Write};
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

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    set_mouse_capture(true)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn handle_input(
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
                KeyCode::Char('q') => {
                    if key.modifiers == KeyModifiers::NONE && shutdown_controller.request_shutdown()
                    {
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
                        Ok(copied_lines) => console.push_log(
                            ConsoleLogLevel::Info,
                            format!("Copied {copied_lines} log line(s) to clipboard."),
                        ),
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

fn render(
    frame: &mut Frame,
    snapshot: &ConsoleSnapshot,
    state: &mut TuiState,
    console: &OperatorConsole,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Min(8),
        ])
        .split(frame.area());

    render_status_bar(frame, layout[0], snapshot);

    let approval_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(38), Constraint::Min(40)])
        .split(layout[1]);

    render_approval_list(frame, approval_layout[0], snapshot, state);
    render_approval_detail(frame, approval_layout[1], snapshot, state);
    render_logs(frame, layout[2], snapshot, state, console);
}

fn render_status_bar(frame: &mut Frame, area: ratatui::layout::Rect, snapshot: &ConsoleSnapshot) {
    let style = if snapshot.pending_approvals.is_empty() {
        Style::default().fg(Color::Black).bg(Color::Green)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    let text = Line::from(vec![
        Span::raw(" Pending approvals: "),
        Span::styled(
            snapshot.pending_approvals.len().to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  TUI: "),
        Span::styled(
            if snapshot.interactive {
                "online"
            } else {
                "offline"
            },
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(
            "  |  Up/Down select  a approve  r reject  Wheel/PgUp/PgDn logs  drag logs copies  Home/End head-tail  q shutdown",
        ),
    ]);
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn render_approval_list(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    snapshot: &ConsoleSnapshot,
    state: &TuiState,
) {
    let items = if snapshot.pending_approvals.is_empty() {
        vec![ListItem::new(Line::from("No pending approvals"))]
    } else {
        snapshot
            .pending_approvals
            .iter()
            .map(|approval| {
                ListItem::new(Line::from(format!(
                    "{} {}",
                    short_id(approval.id),
                    approval.request.command_line
                )))
            })
            .collect()
    };

    let mut list_state = ListState::default();
    if !snapshot.pending_approvals.is_empty() {
        list_state.select(Some(state.selected_approval_index));
    }

    let list = List::new(items)
        .block(Block::default().title("Approvals").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_approval_detail(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    snapshot: &ConsoleSnapshot,
    state: &TuiState,
) {
    let lines = match state.selected_approval(snapshot) {
        Some(approval) => approval_detail_lines(approval),
        None => vec![Line::from(
            "Select a pending approval to inspect its details.",
        )],
    };

    let detail = Paragraph::new(lines).block(
        Block::default()
            .title("Selected Request")
            .borders(Borders::ALL),
    );
    frame.render_widget(detail, area);
}

fn render_logs(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    snapshot: &ConsoleSnapshot,
    state: &mut TuiState,
    console: &OperatorConsole,
) {
    let visible_height = area.height.saturating_sub(2) as usize;
    state.set_log_page_size(visible_height.max(1));
    let start = state.log_start(snapshot, visible_height.max(1));
    let log_entries = console.read_logs(start, visible_height.max(1));
    state.set_visible_logs(area, start, log_entries.len());
    let log_lines = visible_logs(&log_entries, state, start);
    let end = if log_entries.is_empty() {
        start
    } else {
        start + log_entries.len()
    };
    let title = format!(
        "Logs {}..{} / {} ({})",
        if log_entries.is_empty() { 0 } else { start + 1 },
        end,
        snapshot.total_log_count,
        snapshot.log_file_path
    );
    let logs = Paragraph::new(log_lines).block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(logs, area);
}

fn approval_detail_lines(approval: &PendingApprovalView) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("id         : {}", approval.id)),
        Line::from(format!("commandLine: {}", approval.request.command_line)),
        Line::from(format!("executable : {}", approval.request.executable)),
        Line::from(format!("args       : {:?}", approval.request.args)),
        Line::from(format!(
            "workdir    : {}",
            approval.request.working_directory
        )),
        Line::from(format!("timeoutMs  : {}", approval.request.timeout_ms)),
        Line::from(format!("createdAt  : {:?}", approval.created_at)),
    ];

    if approval.request.env.is_empty() {
        lines.push(Line::from("env        : <none>"));
    } else {
        lines.push(Line::from("env        :"));
        let mut keys = approval.request.env.keys().collect::<Vec<_>>();
        keys.sort();
        for key in keys {
            if let Some(value) = approval.request.env.get(key) {
                lines.push(Line::from(format!("  {key}={value}")));
            }
        }
    }

    lines
}

fn visible_logs(entries: &[ConsoleLogEntry], state: &TuiState, start: usize) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return vec![Line::from("No log entries yet.")];
    }

    entries
        .iter()
        .enumerate()
        .map(|(offset, entry)| log_line(entry, state.is_log_line_selected(start + offset)))
        .collect::<Vec<_>>()
}

fn log_line(entry: &ConsoleLogEntry, selected: bool) -> Line<'static> {
    let (label, color) = match entry.level {
        ConsoleLogLevel::Info => ("INFO", Color::Cyan),
        ConsoleLogLevel::Warn => ("WARN", Color::Yellow),
        ConsoleLogLevel::Error => ("ERROR", Color::Red),
    };

    let line_style = if selected {
        Style::default().bg(Color::DarkGray)
    } else {
        Style::default()
    };

    Line::from(vec![
        Span::styled(
            format!("[{label}] "),
            line_style.fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(entry.message.clone(), line_style),
    ])
}

fn copy_logs_to_clipboard(
    console: &OperatorConsole,
    start: usize,
    end: usize,
) -> Result<usize, arboard::Error> {
    let entries = console.read_logs(start, end.saturating_sub(start).saturating_add(1));
    let copied_lines = entries.len();
    let text = entries
        .iter()
        .map(log_line_text)
        .collect::<Vec<_>>()
        .join("\n");
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(copied_lines)
}

fn log_line_text(entry: &ConsoleLogEntry) -> String {
    let label = match entry.level {
        ConsoleLogLevel::Info => "INFO",
        ConsoleLogLevel::Warn => "WARN",
        ConsoleLogLevel::Error => "ERROR",
    };
    format!("[{label}] {}", entry.message)
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

struct TuiState {
    selected_approval_index: usize,
    log_start_index: usize,
    log_page_size: usize,
    follow_logs: bool,
    last_pending_count: usize,
    logs_area: Option<Rect>,
    visible_log_start: usize,
    visible_log_count: usize,
    active_log_selection: Option<LogSelection>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            selected_approval_index: 0,
            log_start_index: 0,
            log_page_size: 0,
            follow_logs: false,
            last_pending_count: 0,
            logs_area: None,
            visible_log_start: 0,
            visible_log_count: 0,
            active_log_selection: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogSelection {
    anchor_line: usize,
    focus_line: usize,
}

impl TuiState {
    fn sync(&mut self, snapshot: &ConsoleSnapshot) {
        if snapshot.pending_approvals.is_empty() {
            self.selected_approval_index = 0;
        } else {
            self.selected_approval_index = self
                .selected_approval_index
                .min(snapshot.pending_approvals.len().saturating_sub(1));
        }
    }

    fn select_previous(&mut self, approval_count: usize) {
        if approval_count == 0 {
            return;
        }

        self.selected_approval_index = self.selected_approval_index.saturating_sub(1);
    }

    fn select_next(&mut self, approval_count: usize) {
        if approval_count == 0 {
            return;
        }

        self.selected_approval_index = (self.selected_approval_index + 1).min(approval_count - 1);
    }

    fn selected_approval<'a>(
        &self,
        snapshot: &'a ConsoleSnapshot,
    ) -> Option<&'a PendingApprovalView> {
        snapshot.pending_approvals.get(self.selected_approval_index)
    }

    fn set_log_page_size(&mut self, page_size: usize) {
        self.log_page_size = page_size.max(1);
    }

    fn set_visible_logs(&mut self, area: Rect, start: usize, count: usize) {
        self.logs_area = Some(area);
        self.visible_log_start = start;
        self.visible_log_count = count;
        if count == 0 {
            self.active_log_selection = None;
        }
    }

    fn log_start(&self, snapshot: &ConsoleSnapshot, visible_height: usize) -> usize {
        let max_start = snapshot.total_log_count.saturating_sub(visible_height);
        if self.follow_logs {
            max_start
        } else {
            self.log_start_index.min(max_start)
        }
    }

    fn current_log_start(&self, snapshot: &ConsoleSnapshot) -> usize {
        self.log_start(snapshot, self.log_page_size.max(1))
    }

    fn page_up(&mut self, snapshot: &ConsoleSnapshot) {
        self.scroll_up(snapshot, self.log_page_size.saturating_sub(1).max(1));
    }

    fn page_down(&mut self, snapshot: &ConsoleSnapshot) {
        self.scroll_down(snapshot, self.log_page_size.saturating_sub(1).max(1));
    }

    fn scroll_up(&mut self, snapshot: &ConsoleSnapshot, lines: usize) {
        let current = self.current_log_start(snapshot);
        self.follow_logs = false;
        self.log_start_index = current.saturating_sub(lines.max(1));
    }

    fn scroll_down(&mut self, snapshot: &ConsoleSnapshot, lines: usize) {
        let max_start = snapshot
            .total_log_count
            .saturating_sub(self.log_page_size.max(1));
        let current = self.current_log_start(snapshot);
        let next = current.saturating_add(lines.max(1)).min(max_start);
        self.log_start_index = next;
        self.follow_logs = next >= max_start;
    }

    fn jump_head(&mut self) {
        self.follow_logs = false;
        self.log_start_index = 0;
    }

    fn follow_tail(&mut self) {
        self.follow_logs = true;
    }

    fn begin_log_selection(&mut self, column: u16, row: u16) {
        self.active_log_selection = self
            .log_index_at(column, row)
            .map(|line_index| LogSelection {
                anchor_line: line_index,
                focus_line: line_index,
            });
    }

    fn extend_log_selection(&mut self, column: u16, row: u16) {
        let Some(line_index) = self.log_index_at(column, row) else {
            return;
        };
        if let Some(selection) = self.active_log_selection.as_mut() {
            selection.focus_line = line_index;
        }
    }

    fn clear_log_selection(&mut self) {
        self.active_log_selection = None;
    }

    fn selected_log_range(&self) -> Option<(usize, usize)> {
        let selection = self.active_log_selection?;
        Some((
            selection.anchor_line.min(selection.focus_line),
            selection.anchor_line.max(selection.focus_line),
        ))
    }

    fn is_log_line_selected(&self, line_index: usize) -> bool {
        let Some((start, end)) = self.selected_log_range() else {
            return false;
        };
        (start..=end).contains(&line_index)
    }

    fn log_index_at(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.logs_area?;
        if area.width <= 2 || area.height <= 2 {
            return None;
        }

        let inner_left = area.x.saturating_add(1);
        let inner_top = area.y.saturating_add(1);
        let inner_right = area.x.saturating_add(area.width.saturating_sub(1));
        let inner_bottom = area.y.saturating_add(area.height.saturating_sub(1));
        if column < inner_left || column >= inner_right || row < inner_top || row >= inner_bottom {
            return None;
        }

        let relative_row = row.saturating_sub(inner_top) as usize;
        if relative_row >= self.visible_log_count {
            return None;
        }

        Some(self.visible_log_start + relative_row)
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

struct TerminalGuard;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

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
        state.set_visible_logs(Rect::new(0, 0, 40, 6), 10, 4);

        state.begin_log_selection(2, 2);
        state.extend_log_selection(2, 4);

        assert_eq!(state.selected_log_range(), Some((11, 13)));
        assert!(state.is_log_line_selected(12));
        assert!(!state.is_log_line_selected(14));
    }
}
