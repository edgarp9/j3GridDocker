use windows_sys::Win32::Foundation::RECT;

use crate::domain::Rect;

use super::{
    COMMAND_BAR_HEIGHT, STATUS_BAR_HEIGHT, TAB_BAR_HEIGHT, TAB_BAR_LEFT, TOOLBAR_TOGGLE_LEFT,
    TOOLBAR_TOGGLE_WIDTH, TOP_BAR_NEW_TAB_LEFT, TOP_BAR_NEW_TAB_WIDTH,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct ClientPoint {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ScreenPoint {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiRect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

impl UiRect {
    pub(super) const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub(super) const fn from_rect(rect: RECT) -> Self {
        Self::new(rect.left, rect.top, rect.right, rect.bottom)
    }

    pub(super) fn contains(self, point: ClientPoint) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    pub(super) const fn width(self) -> i32 {
        self.right - self.left
    }

    pub(super) const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub(super) const fn is_empty(self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub(super) fn intersect(self, other: Self) -> Option<Self> {
        let rect = Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!rect.is_empty()).then_some(rect)
    }

    pub(super) fn union(self, other: Self) -> Self {
        Self::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    pub(super) const fn inset(self, horizontal: i32, vertical: i32) -> Self {
        Self::new(
            self.left + horizontal,
            self.top + vertical,
            self.right - horizontal,
            self.bottom - vertical,
        )
    }

    pub(super) const fn to_rect(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LayoutMetrics {
    pub(super) content_top: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

pub(super) fn top_bar_height(workspace_ui_visible: bool) -> i32 {
    if workspace_ui_visible {
        TAB_BAR_HEIGHT + COMMAND_BAR_HEIGHT
    } else {
        TAB_BAR_HEIGHT
    }
}

pub(super) fn toolbar_toggle_rect() -> UiRect {
    UiRect::new(
        TOOLBAR_TOGGLE_LEFT,
        4,
        TOOLBAR_TOGGLE_LEFT + TOOLBAR_TOGGLE_WIDTH,
        TAB_BAR_HEIGHT - 2,
    )
}

pub(super) fn new_tab_button_rect() -> UiRect {
    UiRect::new(
        TOP_BAR_NEW_TAB_LEFT,
        4,
        TOP_BAR_NEW_TAB_LEFT + TOP_BAR_NEW_TAB_WIDTH,
        TAB_BAR_HEIGHT - 2,
    )
}

pub(super) const TAB_WIDTH: i32 = 132;
pub(super) const TAB_GAP: i32 = 4;

const TAB_TOP: i32 = 4;
const TAB_BOTTOM_INSET: i32 = 2;
const TAB_CLOSE_BUTTON_SIZE: i32 = 14;
const TAB_CLOSE_BUTTON_RIGHT_PADDING: i32 = 8;
const TAB_CLOSE_BUTTON_LABEL_GAP: i32 = 4;
const TAB_CLOSE_BUTTON_MIN_BODY_WIDTH: i32 = 24;
const TAB_LABEL_LEFT_INSET: i32 = 8;
const TAB_LABEL_VERTICAL_INSET: i32 = 2;
const TAB_OVERFLOW_DROPDOWN_WIDTH: i32 = 28;
const TAB_OVERFLOW_DROPDOWN_GAP: i32 = 4;
const TAB_REORDER_SCROLL_ZONE: i32 = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabHitTarget {
    Body,
    CloseButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabOverflowHitTarget {
    Dropdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabOverflowDropdown {
    pub(super) rect: UiRect,
    pub(super) hidden_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabStripLayout {
    pub(super) viewport: UiRect,
    pub(super) first_visible_index: usize,
    pub(super) visible_count: usize,
    pub(super) tab_count: usize,
    pub(super) dropdown: Option<TabOverflowDropdown>,
}

impl TabStripLayout {
    pub(super) fn visible_end_index(self) -> usize {
        self.first_visible_index
            .saturating_add(self.visible_count)
            .min(self.tab_count)
    }

    pub(super) fn is_index_visible(self, index: usize) -> bool {
        index >= self.first_visible_index && index < self.visible_end_index()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabStripHit {
    pub(super) index: usize,
    pub(super) target: TabHitTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabInsertionTarget {
    pub(super) before_index: usize,
    pub(super) x: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabReorderAutoScroll {
    Backward,
    Forward,
}

pub(super) fn tab_strip_layout(
    client: UiRect,
    tab_count: usize,
    active_index: Option<usize>,
    requested_first_visible_index: usize,
) -> TabStripLayout {
    let client_right = client.width().max(0);
    let raw_tab_space = client_right.saturating_sub(TAB_BAR_LEFT);
    let has_overflow = tabs_total_width(tab_count) > raw_tab_space;
    let dropdown = if has_overflow && raw_tab_space > 0 {
        let right = client_right;
        let left = right
            .saturating_sub(TAB_OVERFLOW_DROPDOWN_WIDTH)
            .max(TAB_BAR_LEFT);
        Some(TabOverflowDropdown {
            rect: UiRect::new(left, TAB_TOP, right, TAB_BAR_HEIGHT - TAB_BOTTOM_INSET),
            hidden_count: 0,
        })
    } else {
        None
    };
    let viewport_right = dropdown
        .map(|dropdown| {
            dropdown
                .rect
                .left
                .saturating_sub(TAB_OVERFLOW_DROPDOWN_GAP)
                .max(TAB_BAR_LEFT)
        })
        .unwrap_or(client_right)
        .max(TAB_BAR_LEFT);
    let viewport = UiRect::new(TAB_BAR_LEFT, 0, viewport_right, TAB_BAR_HEIGHT);
    let visible_count = visible_tab_capacity(viewport.width(), tab_count);
    let first_visible_index = ensure_active_tab_visible(
        tab_count,
        visible_count,
        requested_first_visible_index,
        active_index,
    );
    let hidden_count = tab_count.saturating_sub(visible_count);
    let dropdown = dropdown.map(|dropdown| TabOverflowDropdown {
        hidden_count,
        ..dropdown
    });

    TabStripLayout {
        viewport,
        first_visible_index,
        visible_count,
        tab_count,
        dropdown,
    }
}

pub(super) fn tab_rect_for_index(layout: TabStripLayout, index: usize) -> Option<UiRect> {
    if !layout.is_index_visible(index) {
        return None;
    }

    let relative = index.checked_sub(layout.first_visible_index)?;
    let relative = i32::try_from(relative).ok()?;
    let step = TAB_WIDTH.checked_add(TAB_GAP)?;
    let left = layout
        .viewport
        .left
        .checked_add(relative.checked_mul(step)?)?;
    let right = left.checked_add(TAB_WIDTH)?;

    if right > layout.viewport.right || right <= left {
        return None;
    }

    Some(UiRect::new(
        left,
        TAB_TOP,
        right,
        TAB_BAR_HEIGHT - TAB_BOTTOM_INSET,
    ))
}

pub(super) fn hit_test_tab_strip(
    layout: TabStripLayout,
    point: ClientPoint,
) -> Option<TabStripHit> {
    for index in layout.first_visible_index..layout.visible_end_index() {
        let Some(rect) = tab_rect_for_index(layout, index) else {
            continue;
        };
        if let Some(target) = hit_test_tab(rect, point) {
            return Some(TabStripHit { index, target });
        }
    }

    None
}

pub(super) fn tab_insertion_target(
    layout: TabStripLayout,
    point: ClientPoint,
) -> Option<TabInsertionTarget> {
    if point.y < 0 || point.y >= TAB_BAR_HEIGHT {
        return None;
    }

    let visible_start = layout.first_visible_index;
    let visible_end = layout.visible_end_index();
    if visible_start >= visible_end {
        return None;
    }

    for index in visible_start..visible_end {
        let Some(rect) = tab_rect_for_index(layout, index) else {
            continue;
        };
        let midpoint = rect.left + rect.width() / 2;
        if point.x < midpoint {
            return Some(TabInsertionTarget {
                before_index: index,
                x: rect.left,
            });
        }
    }

    let last_index = visible_end - 1;
    let rect = tab_rect_for_index(layout, last_index)?;
    Some(TabInsertionTarget {
        before_index: visible_end.min(layout.tab_count),
        x: rect.right,
    })
}

pub(super) fn tab_reorder_auto_scroll(
    layout: TabStripLayout,
    point: ClientPoint,
) -> Option<TabReorderAutoScroll> {
    if point.y < 0
        || point.y >= TAB_BAR_HEIGHT
        || layout.dropdown.is_none()
        || layout.visible_count == 0
    {
        return None;
    }

    let visible_end = layout.visible_end_index();
    if layout.first_visible_index > 0 && point.x <= layout.viewport.left + TAB_REORDER_SCROLL_ZONE {
        return Some(TabReorderAutoScroll::Backward);
    }

    if visible_end < layout.tab_count && point.x >= layout.viewport.right - TAB_REORDER_SCROLL_ZONE
    {
        return Some(TabReorderAutoScroll::Forward);
    }

    None
}

pub(super) fn hit_test_tab_overflow(
    layout: TabStripLayout,
    point: ClientPoint,
) -> Option<TabOverflowHitTarget> {
    layout
        .dropdown
        .filter(|dropdown| dropdown.rect.contains(point))
        .map(|_| TabOverflowHitTarget::Dropdown)
}

pub(super) fn hit_test_tab_strip_empty(layout: TabStripLayout, point: ClientPoint) -> bool {
    layout.viewport.contains(point)
        && hit_test_tab_strip(layout, point).is_none()
        && hit_test_tab_overflow(layout, point).is_none()
}

pub(super) fn tab_close_button_rect(tab_rect: UiRect) -> Option<UiRect> {
    if tab_rect.width()
        < TAB_CLOSE_BUTTON_MIN_BODY_WIDTH + TAB_CLOSE_BUTTON_SIZE + TAB_CLOSE_BUTTON_RIGHT_PADDING
    {
        return None;
    }

    let right = tab_rect.right - TAB_CLOSE_BUTTON_RIGHT_PADDING;
    let top = tab_rect.top + (tab_rect.height() - TAB_CLOSE_BUTTON_SIZE) / 2;

    Some(UiRect::new(
        right - TAB_CLOSE_BUTTON_SIZE,
        top,
        right,
        top + TAB_CLOSE_BUTTON_SIZE,
    ))
}

pub(super) fn tab_label_rect(tab_rect: UiRect) -> UiRect {
    let left = tab_rect.left + TAB_LABEL_LEFT_INSET;
    let right = tab_close_button_rect(tab_rect)
        .map(|rect| rect.left - TAB_CLOSE_BUTTON_LABEL_GAP)
        .unwrap_or(tab_rect.right - TAB_LABEL_LEFT_INSET)
        .max(left);

    UiRect::new(
        left,
        tab_rect.top + TAB_LABEL_VERTICAL_INSET,
        right,
        tab_rect.bottom - TAB_LABEL_VERTICAL_INSET,
    )
}

pub(super) fn hit_test_tab(tab_rect: UiRect, point: ClientPoint) -> Option<TabHitTarget> {
    if !tab_rect.contains(point) {
        return None;
    }

    if tab_close_button_rect(tab_rect).is_some_and(|rect| rect.contains(point)) {
        Some(TabHitTarget::CloseButton)
    } else {
        Some(TabHitTarget::Body)
    }
}

pub(super) fn layout_metrics(client: UiRect, workspace_ui_visible: bool) -> Option<LayoutMetrics> {
    let content_top = top_bar_height(workspace_ui_visible);
    let content_bottom = if workspace_ui_visible {
        client.bottom.checked_sub(STATUS_BAR_HEIGHT)?
    } else {
        client.bottom
    };
    let width = client.width();
    let height = content_bottom.checked_sub(content_top)?;

    if width <= 0 || height <= 0 {
        return None;
    }

    Some(LayoutMetrics {
        content_top,
        width,
        height,
    })
}

pub(super) fn layout_bounds_for_client_rect(
    client: UiRect,
    workspace_ui_visible: bool,
) -> Option<Rect> {
    let metrics = layout_metrics(client, workspace_ui_visible)?;
    Rect::new(0, 0, metrics.width, metrics.height).ok()
}

pub(super) fn layout_rect_to_client_rect(
    content_top: i32,
    bounds: Rect,
    rect: Rect,
) -> Option<UiRect> {
    let left = rect.left().checked_sub(bounds.left())?;
    let local_top = rect.top().checked_sub(bounds.top())?;
    let top = local_top.checked_add(content_top)?;
    let right = left.checked_add(rect.width())?;
    let bottom = top.checked_add(rect.height())?;

    Some(UiRect::new(left, top, right, bottom))
}

fn tabs_total_width(tab_count: usize) -> i32 {
    if tab_count == 0 {
        return 0;
    }

    let tab_count = match i64::try_from(tab_count) {
        Ok(value) => value,
        Err(_) => return i32::MAX,
    };
    let width = tab_count
        .saturating_mul(i64::from(TAB_WIDTH))
        .saturating_add(
            tab_count
                .saturating_sub(1)
                .saturating_mul(i64::from(TAB_GAP)),
        );

    if width > i64::from(i32::MAX) {
        i32::MAX
    } else {
        width as i32
    }
}

fn visible_tab_capacity(viewport_width: i32, tab_count: usize) -> usize {
    if viewport_width < TAB_WIDTH || tab_count == 0 {
        return 0;
    }

    let count = viewport_width.saturating_add(TAB_GAP) / (TAB_WIDTH + TAB_GAP);
    match usize::try_from(count) {
        Ok(value) => value.min(tab_count),
        Err(_) => tab_count,
    }
}

fn ensure_active_tab_visible(
    tab_count: usize,
    visible_count: usize,
    requested_first_visible_index: usize,
    active_index: Option<usize>,
) -> usize {
    if tab_count == 0 || visible_count == 0 || visible_count >= tab_count {
        return 0;
    }

    let max_first = tab_count - visible_count;
    let mut first = requested_first_visible_index.min(max_first);

    if let Some(active_index) = active_index.filter(|index| *index < tab_count) {
        if active_index < first {
            first = active_index;
        } else if active_index >= first + visible_count {
            first = active_index + 1 - visible_count;
        }
    }

    first.min(max_first)
}

#[cfg(test)]
mod tests {
    use super::super::{TAB_BAR_LEFT, TOOLBAR_TOGGLE_GAP, TOP_BAR_NEW_TAB_WIDTH};
    use super::*;

    fn visible_tab_rects(layout: TabStripLayout) -> Vec<(usize, UiRect)> {
        (layout.first_visible_index..layout.visible_end_index())
            .filter_map(|index| tab_rect_for_index(layout, index).map(|rect| (index, rect)))
            .collect()
    }

    fn overflow_client_for_viewport_width(viewport_width: i32) -> UiRect {
        UiRect::new(
            0,
            0,
            TAB_BAR_LEFT + viewport_width + TAB_OVERFLOW_DROPDOWN_GAP + TAB_OVERFLOW_DROPDOWN_WIDTH,
            400,
        )
    }

    #[test]
    fn workspace_ui_visibility_controls_top_bar_height() {
        assert_eq!(top_bar_height(true), TAB_BAR_HEIGHT + COMMAND_BAR_HEIGHT);
        assert_eq!(top_bar_height(false), TAB_BAR_HEIGHT);
    }

    #[test]
    fn layout_bounds_for_paint_are_client_local() {
        let client = UiRect::new(0, 0, 900, 650);
        let bounds = layout_bounds_for_client_rect(client, true);

        assert_eq!(
            bounds,
            Rect::new(
                0,
                0,
                900,
                650 - TAB_BAR_HEIGHT - COMMAND_BAR_HEIGHT - STATUS_BAR_HEIGHT
            )
            .ok()
        );
    }

    #[test]
    fn ui_rect_intersection_discards_empty_overlap() {
        let rect = UiRect::new(10, 20, 60, 80);

        assert_eq!(
            rect.intersect(UiRect::new(40, 0, 90, 40)),
            Some(UiRect::new(40, 20, 60, 40))
        );
        assert_eq!(rect.intersect(UiRect::new(60, 0, 90, 40)), None);
    }

    #[test]
    fn ui_rect_union_covers_both_rects() {
        let first = UiRect::new(10, 20, 30, 40);
        let second = UiRect::new(0, 25, 50, 35);

        assert_eq!(first.union(second), UiRect::new(0, 20, 50, 40));
    }

    #[test]
    fn hidden_workspace_ui_extends_layout_to_client_bottom() {
        let client = UiRect::new(0, 0, 900, 650);
        let bounds = layout_bounds_for_client_rect(client, false);

        assert_eq!(bounds, Rect::new(0, 0, 900, 650 - TAB_BAR_HEIGHT).ok());
    }

    #[test]
    fn layout_metrics_rejects_non_positive_content_area() {
        let client = UiRect::new(
            0,
            0,
            900,
            TAB_BAR_HEIGHT + COMMAND_BAR_HEIGHT + STATUS_BAR_HEIGHT,
        );

        assert_eq!(layout_metrics(client, true), None);
    }

    #[test]
    fn client_layout_rect_mapping_is_stable_across_screen_moves()
    -> Result<(), crate::domain::DomainError> {
        let local_bounds = Rect::new(0, 0, 900, 556)?;
        let moved_screen_bounds = Rect::new(240, 180, 900, 556)?;
        let local_rect = Rect::new(450, 0, 450, 556)?;
        let moved_screen_rect = Rect::new(690, 180, 450, 556)?;

        assert_eq!(
            layout_rect_to_client_rect(
                TAB_BAR_HEIGHT + COMMAND_BAR_HEIGHT,
                local_bounds,
                local_rect
            ),
            layout_rect_to_client_rect(
                TAB_BAR_HEIGHT + COMMAND_BAR_HEIGHT,
                moved_screen_bounds,
                moved_screen_rect
            )
        );

        Ok(())
    }

    #[test]
    fn top_bar_buttons_are_fixed_left_of_tabs() {
        let toggle = toolbar_toggle_rect();
        let new_tab = new_tab_button_rect();

        assert_eq!(toggle.left, TOOLBAR_TOGGLE_LEFT);
        assert_eq!(toggle.right, TOOLBAR_TOGGLE_LEFT + TOOLBAR_TOGGLE_WIDTH);
        assert_eq!(new_tab.left, toggle.right + TOOLBAR_TOGGLE_GAP);
        assert_eq!(new_tab.width(), TOP_BAR_NEW_TAB_WIDTH);
        assert_eq!(new_tab.right + TOOLBAR_TOGGLE_GAP, TAB_BAR_LEFT);
        assert_eq!(toggle.top, new_tab.top);
        assert_eq!(toggle.bottom, new_tab.bottom);
        assert!(toggle.top < toggle.bottom);
    }

    #[test]
    fn tab_close_button_rect_is_inside_tab_right_edge() {
        let tab = UiRect::new(TAB_BAR_LEFT, 4, TAB_BAR_LEFT + 132, TAB_BAR_HEIGHT - 2);
        let close = tab_close_button_rect(tab);

        assert_eq!(
            close,
            Some(UiRect::new(TAB_BAR_LEFT + 110, 11, TAB_BAR_LEFT + 124, 25))
        );
        assert!(tab.contains(ClientPoint {
            x: TAB_BAR_LEFT + 110,
            y: 11
        }));
    }

    #[test]
    fn tab_label_rect_leaves_room_for_close_button() {
        let tab = UiRect::new(TAB_BAR_LEFT, 4, TAB_BAR_LEFT + 132, TAB_BAR_HEIGHT - 2);
        let label = tab_label_rect(tab);
        let close = match tab_close_button_rect(tab) {
            Some(rect) => rect,
            None => panic!("test tab is wide enough for a close button"),
        };

        assert_eq!(label.left, TAB_BAR_LEFT + 8);
        assert!(label.right <= close.left - 4);
        assert!(label.top > tab.top);
        assert!(label.bottom < tab.bottom);
    }

    #[test]
    fn tab_hit_test_separates_body_and_close_button() {
        let tab = UiRect::new(TAB_BAR_LEFT, 4, TAB_BAR_LEFT + 132, TAB_BAR_HEIGHT - 2);
        let close = match tab_close_button_rect(tab) {
            Some(rect) => rect,
            None => panic!("test tab is wide enough for a close button"),
        };

        assert_eq!(
            hit_test_tab(
                tab,
                ClientPoint {
                    x: close.left + 1,
                    y: close.top + 1
                }
            ),
            Some(TabHitTarget::CloseButton)
        );
        assert_eq!(
            hit_test_tab(
                tab,
                ClientPoint {
                    x: close.left - 1,
                    y: close.top + 1
                }
            ),
            Some(TabHitTarget::Body)
        );
        assert_eq!(
            hit_test_tab(
                tab,
                ClientPoint {
                    x: tab.right,
                    y: close.top
                }
            ),
            None
        );
    }

    #[test]
    fn tab_strip_layout_uses_simple_tabs_when_they_fit() {
        let client = UiRect::new(0, 0, TAB_BAR_LEFT + TAB_WIDTH * 2 + TAB_GAP, 400);
        let layout = tab_strip_layout(client, 2, Some(1), 1);

        assert_eq!(layout.first_visible_index, 0);
        assert_eq!(layout.visible_count, 2);
        assert!(layout.dropdown.is_none());
        assert_eq!(
            tab_rect_for_index(layout, 1),
            Some(UiRect::new(
                TAB_BAR_LEFT + TAB_WIDTH + TAB_GAP,
                4,
                TAB_BAR_LEFT + TAB_WIDTH * 2 + TAB_GAP,
                TAB_BAR_HEIGHT - 2
            ))
        );
    }

    #[test]
    fn tab_strip_layout_keeps_active_tab_visible_in_overflow() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);

        assert!(layout.dropdown.is_some());
        assert_eq!(layout.visible_count, 3);
        assert_eq!(layout.first_visible_index, 3);
        assert!(layout.is_index_visible(5));
        assert!(!layout.is_index_visible(2));

        let dropdown = match layout.dropdown {
            Some(dropdown) => dropdown,
            None => panic!("overflow layout should expose a dropdown button"),
        };
        assert_eq!(dropdown.hidden_count, 3);
        assert!(dropdown.rect.right <= client.width());
    }

    #[test]
    fn tab_strip_layout_recalculates_visible_range_after_resize() {
        let narrow = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let narrow_layout = tab_strip_layout(narrow, 6, Some(5), 0);
        let wide = UiRect::new(0, 0, 1200, 400);
        let wide_layout = tab_strip_layout(wide, 6, Some(5), narrow_layout.first_visible_index);

        assert_eq!(narrow_layout.first_visible_index, 3);
        assert_eq!(wide_layout.first_visible_index, 0);
        assert_eq!(wide_layout.visible_count, 6);
        assert!(wide_layout.dropdown.is_none());
    }

    #[test]
    fn tab_strip_hit_test_uses_visible_offset_and_close_button() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);
        let tab = match tab_rect_for_index(layout, 4) {
            Some(rect) => rect,
            None => panic!("tab index 4 should be visible"),
        };
        let close = match tab_close_button_rect(tab) {
            Some(rect) => rect,
            None => panic!("visible tab should be wide enough for close button"),
        };

        assert_eq!(
            hit_test_tab_strip(
                layout,
                ClientPoint {
                    x: close.left + 1,
                    y: close.top + 1
                }
            ),
            Some(TabStripHit {
                index: 4,
                target: TabHitTarget::CloseButton
            })
        );
        assert_eq!(
            hit_test_tab_strip(
                layout,
                ClientPoint {
                    x: TAB_BAR_LEFT + 1,
                    y: tab.top + 1
                }
            ),
            Some(TabStripHit {
                index: 3,
                target: TabHitTarget::Body
            })
        );
    }

    #[test]
    fn tab_strip_hit_test_does_not_report_hidden_tabs_or_dropdown_as_tab() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);
        let dropdown = match layout.dropdown {
            Some(dropdown) => dropdown,
            None => panic!("overflow layout should expose a dropdown button"),
        };

        assert_eq!(
            hit_test_tab_overflow(
                layout,
                ClientPoint {
                    x: dropdown.rect.left + 1,
                    y: dropdown.rect.top + 1
                }
            ),
            Some(TabOverflowHitTarget::Dropdown)
        );
        assert_eq!(
            hit_test_tab_strip(
                layout,
                ClientPoint {
                    x: dropdown.rect.left + 1,
                    y: dropdown.rect.top + 1
                }
            ),
            None
        );
        assert_eq!(tab_rect_for_index(layout, 0), None);
    }

    #[test]
    fn tab_strip_empty_hit_test_reports_only_blank_viewport() {
        let client = UiRect::new(0, 0, TAB_BAR_LEFT + TAB_WIDTH * 2 + TAB_GAP + 80, 400);
        let layout = tab_strip_layout(client, 2, Some(1), 0);
        let first_tab = match tab_rect_for_index(layout, 0) {
            Some(rect) => rect,
            None => panic!("tab index 0 should be visible"),
        };

        assert!(!hit_test_tab_strip_empty(
            layout,
            ClientPoint {
                x: first_tab.left + 1,
                y: first_tab.top + 1
            }
        ));
        assert!(hit_test_tab_strip_empty(
            layout,
            ClientPoint {
                x: TAB_BAR_LEFT + TAB_WIDTH * 2 + TAB_GAP + 10,
                y: 8
            }
        ));
        assert!(!hit_test_tab_strip_empty(
            layout,
            ClientPoint {
                x: TAB_BAR_LEFT,
                y: TAB_BAR_HEIGHT
            }
        ));
    }

    #[test]
    fn tab_strip_empty_hit_test_ignores_overflow_dropdown() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);
        let dropdown = match layout.dropdown {
            Some(dropdown) => dropdown,
            None => panic!("overflow layout should expose a dropdown button"),
        };

        assert!(!hit_test_tab_strip_empty(
            layout,
            ClientPoint {
                x: dropdown.rect.left + 1,
                y: dropdown.rect.top + 1
            }
        ));
    }

    #[test]
    fn overflow_visible_tabs_stay_inside_viewport_and_before_dropdown() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);
        let dropdown = match layout.dropdown {
            Some(dropdown) => dropdown,
            None => panic!("overflow layout should expose a dropdown button"),
        };

        for (_, rect) in visible_tab_rects(layout) {
            assert!(rect.left >= layout.viewport.left);
            assert!(rect.right <= layout.viewport.right);
            assert!(rect.right <= dropdown.rect.left);
        }
    }

    #[test]
    fn overflow_does_not_mark_clipped_tab_sliver_visible() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH - 1);
        let layout = tab_strip_layout(client, 2, Some(1), 0);
        let dropdown = match layout.dropdown {
            Some(dropdown) => dropdown,
            None => panic!("overflow layout should expose a dropdown button"),
        };

        assert_eq!(layout.visible_count, 0);
        assert_eq!(layout.visible_end_index(), 0);
        assert!(!layout.is_index_visible(0));
        assert!(!layout.is_index_visible(1));
        assert_eq!(dropdown.hidden_count, 2);
        assert_eq!(tab_rect_for_index(layout, 0), None);
        assert_eq!(
            hit_test_tab_strip(
                layout,
                ClientPoint {
                    x: layout.viewport.left,
                    y: TAB_TOP + 1
                }
            ),
            None
        );
        assert_eq!(
            tab_reorder_auto_scroll(
                layout,
                ClientPoint {
                    x: layout.viewport.right,
                    y: TAB_TOP + 1
                }
            ),
            None
        );
    }

    #[test]
    fn tab_insertion_target_uses_visible_indices_and_midpoints() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let layout = tab_strip_layout(client, 6, Some(5), 0);
        let first_visible = match tab_rect_for_index(layout, 3) {
            Some(rect) => rect,
            None => panic!("tab index 3 should be visible"),
        };
        let second_visible = match tab_rect_for_index(layout, 4) {
            Some(rect) => rect,
            None => panic!("tab index 4 should be visible"),
        };
        let last_visible = match tab_rect_for_index(layout, 5) {
            Some(rect) => rect,
            None => panic!("tab index 5 should be visible"),
        };

        assert_eq!(
            tab_insertion_target(
                layout,
                ClientPoint {
                    x: first_visible.left,
                    y: first_visible.top
                }
            ),
            Some(TabInsertionTarget {
                before_index: 3,
                x: first_visible.left
            })
        );
        assert_eq!(
            tab_insertion_target(
                layout,
                ClientPoint {
                    x: second_visible.left + second_visible.width() / 2,
                    y: second_visible.top
                }
            ),
            Some(TabInsertionTarget {
                before_index: 5,
                x: last_visible.left
            })
        );
        assert_eq!(
            tab_insertion_target(
                layout,
                ClientPoint {
                    x: last_visible.right + 20,
                    y: last_visible.top
                }
            ),
            Some(TabInsertionTarget {
                before_index: 6,
                x: last_visible.right
            })
        );
    }

    #[test]
    fn tab_reorder_auto_scroll_only_reports_available_overflow_direction() {
        let client = overflow_client_for_viewport_width(TAB_WIDTH * 3 + TAB_GAP * 2);
        let middle = tab_strip_layout(client, 6, None, 2);
        let first = tab_strip_layout(client, 6, None, 0);
        let last = tab_strip_layout(client, 6, None, 3);

        assert_eq!(
            tab_reorder_auto_scroll(
                middle,
                ClientPoint {
                    x: middle.viewport.left,
                    y: 8
                }
            ),
            Some(TabReorderAutoScroll::Backward)
        );
        assert_eq!(
            tab_reorder_auto_scroll(
                middle,
                ClientPoint {
                    x: middle.viewport.right,
                    y: 8
                }
            ),
            Some(TabReorderAutoScroll::Forward)
        );
        assert_eq!(
            tab_reorder_auto_scroll(
                first,
                ClientPoint {
                    x: first.viewport.left,
                    y: 8
                }
            ),
            None
        );
        assert_eq!(
            tab_reorder_auto_scroll(
                last,
                ClientPoint {
                    x: last.viewport.right,
                    y: 8
                }
            ),
            None
        );
    }
}
