//! Mouse Click and Scroll Support for Ratatui Inline Lists
//!
//! Provides optional, high-performance mouse event handling for interactive terminal UI:
//! - RAII `MouseCaptureGuard` for enabling and disabling terminal mouse capture.
//! - `MouseConfig` for configurable scroll speed, double-click timing, and drag behavior.
//! - `MouseTracker` for stateful tracking of clicks, double/triple clicks, drag gestures, and hover positions.
//! - `ListMouseHandler` for translating raw mouse events into high-level list actions:
//!   - Single-click row selection
//!   - Double-click (or click-on-selected) row execution/submission
//!   - Mouse wheel scroll up/down with configurable step delta
//!   - Right-click or outside-click cancellation
//! - `TabBarMouseHandler` for detecting clicks on inline tab bars (e.g. provider tabs in model picker).
//! - `ScrollController` for smooth list scrolling, viewport visibility clamping, and scrollbar drag tracking.

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
};
use ratatui::layout::Rect;
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Mouse Capture RAII Guard & Helpers
// ---------------------------------------------------------------------------

/// Enables mouse capture in terminal stdout.
pub fn enable_mouse_capture() -> std::io::Result<()> {
    execute!(stdout(), EnableMouseCapture)?;
    stdout().flush()
}

/// Disables mouse capture in terminal stdout.
pub fn disable_mouse_capture() -> std::io::Result<()> {
    execute!(stdout(), DisableMouseCapture)?;
    stdout().flush()
}

/// RAII Guard that enables mouse event capture on creation and restores the terminal on drop.
pub struct MouseCaptureGuard {
    active: bool,
}

impl MouseCaptureGuard {
    /// Enables mouse event capture and returns an active guard.
    pub fn enter() -> std::io::Result<Self> {
        enable_mouse_capture()?;
        Ok(Self { active: true })
    }

    /// Conditionally enables mouse event capture if `enabled` is true.
    pub fn enter_opt(enabled: bool) -> std::io::Result<Option<Self>> {
        if enabled {
            Ok(Some(Self::enter()?))
        } else {
            Ok(None)
        }
    }

    /// Returns whether mouse capture is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Manually deactivates mouse capture before drop.
    pub fn deactivate(&mut self) -> std::io::Result<()> {
        if self.active {
            self.active = false;
            disable_mouse_capture()?;
        }
        Ok(())
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_mouse_capture();
            self.active = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Mouse Configuration
// ---------------------------------------------------------------------------

/// Configuration options for mouse interaction in inline terminal lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseConfig {
    /// Whether mouse support is enabled.
    pub enabled: bool,
    /// Number of items/rows to scroll per mouse wheel tick.
    pub scroll_speed: usize,
    /// Maximum duration between clicks to be recognized as a double click (milliseconds).
    pub double_click_threshold_ms: u64,
    /// Whether clicking an already selected row submits/confirms the item immediately.
    pub click_selected_to_submit: bool,
    /// Whether right-clicking cancels/closes the inline picker.
    pub right_click_to_cancel: bool,
    /// Whether clicking outside the list bounds cancels/closes the picker.
    pub click_outside_to_cancel: bool,
    /// Whether mouse hovering updates selection or highlight.
    pub hover_selection: bool,
    /// Invert mouse wheel scroll direction.
    pub invert_scroll: bool,
    /// Number of rows to jump on page-up/page-down gestures.
    pub page_scroll_size: usize,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scroll_speed: 3,
            double_click_threshold_ms: 400,
            click_selected_to_submit: true,
            right_click_to_cancel: true,
            click_outside_to_cancel: false,
            hover_selection: false,
            invert_scroll: false,
            page_scroll_size: 10,
        }
    }
}

impl MouseConfig {
    /// Creates a default mouse configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether mouse support is enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets scroll step speed (number of items per wheel click).
    pub fn with_scroll_speed(mut self, speed: usize) -> Self {
        self.scroll_speed = speed.max(1);
        self
    }

    /// Sets double click duration threshold in milliseconds.
    pub fn with_double_click_threshold_ms(mut self, threshold_ms: u64) -> Self {
        self.double_click_threshold_ms = threshold_ms;
        self
    }

    /// Sets whether clicking an already selected item triggers submission.
    pub fn with_click_selected_to_submit(mut self, submit: bool) -> Self {
        self.click_selected_to_submit = submit;
        self
    }

    /// Sets whether right-clicking cancels the active prompt.
    pub fn with_right_click_to_cancel(mut self, cancel: bool) -> Self {
        self.right_click_to_cancel = cancel;
        self
    }

    /// Sets whether clicking outside the list cancels.
    pub fn with_click_outside_to_cancel(mut self, cancel: bool) -> Self {
        self.click_outside_to_cancel = cancel;
        self
    }

    /// Sets whether hover updates selection.
    pub fn with_hover_selection(mut self, hover: bool) -> Self {
        self.hover_selection = hover;
        self
    }

    /// Inverts mouse wheel scroll direction.
    pub fn with_invert_scroll(mut self, invert: bool) -> Self {
        self.invert_scroll = invert;
        self
    }

    /// Sets page scroll step size.
    pub fn with_page_scroll_size(mut self, page_size: usize) -> Self {
        self.page_scroll_size = page_size.max(1);
        self
    }
}

// ---------------------------------------------------------------------------
// Click & Event Types
// ---------------------------------------------------------------------------

/// Detected classification of a mouse click sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickKind {
    /// Single click.
    Single,
    /// Double click within threshold.
    Double,
    /// Triple click within threshold.
    Triple,
}

/// Normalized mouse button representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonKind {
    Left,
    Right,
    Middle,
    Other,
}

impl From<MouseButton> for MouseButtonKind {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => MouseButtonKind::Left,
            MouseButton::Right => MouseButtonKind::Right,
            MouseButton::Middle => MouseButtonKind::Middle,
        }
    }
}

/// Action produced by processing a mouse event on an inline list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListMouseAction {
    /// Select the item at the given 0-based absolute list index.
    Select(usize),
    /// Select and confirm/submit the item at the given 0-based index.
    SelectAndSubmit(usize),
    /// Scroll up by the given number of items.
    ScrollUp { delta: usize },
    /// Scroll down by the given number of items.
    ScrollDown { delta: usize },
    /// Page up in list.
    PageUp,
    /// Page down in list.
    PageDown,
    /// Hover over item at given absolute index.
    Hover(usize),
    /// Clicked on tab bar, switching to tab at given index.
    SwitchTab(usize),
    /// Submit currently active item.
    SubmitCurrent,
    /// Cancel / Close the interactive prompt.
    Cancel,
    /// Mouse event was processed but triggered no state change.
    Ignored,
}

impl ListMouseAction {
    /// Returns true if the action requests a selection change.
    pub fn is_select(&self) -> bool {
        matches!(
            self,
            ListMouseAction::Select(_) | ListMouseAction::SelectAndSubmit(_)
        )
    }

    /// Returns true if the action requests item submission/confirmation.
    pub fn is_submit(&self) -> bool {
        matches!(
            self,
            ListMouseAction::SelectAndSubmit(_) | ListMouseAction::SubmitCurrent
        )
    }

    /// Returns true if the action represents a scroll request.
    pub fn is_scroll(&self) -> bool {
        matches!(
            self,
            ListMouseAction::ScrollUp { .. }
                | ListMouseAction::ScrollDown { .. }
                | ListMouseAction::PageUp
                | ListMouseAction::PageDown
        )
    }

    /// Returns target selected index if action specifies one.
    pub fn selected_index(&self) -> Option<usize> {
        match self {
            ListMouseAction::Select(idx) | ListMouseAction::SelectAndSubmit(idx) => Some(*idx),
            _ => None,
        }
    }

    /// Returns signed scroll delta if action is a scroll.
    pub fn scroll_delta(&self) -> Option<isize> {
        match self {
            ListMouseAction::ScrollUp { delta } => Some(-(*delta as isize)),
            ListMouseAction::ScrollDown { delta } => Some(*delta as isize),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Mouse State Tracker
// ---------------------------------------------------------------------------

/// Tracks click timestamps, positions, and drag gestures to detect double-clicks and drags.
#[derive(Debug, Clone)]
pub struct MouseTracker {
    last_click_time: Option<Instant>,
    last_click_pos: Option<(u16, u16)>,
    last_click_button: Option<MouseButton>,
    click_count: usize,
    drag_start: Option<(u16, u16)>,
    last_pos: Option<(u16, u16)>,
    is_dragging: bool,
    config: MouseConfig,
}

impl Default for MouseTracker {
    fn default() -> Self {
        Self::new(MouseConfig::default())
    }
}

impl MouseTracker {
    /// Creates a new tracker with the specified configuration.
    pub fn new(config: MouseConfig) -> Self {
        Self {
            last_click_time: None,
            last_click_pos: None,
            last_click_button: None,
            click_count: 0,
            drag_start: None,
            last_pos: None,
            is_dragging: false,
            config,
        }
    }

    /// Records a mouse button press or click event and returns the detected click kind.
    pub fn record_click(
        &mut self,
        col: u16,
        row: u16,
        button: MouseButton,
        now: Instant,
    ) -> ClickKind {
        let threshold = Duration::from_millis(self.config.double_click_threshold_ms);

        let is_consecutive = if let (Some(last_time), Some(last_pos), Some(last_btn)) = (
            self.last_click_time,
            self.last_click_pos,
            self.last_click_button,
        ) {
            let elapsed = now.duration_since(last_time);
            let same_pos = (last_pos.0 == col) && (last_pos.1 == row);
            let same_btn = last_btn == button;
            elapsed <= threshold && same_pos && same_btn
        } else {
            false
        };

        if is_consecutive {
            self.click_count = (self.click_count % 3) + 1;
        } else {
            self.click_count = 1;
        }

        self.last_click_time = Some(now);
        self.last_click_pos = Some((col, row));
        self.last_click_button = Some(button);
        self.last_pos = Some((col, row));

        match self.click_count {
            2 => ClickKind::Double,
            3 => ClickKind::Triple,
            _ => ClickKind::Single,
        }
    }

    /// Records a mouse drag movement.
    pub fn record_drag(&mut self, col: u16, row: u16) -> Option<(u16, u16)> {
        if self.drag_start.is_none() {
            self.drag_start = self.last_pos.or(Some((col, row)));
        }
        self.is_dragging = true;
        self.last_pos = Some((col, row));
        self.drag_start
    }

    /// Records a mouse button release.
    pub fn record_up(&mut self, col: u16, row: u16) {
        self.is_dragging = false;
        self.drag_start = None;
        self.last_pos = Some((col, row));
    }

    /// Records mouse cursor movement without buttons pressed.
    pub fn record_move(&mut self, col: u16, row: u16) {
        self.last_pos = Some((col, row));
    }

    /// Returns current cursor position if known.
    pub fn current_position(&self) -> Option<(u16, u16)> {
        self.last_pos
    }

    /// Returns whether mouse is currently in a drag gesture.
    pub fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    /// Resets all tracked state.
    pub fn reset(&mut self) {
        self.last_click_time = None;
        self.last_click_pos = None;
        self.last_click_button = None;
        self.click_count = 0;
        self.drag_start = None;
        self.last_pos = None;
        self.is_dragging = false;
    }
}

// ---------------------------------------------------------------------------
// List Mouse Handler
// ---------------------------------------------------------------------------

/// High-level mouse event handler for Ratatui list widgets and viewports.
#[derive(Debug, Clone)]
pub struct ListMouseHandler {
    /// Active configuration.
    pub config: MouseConfig,
    /// Internal gesture and click tracker.
    pub tracker: MouseTracker,
}

impl Default for ListMouseHandler {
    fn default() -> Self {
        Self::new(MouseConfig::default())
    }
}

impl ListMouseHandler {
    /// Creates a new handler with the given configuration.
    pub fn new(config: MouseConfig) -> Self {
        let tracker = MouseTracker::new(config.clone());
        Self { config, tracker }
    }

    /// Checks if a (col, row) coordinate falls within a `Rect`.
    pub fn is_inside(col: u16, row: u16, rect: Rect) -> bool {
        col >= rect.x
            && col < rect.x.saturating_add(rect.width)
            && row >= rect.y
            && row < rect.y.saturating_add(rect.height)
    }

    /// Calculates the 0-based absolute item index from a screen row coordinate.
    ///
    /// Returns `None` if the row is outside the list area or beyond `total_items`.
    pub fn calculate_row_index(
        row: u16,
        list_area: Rect,
        scroll_offset: usize,
        total_items: usize,
    ) -> Option<usize> {
        if list_area.height == 0 || total_items == 0 {
            return None;
        }

        if row < list_area.y || row >= list_area.y.saturating_add(list_area.height) {
            return None;
        }

        let relative_row = (row - list_area.y) as usize;
        let item_idx = scroll_offset.saturating_add(relative_row);

        if item_idx < total_items {
            Some(item_idx)
        } else {
            None
        }
    }

    /// Processes a crossterm `MouseEvent` on a list area and returns the resulting `ListMouseAction`.
    pub fn handle_event(
        &mut self,
        event: &MouseEvent,
        list_area: Rect,
        scroll_offset: usize,
        total_items: usize,
        selected_index: usize,
    ) -> ListMouseAction {
        self.handle_event_with_time(
            event,
            list_area,
            scroll_offset,
            total_items,
            selected_index,
            Instant::now(),
        )
    }

    /// Processes a crossterm `MouseEvent` with an explicit timestamp (for deterministic testing).
    pub fn handle_event_with_time(
        &mut self,
        event: &MouseEvent,
        list_area: Rect,
        scroll_offset: usize,
        total_items: usize,
        selected_index: usize,
        now: Instant,
    ) -> ListMouseAction {
        if !self.config.enabled {
            return ListMouseAction::Ignored;
        }

        let col = event.column;
        let row = event.row;
        let inside = Self::is_inside(col, row, list_area);

        match event.kind {
            MouseEventKind::ScrollDown => {
                if self.config.invert_scroll {
                    ListMouseAction::ScrollUp {
                        delta: self.config.scroll_speed,
                    }
                } else {
                    ListMouseAction::ScrollDown {
                        delta: self.config.scroll_speed,
                    }
                }
            }

            MouseEventKind::ScrollUp => {
                if self.config.invert_scroll {
                    ListMouseAction::ScrollDown {
                        delta: self.config.scroll_speed,
                    }
                } else {
                    ListMouseAction::ScrollUp {
                        delta: self.config.scroll_speed,
                    }
                }
            }

            MouseEventKind::Down(MouseButton::Left) => {
                let click_kind = self.tracker.record_click(col, row, MouseButton::Left, now);

                if inside {
                    if let Some(clicked_idx) =
                        Self::calculate_row_index(row, list_area, scroll_offset, total_items)
                    {
                        if click_kind == ClickKind::Double {
                            ListMouseAction::SelectAndSubmit(clicked_idx)
                        } else if self.config.click_selected_to_submit
                            && clicked_idx == selected_index
                        {
                            // Clicking the already active item submits it
                            ListMouseAction::SelectAndSubmit(clicked_idx)
                        } else {
                            ListMouseAction::Select(clicked_idx)
                        }
                    } else {
                        ListMouseAction::Ignored
                    }
                } else if self.config.click_outside_to_cancel {
                    ListMouseAction::Cancel
                } else {
                    ListMouseAction::Ignored
                }
            }

            MouseEventKind::Down(MouseButton::Right) => {
                self.tracker.record_click(col, row, MouseButton::Right, now);
                if self.config.right_click_to_cancel {
                    ListMouseAction::Cancel
                } else {
                    ListMouseAction::Ignored
                }
            }

            MouseEventKind::Down(MouseButton::Middle) => {
                self.tracker
                    .record_click(col, row, MouseButton::Middle, now);
                ListMouseAction::SubmitCurrent
            }

            MouseEventKind::Drag(MouseButton::Left) => {
                self.tracker.record_drag(col, row);
                if inside {
                    if let Some(drag_idx) =
                        Self::calculate_row_index(row, list_area, scroll_offset, total_items)
                    {
                        ListMouseAction::Select(drag_idx)
                    } else {
                        ListMouseAction::Ignored
                    }
                } else {
                    ListMouseAction::Ignored
                }
            }

            MouseEventKind::Up(btn) => {
                self.tracker.record_up(col, row);
                ListMouseAction::Ignored
            }

            MouseEventKind::Moved => {
                self.tracker.record_move(col, row);
                if inside && self.config.hover_selection {
                    if let Some(hover_idx) =
                        Self::calculate_row_index(row, list_area, scroll_offset, total_items)
                    {
                        ListMouseAction::Hover(hover_idx)
                    } else {
                        ListMouseAction::Ignored
                    }
                } else {
                    ListMouseAction::Ignored
                }
            }

            _ => ListMouseAction::Ignored,
        }
    }

    /// Handles a mouse click on a horizontal tab bar area.
    ///
    /// Computes which tab was clicked based on equal width division.
    pub fn handle_tab_bar_equal(
        &mut self,
        event: &MouseEvent,
        tabs_area: Rect,
        tab_count: usize,
    ) -> Option<usize> {
        if !self.config.enabled || tab_count == 0 {
            return None;
        }

        if !Self::is_inside(event.column, event.row, tabs_area) {
            return None;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let tab_width = (tabs_area.width as usize) / tab_count;
                if tab_width == 0 {
                    return None;
                }
                let relative_x = (event.column - tabs_area.x) as usize;
                let clicked_tab = (relative_x / tab_width).min(tab_count - 1);
                Some(clicked_tab)
            }
            _ => None,
        }
    }

    /// Handles a mouse click on a horizontal tab bar with variable label widths and spacing.
    pub fn handle_tab_bar_spans(
        &mut self,
        event: &MouseEvent,
        tabs_area: Rect,
        tab_labels: &[&str],
        padding: u16,
    ) -> Option<usize> {
        if !self.config.enabled || tab_labels.is_empty() {
            return None;
        }

        if !Self::is_inside(event.column, event.row, tabs_area) {
            return None;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let col = event.column;
                let mut current_x = tabs_area.x;

                for (idx, label) in tab_labels.iter().enumerate() {
                    // label with brackets: `[Label]` => len + 2
                    let tab_len = (label.chars().count() as u16) + 2;
                    let tab_end = current_x + tab_len;

                    if col >= current_x && col < tab_end {
                        return Some(idx);
                    }

                    current_x = tab_end + padding;
                }

                None
            }
            _ => None,
        }
    }

    /// Adjusts selection and scroll offset following a scroll delta.
    ///
    /// Returns `(new_selected_index, new_scroll_offset)`.
    pub fn apply_scroll_delta(
        current_selected: usize,
        current_scroll: usize,
        delta: isize,
        total_items: usize,
        visible_rows: usize,
    ) -> (usize, usize) {
        if total_items == 0 {
            return (0, 0);
        }

        let new_selected = if delta < 0 {
            current_selected.saturating_sub(delta.unsigned_abs())
        } else {
            (current_selected + (delta as usize)).min(total_items.saturating_sub(1))
        };

        let new_scroll =
            Self::ensure_selection_visible(new_selected, current_scroll, visible_rows, total_items);

        (new_selected, new_scroll)
    }

    /// Ensures the selected index remains visible within the viewport window.
    pub fn ensure_selection_visible(
        selected: usize,
        current_scroll: usize,
        visible_rows: usize,
        total_items: usize,
    ) -> usize {
        if visible_rows == 0 || total_items == 0 {
            return 0;
        }

        let max_scroll = total_items.saturating_sub(visible_rows);

        if selected < current_scroll {
            selected
        } else if selected >= current_scroll + visible_rows {
            (selected + 1).saturating_sub(visible_rows).min(max_scroll)
        } else {
            current_scroll.min(max_scroll)
        }
    }
}

// ---------------------------------------------------------------------------
// Tab Bar Mouse Handler
// ---------------------------------------------------------------------------

/// Dedicated helper for clickable tab navigation bars.
pub struct TabBarMouseHandler;

impl TabBarMouseHandler {
    /// Computes which tab index was clicked from absolute screen coordinates.
    pub fn find_clicked_tab(
        col: u16,
        row: u16,
        tabs_area: Rect,
        tab_labels: &[&str],
        gap: u16,
        left_pad: u16,
    ) -> Option<usize> {
        if row != tabs_area.y {
            return None;
        }

        let mut curr_x = tabs_area.x.saturating_add(left_pad);

        for (idx, label) in tab_labels.iter().enumerate() {
            // e.g. `[Anthropic]` or `[All]` => characters count + 2 brackets
            let width = (label.chars().count() as u16).saturating_add(2);
            let end_x = curr_x.saturating_add(width);

            if col >= curr_x && col < end_x {
                return Some(idx);
            }

            curr_x = end_x.saturating_add(gap);
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Scroll Controller
// ---------------------------------------------------------------------------

/// Controller for computing scroll positions, viewport windows, and scrollbar geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollController {
    /// Total number of items in the underlying collection.
    pub total_items: usize,
    /// Number of visible rows in the viewport.
    pub visible_rows: usize,
    /// Current 0-based scroll offset.
    pub scroll_offset: usize,
    /// Current selected item index.
    pub selected_index: usize,
}

impl ScrollController {
    /// Creates a new scroll controller.
    pub fn new(total_items: usize, visible_rows: usize) -> Self {
        Self {
            total_items,
            visible_rows,
            scroll_offset: 0,
            selected_index: 0,
        }
    }

    /// Scrolls up by `delta` items and clamps selection.
    pub fn scroll_up(&mut self, delta: usize) {
        self.selected_index = self.selected_index.saturating_sub(delta);
        self.update_scroll();
    }

    /// Scrolls down by `delta` items and clamps selection.
    pub fn scroll_down(&mut self, delta: usize) {
        if self.total_items > 0 {
            self.selected_index = (self.selected_index + delta).min(self.total_items - 1);
            self.update_scroll();
        }
    }

    /// Selects a specific absolute index and updates scroll offset.
    pub fn select(&mut self, index: usize) {
        if self.total_items > 0 {
            self.selected_index = index.min(self.total_items - 1);
            self.update_scroll();
        }
    }

    /// Updates `scroll_offset` to keep `selected_index` in the visible window.
    pub fn update_scroll(&mut self) {
        self.scroll_offset = ListMouseHandler::ensure_selection_visible(
            self.selected_index,
            self.scroll_offset,
            self.visible_rows,
            self.total_items,
        );
    }

    /// Computes scrollbar thumb position `(thumb_y_start, thumb_height)` for a given track height.
    pub fn scrollbar_thumb(&self, track_height: u16) -> (u16, u16) {
        if track_height == 0 || self.total_items == 0 || self.total_items <= self.visible_rows {
            return (0, track_height);
        }

        let visible_fraction = (self.visible_rows as f32) / (self.total_items as f32);
        let thumb_height = ((track_height as f32) * visible_fraction)
            .round()
            .clamp(1.0, track_height as f32) as u16;

        let max_scroll = self.total_items.saturating_sub(self.visible_rows);
        let scroll_fraction = if max_scroll > 0 {
            (self.scroll_offset as f32) / (max_scroll as f32)
        } else {
            0.0
        };

        let max_thumb_y = track_height.saturating_sub(thumb_height);
        let thumb_y = ((max_thumb_y as f32) * scroll_fraction).round() as u16;

        (thumb_y, thumb_height)
    }

    /// Computes target scroll offset from a click position on the scrollbar track.
    pub fn scroll_from_track_click(
        click_y: u16,
        track_height: u16,
        total_items: usize,
        visible_rows: usize,
    ) -> usize {
        if track_height == 0 || total_items <= visible_rows {
            return 0;
        }

        let max_scroll = total_items.saturating_sub(visible_rows);
        let fraction = (click_y as f32) / (track_height as f32);
        ((max_scroll as f32) * fraction).round() as usize
    }
}

// ---------------------------------------------------------------------------
// Region Layout & Interaction Classifier
// ---------------------------------------------------------------------------

/// Categorized sub-region within an interactive inline prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRegion {
    /// Search input bar.
    SearchInput,
    /// Provider / Category tab bar with clicked tab index.
    TabBar(usize),
    /// List viewport with clicked item index.
    ListItem(usize),
    /// List viewport blank/empty row.
    ListEmpty,
    /// Footer key hint bar.
    Footer,
    /// Clicked outside interactive bounds.
    Outside,
}

/// Helper for classifying a screen coordinate against multi-section inline UI components.
pub struct RegionClassifier;

impl RegionClassifier {
    /// Classifies a (col, row) coordinate against search, tabs, list, and footer areas.
    pub fn classify(
        col: u16,
        row: u16,
        search_rect: Option<Rect>,
        tabs_rect: Option<Rect>,
        tab_labels: Option<&[&str]>,
        list_rect: Rect,
        scroll_offset: usize,
        total_items: usize,
        footer_rect: Option<Rect>,
    ) -> PromptRegion {
        if let Some(sr) = search_rect {
            if ListMouseHandler::is_inside(col, row, sr) {
                return PromptRegion::SearchInput;
            }
        }

        if let Some(tr) = tabs_rect {
            if ListMouseHandler::is_inside(col, row, tr) {
                if let Some(labels) = tab_labels {
                    if let Some(tab_idx) =
                        TabBarMouseHandler::find_clicked_tab(col, row, tr, labels, 2, 1)
                    {
                        return PromptRegion::TabBar(tab_idx);
                    }
                }
                return PromptRegion::TabBar(0);
            }
        }

        if ListMouseHandler::is_inside(col, row, list_rect) {
            if let Some(item_idx) =
                ListMouseHandler::calculate_row_index(row, list_rect, scroll_offset, total_items)
            {
                return PromptRegion::ListItem(item_idx);
            } else {
                return PromptRegion::ListEmpty;
            }
        }

        if let Some(fr) = footer_rect {
            if ListMouseHandler::is_inside(col, row, fr) {
                return PromptRegion::Footer;
            }
        }

        PromptRegion::Outside
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mouse_config_builder() {
        let config = MouseConfig::new()
            .with_enabled(true)
            .with_scroll_speed(5)
            .with_double_click_threshold_ms(300)
            .with_click_selected_to_submit(false)
            .with_right_click_to_cancel(true)
            .with_invert_scroll(true);

        assert!(config.enabled);
        assert_eq!(config.scroll_speed, 5);
        assert_eq!(config.double_click_threshold_ms, 300);
        assert!(!config.click_selected_to_submit);
        assert!(config.right_click_to_cancel);
        assert!(config.invert_scroll);
    }

    #[test]
    fn test_is_inside() {
        let rect = Rect::new(10, 5, 20, 10);

        assert!(ListMouseHandler::is_inside(10, 5, rect));
        assert!(ListMouseHandler::is_inside(29, 14, rect));
        assert!(ListMouseHandler::is_inside(15, 8, rect));

        assert!(!ListMouseHandler::is_inside(9, 5, rect));
        assert!(!ListMouseHandler::is_inside(10, 4, rect));
        assert!(!ListMouseHandler::is_inside(30, 10, rect));
        assert!(!ListMouseHandler::is_inside(20, 15, rect));
    }

    #[test]
    fn test_calculate_row_index() {
        let list_area = Rect::new(0, 2, 80, 5);

        // Within bounds
        assert_eq!(
            ListMouseHandler::calculate_row_index(2, list_area, 0, 10),
            Some(0)
        );
        assert_eq!(
            ListMouseHandler::calculate_row_index(4, list_area, 0, 10),
            Some(2)
        );
        assert_eq!(
            ListMouseHandler::calculate_row_index(6, list_area, 0, 10),
            Some(4)
        );

        // With scroll offset
        assert_eq!(
            ListMouseHandler::calculate_row_index(2, list_area, 5, 10),
            Some(5)
        );
        assert_eq!(
            ListMouseHandler::calculate_row_index(6, list_area, 5, 10),
            Some(9)
        );

        // Outside item count
        assert_eq!(
            ListMouseHandler::calculate_row_index(5, list_area, 0, 3),
            None
        );

        // Outside area y
        assert_eq!(
            ListMouseHandler::calculate_row_index(1, list_area, 0, 10),
            None
        );
        assert_eq!(
            ListMouseHandler::calculate_row_index(7, list_area, 0, 10),
            None
        );
    }

    #[test]
    fn test_mouse_tracker_double_click() {
        let config = MouseConfig::default().with_double_click_threshold_ms(300);
        let mut tracker = MouseTracker::new(config);

        let t0 = Instant::now();
        let k1 = tracker.record_click(10, 5, MouseButton::Left, t0);
        assert_eq!(k1, ClickKind::Single);

        // Fast click at same position -> Double
        let t1 = t0 + Duration::from_millis(100);
        let k2 = tracker.record_click(10, 5, MouseButton::Left, t1);
        assert_eq!(k2, ClickKind::Double);

        // Third fast click -> Triple
        let t2 = t1 + Duration::from_millis(100);
        let k3 = tracker.record_click(10, 5, MouseButton::Left, t2);
        assert_eq!(k3, ClickKind::Triple);

        // Delayed click -> Single again
        let t3 = t2 + Duration::from_millis(500);
        let k4 = tracker.record_click(10, 5, MouseButton::Left, t3);
        assert_eq!(k4, ClickKind::Single);
    }

    #[test]
    fn test_list_mouse_handler_click_and_scroll() {
        let mut handler = ListMouseHandler::default();
        let list_area = Rect::new(0, 1, 40, 5);
        let t0 = Instant::now();

        // 1. Click on item 2 (row 3) when current selection is 0 -> Select(2)
        let ev_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::empty(),
        };
        let action = handler.handle_event_with_time(&ev_click, list_area, 0, 10, 0, t0);
        assert_eq!(action, ListMouseAction::Select(2));

        // 2. Click again on item 2 (row 3) when selection is now 2 -> SelectAndSubmit(2)
        let t1 = t0 + Duration::from_millis(50);
        let action2 = handler.handle_event_with_time(&ev_click, list_area, 0, 10, 2, t1);
        assert_eq!(action2, ListMouseAction::SelectAndSubmit(2));

        // 3. Scroll Down
        let ev_scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 3,
            modifiers: KeyModifiers::empty(),
        };
        let action3 = handler.handle_event_with_time(&ev_scroll_down, list_area, 0, 10, 2, t1);
        assert_eq!(
            action3,
            ListMouseAction::ScrollDown {
                delta: handler.config.scroll_speed
            }
        );

        // 4. Scroll Up
        let ev_scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 3,
            modifiers: KeyModifiers::empty(),
        };
        let action4 = handler.handle_event_with_time(&ev_scroll_up, list_area, 0, 10, 2, t1);
        assert_eq!(
            action4,
            ListMouseAction::ScrollUp {
                delta: handler.config.scroll_speed
            }
        );

        // 5. Right Click -> Cancel
        let ev_right = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::empty(),
        };
        let action5 = handler.handle_event_with_time(&ev_right, list_area, 0, 10, 2, t1);
        assert_eq!(action5, ListMouseAction::Cancel);
    }

    #[test]
    fn test_tab_bar_find_clicked_tab() {
        let tabs_area = Rect::new(0, 0, 80, 1);
        let labels = ["All", "Anthropic", "OpenAI", "DeepSeek"];

        // " [All]  [Anthropic]  [OpenAI]  [DeepSeek]"
        // Index 0: ` [All]` -> starts at 1, len=5 -> 1..6
        // Gap = 2 -> next starts at 8
        // Index 1: `[Anthropic]` -> len=11 -> 8..19
        // Gap = 2 -> next starts at 21
        // Index 2: `[OpenAI]` -> len=8 -> 21..29
        // Gap = 2 -> next starts at 31
        // Index 3: `[DeepSeek]` -> len=10 -> 31..41

        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(2, 0, tabs_area, &labels, 2, 1),
            Some(0)
        );
        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(10, 0, tabs_area, &labels, 2, 1),
            Some(1)
        );
        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(24, 0, tabs_area, &labels, 2, 1),
            Some(2)
        );
        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(35, 0, tabs_area, &labels, 2, 1),
            Some(3)
        );

        // Click in gap
        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(7, 0, tabs_area, &labels, 2, 1),
            None
        );
        // Wrong row
        assert_eq!(
            TabBarMouseHandler::find_clicked_tab(2, 1, tabs_area, &labels, 2, 1),
            None
        );
    }

    #[test]
    fn test_scroll_controller() {
        let mut controller = ScrollController::new(20, 5);
        assert_eq!(controller.selected_index, 0);
        assert_eq!(controller.scroll_offset, 0);

        controller.scroll_down(3);
        assert_eq!(controller.selected_index, 3);
        assert_eq!(controller.scroll_offset, 0);

        controller.scroll_down(3);
        assert_eq!(controller.selected_index, 6);
        assert_eq!(controller.scroll_offset, 2); // 6 + 1 - 5 = 2

        controller.scroll_up(4);
        assert_eq!(controller.selected_index, 2);
        assert_eq!(controller.scroll_offset, 2); // selected 2 is visible with scroll 2

        controller.scroll_up(2);
        assert_eq!(controller.selected_index, 0);
        assert_eq!(controller.scroll_offset, 0);
    }

    #[test]
    fn test_scrollbar_thumb() {
        let mut controller = ScrollController::new(100, 10);
        let track_height = 10;

        let (y, height) = controller.scrollbar_thumb(track_height);
        assert_eq!(y, 0);
        assert_eq!(height, 1);

        controller.select(99);
        let (y2, height2) = controller.scrollbar_thumb(track_height);
        assert_eq!(y2, 9);
        assert_eq!(height2, 1);
    }

    #[test]
    fn test_region_classifier() {
        let search = Some(Rect::new(0, 0, 80, 1));
        let tabs = Some(Rect::new(0, 1, 80, 1));
        let list = Rect::new(0, 2, 80, 5);
        let footer = Some(Rect::new(0, 7, 80, 1));
        let tab_labels = ["All", "Anthropic"];

        let reg_search =
            RegionClassifier::classify(5, 0, search, tabs, Some(&tab_labels), list, 0, 10, footer);
        assert_eq!(reg_search, PromptRegion::SearchInput);

        let reg_tab =
            RegionClassifier::classify(2, 1, search, tabs, Some(&tab_labels), list, 0, 10, footer);
        assert_eq!(reg_tab, PromptRegion::TabBar(0));

        let reg_item =
            RegionClassifier::classify(5, 3, search, tabs, Some(&tab_labels), list, 0, 10, footer);
        assert_eq!(reg_item, PromptRegion::ListItem(1));

        let reg_foot =
            RegionClassifier::classify(5, 7, search, tabs, Some(&tab_labels), list, 0, 10, footer);
        assert_eq!(reg_foot, PromptRegion::Footer);

        let reg_out =
            RegionClassifier::classify(5, 9, search, tabs, Some(&tab_labels), list, 0, 10, footer);
        assert_eq!(reg_out, PromptRegion::Outside);
    }
}
