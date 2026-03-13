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
use crate::transport::tui::state::TuiState;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub(super) fn render(
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

pub(super) fn visible_logs(
    entries: &[ConsoleLogEntry],
    state: &TuiState,
    start: usize,
) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return vec![Line::from("No log entries yet.")];
    }

    entries
        .iter()
        .enumerate()
        .map(|(offset, entry)| log_line(entry, state.is_log_line_selected(start + offset)))
        .collect::<Vec<_>>()
}

pub(super) fn log_line_text(entry: &ConsoleLogEntry) -> String {
    let label = match entry.level {
        ConsoleLogLevel::Info => "INFO",
        ConsoleLogLevel::Warn => "WARN",
        ConsoleLogLevel::Error => "ERROR",
    };
    format!("{} {:>5} {}", entry.timestamp, label, entry.message)
}

fn render_status_bar(frame: &mut Frame, area: Rect, snapshot: &ConsoleSnapshot) {
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
            "  |  Up/Down select  Left/Right x-scroll  a approve  r reject  Wheel/PgUp/PgDn logs  Home/End head-tail  q shutdown",
        ),
    ]);
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn render_approval_list(
    frame: &mut Frame,
    area: Rect,
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
    area: Rect,
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
    area: Rect,
    snapshot: &ConsoleSnapshot,
    state: &mut TuiState,
    console: &OperatorConsole,
) {
    let visible_height = area.height.saturating_sub(2) as usize;
    state.set_log_page_size(visible_height.max(1));
    let start = state.log_start(snapshot, visible_height.max(1));
    let log_entries = console.read_logs(start, visible_height.max(1));
    let log_lines = visible_logs(&log_entries, state, start);
    state.set_visible_logs(
        area,
        start,
        log_entries.len(),
        max_log_line_width(&log_entries),
    );
    let end = if log_entries.is_empty() {
        start
    } else {
        start + log_entries.len()
    };
    let horizontal_offset = state.log_horizontal_offset_columns();
    let title = if horizontal_offset == 0 {
        format!(
            "Logs {}..{} / {} ({})",
            if log_entries.is_empty() { 0 } else { start + 1 },
            end,
            snapshot.total_log_count,
            snapshot.log_file_path
        )
    } else {
        format!(
            "Logs {}..{} / {} [x:{}] ({})",
            if log_entries.is_empty() { 0 } else { start + 1 },
            end,
            snapshot.total_log_count,
            horizontal_offset,
            snapshot.log_file_path
        )
    };
    let logs = Paragraph::new(log_lines)
        .scroll((0, state.log_horizontal_offset()))
        .block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(logs, area);
}

fn max_log_line_width(entries: &[ConsoleLogEntry]) -> usize {
    entries
        .iter()
        .map(log_line_text)
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

fn approval_detail_lines(approval: &PendingApprovalView) -> Vec<Line<'static>> {
    let working_directory = approval
        .request
        .working_directory
        .as_deref()
        .unwrap_or("<remote default>");
    let mut lines = vec![
        Line::from(format!("id         : {}", approval.id)),
        Line::from(format!("server     : {}", approval.request.server)),
        Line::from(format!("platform   : {}", approval.request.platform)),
        Line::from(format!("commandLine: {}", approval.request.command_line)),
        Line::from(format!("executable : {}", approval.request.executable)),
        Line::from(format!("args       : {:?}", approval.request.args)),
        Line::from(format!("workdir    : {}", working_directory)),
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
    let timestamp_style = if selected {
        line_style.fg(Color::Gray)
    } else {
        line_style.fg(Color::DarkGray)
    };

    Line::from(vec![
        Span::styled(format!("{} ", entry.timestamp), timestamp_style),
        Span::styled(
            format!("{label:>5}"),
            line_style.fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {}", entry.message), line_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_line_text_uses_timestamp_and_aligned_level() {
        let info_entry = ConsoleLogEntry {
            timestamp: "2026-03-09T16:16:21.751592Z".to_string(),
            level: ConsoleLogLevel::Info,
            message: "Execution submitted".to_string(),
        };
        let error_entry = ConsoleLogEntry {
            timestamp: "2026-03-09T16:16:21.751592Z".to_string(),
            level: ConsoleLogLevel::Error,
            message: "Execution failed".to_string(),
        };

        assert_eq!(
            log_line_text(&info_entry),
            "2026-03-09T16:16:21.751592Z  INFO Execution submitted"
        );
        assert_eq!(
            log_line_text(&error_entry),
            "2026-03-09T16:16:21.751592Z ERROR Execution failed"
        );
    }
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string().chars().take(8).collect()
}
