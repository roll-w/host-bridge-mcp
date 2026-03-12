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
    pub(super) log_horizontal_offset: usize,
    pub(super) max_log_horizontal_offset: usize,
    pub(super) follow_logs: bool,
    pub(super) last_pending_count: usize,
    pub(super) logs_area: Option<Rect>,
    pub(super) log_scroll_area: Option<Rect>,
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
            log_horizontal_offset: 0,
            max_log_horizontal_offset: 0,
            follow_logs: false,
            last_pending_count: 0,
            logs_area: None,
            log_scroll_area: None,
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

    pub(super) fn set_log_scroll_area(&mut self, area: Rect) {
        self.log_scroll_area = Some(area);
    }

    pub(super) fn set_visible_logs(
        &mut self,
        area: Rect,
        start: usize,
        count: usize,
        max_line_width: usize,
    ) {
        self.logs_area = Some(area);
        self.visible_log_start = start;
        self.visible_log_count = count;
        if count == 0 {
            self.active_log_selection = None;
            self.log_horizontal_offset = 0;
            self.max_log_horizontal_offset = 0;
            return;
        }

        self.max_log_horizontal_offset = max_line_width.saturating_sub(log_view_width(area));
        self.log_horizontal_offset = self
            .log_horizontal_offset
            .min(self.max_log_horizontal_offset);
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

    pub(super) fn scroll_logs_left(&mut self, columns: usize) {
        self.log_horizontal_offset = self.log_horizontal_offset.saturating_sub(columns.max(1));
    }

    pub(super) fn scroll_logs_right(&mut self, columns: usize) {
        self.log_horizontal_offset = self
            .log_horizontal_offset
            .saturating_add(columns.max(1))
            .min(self.max_log_horizontal_offset);
    }

    pub(super) fn log_horizontal_offset(&self) -> u16 {
        self.log_horizontal_offset.min(u16::MAX as usize) as u16
    }

    pub(super) fn log_horizontal_offset_columns(&self) -> usize {
        self.log_horizontal_offset
    }

    pub(super) fn is_log_scroll_hit(&self, column: u16, row: u16) -> bool {
        self.log_scroll_area
            .map(|area| rect_contains(area, column, row))
            .unwrap_or(false)
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
        if area.width == 0 || area.height == 0 || !rect_contains(area, column, row) {
            return None;
        }

        let relative_row = row.saturating_sub(area.y) as usize;
        if relative_row >= self.visible_log_count {
            return None;
        }

        Some(self.visible_log_start + relative_row)
    }
}

fn log_view_width(area: Rect) -> usize {
    area.width as usize
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    let right = area.x.saturating_add(area.width);
    let bottom = area.y.saturating_add(area.height);
    column >= area.x && column < right && row >= area.y && row < bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_scroll_is_clamped_to_visible_content() {
        let mut state = TuiState::default();
        state.set_visible_logs(Rect::new(0, 0, 20, 6), 0, 3, 40);

        state.scroll_logs_right(64);
        assert_eq!(state.log_horizontal_offset_columns(), 20);

        state.scroll_logs_left(5);
        assert_eq!(state.log_horizontal_offset_columns(), 15);

        state.scroll_logs_left(64);
        assert_eq!(state.log_horizontal_offset_columns(), 0);
    }

    #[test]
    fn log_selection_uses_content_area_coordinates() {
        let mut state = TuiState::default();
        state.set_visible_logs(Rect::new(10, 20, 5, 3), 7, 3, 32);

        state.begin_log_selection(10, 20);
        state.extend_log_selection(14, 22);

        assert_eq!(state.selected_log_range(), Some((7, 9)));
    }

    #[test]
    fn log_scroll_hit_uses_scrollable_area() {
        let mut state = TuiState::default();
        state.set_log_scroll_area(Rect::new(4, 5, 6, 3));

        assert!(state.is_log_scroll_hit(4, 5));
        assert!(state.is_log_scroll_hit(9, 7));
        assert!(!state.is_log_scroll_hit(10, 7));
        assert!(!state.is_log_scroll_hit(9, 8));
    }
}
