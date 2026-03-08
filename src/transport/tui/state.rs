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

use crate::application::operator_console::{ConsoleSnapshot, PendingApprovalView};
use ratatui::layout::Rect;

pub(super) struct TuiState {
    pub(super) selected_approval_index: usize,
    pub(super) log_start_index: usize,
    pub(super) log_page_size: usize,
    pub(super) follow_logs: bool,
    pub(super) last_pending_count: usize,
    pub(super) logs_area: Option<Rect>,
    pub(super) visible_log_start: usize,
    pub(super) visible_log_count: usize,
    pub(super) active_log_selection: Option<LogSelection>,
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
pub(super) struct LogSelection {
    anchor_line: usize,
    focus_line: usize,
}

impl TuiState {
    pub(super) fn sync(&mut self, snapshot: &ConsoleSnapshot) {
        if snapshot.pending_approvals.is_empty() {
            self.selected_approval_index = 0;
        } else {
            self.selected_approval_index = self
                .selected_approval_index
                .min(snapshot.pending_approvals.len().saturating_sub(1));
        }
    }

    pub(super) fn select_previous(&mut self, approval_count: usize) {
        if approval_count == 0 {
            return;
        }

        self.selected_approval_index = self.selected_approval_index.saturating_sub(1);
    }

    pub(super) fn select_next(&mut self, approval_count: usize) {
        if approval_count == 0 {
            return;
        }

        self.selected_approval_index = (self.selected_approval_index + 1).min(approval_count - 1);
    }

    pub(super) fn selected_approval<'a>(
        &self,
        snapshot: &'a ConsoleSnapshot,
    ) -> Option<&'a PendingApprovalView> {
        snapshot.pending_approvals.get(self.selected_approval_index)
    }

    pub(super) fn set_log_page_size(&mut self, page_size: usize) {
        self.log_page_size = page_size.max(1);
    }

    pub(super) fn set_visible_logs(&mut self, area: Rect, start: usize, count: usize) {
        self.logs_area = Some(area);
        self.visible_log_start = start;
        self.visible_log_count = count;
        if count == 0 {
            self.active_log_selection = None;
        }
    }

    pub(super) fn log_start(&self, snapshot: &ConsoleSnapshot, visible_height: usize) -> usize {
        let max_start = snapshot.total_log_count.saturating_sub(visible_height);
        if self.follow_logs {
            max_start
        } else {
            self.log_start_index.min(max_start)
        }
    }

    pub(super) fn page_up(&mut self, snapshot: &ConsoleSnapshot) {
        self.scroll_up(snapshot, self.log_page_size.saturating_sub(1).max(1));
    }

    pub(super) fn page_down(&mut self, snapshot: &ConsoleSnapshot) {
        self.scroll_down(snapshot, self.log_page_size.saturating_sub(1).max(1));
    }

    pub(super) fn scroll_up(&mut self, snapshot: &ConsoleSnapshot, lines: usize) {
        let current = self.current_log_start(snapshot);
        self.follow_logs = false;
        self.log_start_index = current.saturating_sub(lines.max(1));
    }

    pub(super) fn scroll_down(&mut self, snapshot: &ConsoleSnapshot, lines: usize) {
        let max_start = snapshot
            .total_log_count
            .saturating_sub(self.log_page_size.max(1));
        let current = self.current_log_start(snapshot);
        let next = current.saturating_add(lines.max(1)).min(max_start);
        self.log_start_index = next;
        self.follow_logs = next >= max_start;
    }

    pub(super) fn jump_head(&mut self) {
        self.follow_logs = false;
        self.log_start_index = 0;
    }

    pub(super) fn follow_tail(&mut self) {
        self.follow_logs = true;
    }

    pub(super) fn begin_log_selection(&mut self, column: u16, row: u16) {
        self.active_log_selection = self
            .log_index_at(column, row)
            .map(|line_index| LogSelection {
                anchor_line: line_index,
                focus_line: line_index,
            });
    }

    pub(super) fn extend_log_selection(&mut self, column: u16, row: u16) {
        let Some(line_index) = self.log_index_at(column, row) else {
            return;
        };
        if let Some(selection) = self.active_log_selection.as_mut() {
            selection.focus_line = line_index;
        }
    }

    pub(super) fn clear_log_selection(&mut self) {
        self.active_log_selection = None;
    }

    pub(super) fn selected_log_range(&self) -> Option<(usize, usize)> {
        let selection = self.active_log_selection?;
        Some((
            selection.anchor_line.min(selection.focus_line),
            selection.anchor_line.max(selection.focus_line),
        ))
    }

    pub(super) fn is_log_line_selected(&self, line_index: usize) -> bool {
        let Some((start, end)) = self.selected_log_range() else {
            return false;
        };
        (start..=end).contains(&line_index)
    }

    fn current_log_start(&self, snapshot: &ConsoleSnapshot) -> usize {
        self.log_start(snapshot, self.log_page_size.max(1))
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
