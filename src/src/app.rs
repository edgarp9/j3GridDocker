use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use crate::domain::{
    ActiveTabChange, DEFAULT_MIN_REGION_SIZE, DomainError, DomainResult, ExternalProgramSpec,
    LayoutNode, Placement, Rect, RegionId, RegionRect, RemovedPlacement, SplitDirection,
    SplitterPath, SplitterRect, SplitterResizeRollback, TabDeletion, TabId, TabPreset,
    TabPresetProgramPlacement, WindowDisplayState, WindowHandle, WindowSnapshot, Workspace,
    WorkspaceSettings, normalize_tab_preset_name, remove_tab_preset, upsert_tab_preset,
    validate_tab_preset_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationPolicy {
    NoActivate,
    Activate,
}

pub const DEFAULT_WINDOW_ACTIVATION_POLICY: ActivationPolicy = ActivationPolicy::NoActivate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOperation {
    Validate,
    Snapshot,
    InspectProgram,
    Hide,
    Show,
    SetPosition,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowControlError {
    operation: WindowOperation,
    hwnd: Option<WindowHandle>,
    user_message: String,
    internal_detail: Option<String>,
    win32_api: Option<&'static str>,
    last_error: Option<u32>,
}

impl WindowControlError {
    pub fn new(
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
        user_message: impl Into<String>,
        internal_detail: Option<String>,
    ) -> Self {
        Self {
            operation,
            hwnd,
            user_message: user_message.into(),
            internal_detail,
            win32_api: None,
            last_error: None,
        }
    }

    pub fn from_win32(
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
        api: &'static str,
        last_error: u32,
        user_message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            hwnd,
            user_message: user_message.into(),
            internal_detail: Some(format!("{api} failed with GetLastError={last_error}")),
            win32_api: Some(api),
            last_error: Some(last_error),
        }
    }

    pub const fn operation(&self) -> WindowOperation {
        self.operation
    }

    pub const fn hwnd(&self) -> Option<WindowHandle> {
        self.hwnd
    }

    pub fn user_message(&self) -> &str {
        &self.user_message
    }

    pub fn internal_detail(&self) -> Option<&str> {
        self.internal_detail.as_deref()
    }

    pub const fn win32_api(&self) -> Option<&'static str> {
        self.win32_api
    }

    pub const fn last_error(&self) -> Option<u32> {
        self.last_error
    }
}

impl fmt::Display for WindowControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "window operation failed: operation={:?}, hwnd={:?}",
            self.operation, self.hwnd
        )?;

        if let Some(detail) = &self.internal_detail {
            write!(formatter, ", detail={detail}")?;
        }

        if let Some(api) = self.win32_api {
            write!(formatter, ", api={api}")?;
        }

        if let Some(last_error) = self.last_error {
            write!(formatter, ", last_error={last_error}")?;
        }

        Ok(())
    }
}

impl Error for WindowControlError {}

#[derive(Debug)]
pub enum AppError {
    Domain(DomainError),
    Window(WindowControlError),
}

impl AppError {
    pub fn user_message(&self) -> &str {
        match self {
            Self::Domain(error) => error.user_message(),
            Self::Window(error) => error.user_message(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(error) => write!(formatter, "{error}"),
            Self::Window(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Domain(error) => Some(error),
            Self::Window(error) => Some(error),
        }
    }
}

impl From<DomainError> for AppError {
    fn from(value: DomainError) -> Self {
        Self::Domain(value)
    }
}

impl From<WindowControlError> for AppError {
    fn from(value: WindowControlError) -> Self {
        Self::Window(value)
    }
}

pub trait WindowController {
    fn is_valid_external_window(&mut self, hwnd: WindowHandle) -> Result<bool, WindowControlError>;
    fn is_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
    ) -> Result<bool, WindowControlError>;
    fn snapshot(&mut self, hwnd: WindowHandle) -> Result<WindowSnapshot, WindowControlError>;
    fn hide(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError>;
    fn show(
        &mut self,
        snapshot: &WindowSnapshot,
        activation: ActivationPolicy,
    ) -> Result<(), WindowControlError>;
    /// Position an actively docked external window.
    ///
    /// Platform implementations may also repair the non-topmost z-order needed
    /// for top-level dock windows to remain visible above the owner content.
    fn set_position(
        &mut self,
        snapshot: &WindowSnapshot,
        rect: Rect,
    ) -> Result<(), WindowControlError>;
    fn set_position_if_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
        rect: Rect,
    ) -> Result<bool, WindowControlError> {
        if !self.is_same_external_window(snapshot)? {
            return Ok(false);
        }

        match self.set_position(snapshot, rect) {
            Ok(()) => Ok(true),
            Err(error) => match self.is_same_external_window(snapshot) {
                Ok(false) => Ok(false),
                Ok(true) | Err(_) => Err(error),
            },
        }
    }
    fn set_positions_if_same_external_windows(
        &mut self,
        positions: &[WindowPositionRequest<'_>],
    ) -> Vec<WindowPositionResult> {
        self.set_position_requests_if_same_external_windows(positions.iter().copied())
    }
    fn set_position_requests_if_same_external_windows<'a, I>(
        &mut self,
        positions: I,
    ) -> Vec<WindowPositionResult>
    where
        I: Clone + IntoIterator<Item = WindowPositionRequest<'a>>,
    {
        let positions = positions.into_iter();
        let (lower_bound, upper_bound) = positions.size_hint();
        let mut results = Vec::with_capacity(upper_bound.unwrap_or(lower_bound));
        for position in positions {
            match self.set_position_if_same_external_window(position.snapshot(), position.rect()) {
                Ok(true) => results.push(WindowPositionResult::Positioned),
                Ok(false) => results.push(WindowPositionResult::Stale),
                Err(error) => results.push(WindowPositionResult::Failed(error)),
            }
        }
        results
    }
    fn restore(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError>;
    fn restore_detached(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        self.restore(snapshot)
    }
    fn program_spec_for_snapshot(
        &mut self,
        snapshot: &WindowSnapshot,
        _title: Option<String>,
    ) -> Result<ExternalProgramSpec, WindowControlError> {
        Err(WindowControlError::new(
            WindowOperation::InspectProgram,
            Some(snapshot.hwnd()),
            "외부 프로그램 정보를 조회할 수 없습니다.",
            Some(String::from(
                "WindowController implementation does not provide program inspection.",
            )),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowPositionRequest<'a> {
    snapshot: &'a WindowSnapshot,
    rect: Rect,
}

impl<'a> WindowPositionRequest<'a> {
    pub const fn new(snapshot: &'a WindowSnapshot, rect: Rect) -> Self {
        Self { snapshot, rect }
    }

    pub const fn snapshot(self) -> &'a WindowSnapshot {
        self.snapshot
    }

    pub const fn rect(self) -> Rect {
        self.rect
    }
}

#[derive(Debug)]
pub enum WindowPositionResult {
    Positioned,
    Stale,
    Failed(WindowControlError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    workspace: Workspace,
    min_region_size: i32,
    tab_presets: Vec<TabPreset>,
}

impl AppState {
    pub const fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            min_region_size: DEFAULT_MIN_REGION_SIZE,
            tab_presets: Vec::new(),
        }
    }

    pub fn with_min_region_size(min_region_size: i32) -> DomainResult<Self> {
        if min_region_size <= 0 {
            return Err(DomainError::InvalidMinimumRegionSize(min_region_size));
        }

        Ok(Self {
            workspace: Workspace::new(),
            min_region_size,
            tab_presets: Vec::new(),
        })
    }

    pub fn from_workspace(workspace: Workspace, min_region_size: i32) -> DomainResult<Self> {
        if min_region_size <= 0 {
            return Err(DomainError::InvalidMinimumRegionSize(min_region_size));
        }

        Ok(Self {
            workspace,
            min_region_size,
            tab_presets: Vec::new(),
        })
    }

    pub fn from_settings_layout_only(
        settings: WorkspaceSettings,
        min_region_size: i32,
    ) -> DomainResult<(Self, usize)> {
        let (workspace, deferred_placements, tab_presets) =
            Workspace::from_settings_layout_only_preserving_presets(settings)?;
        let mut state = Self::from_workspace(workspace, min_region_size)?;
        state.tab_presets = tab_presets;

        Ok((state, deferred_placements))
    }

    pub fn from_tab_presets_only(
        tab_presets: Vec<TabPreset>,
        min_region_size: i32,
    ) -> DomainResult<Self> {
        let mut state = Self::with_min_region_size(min_region_size)?;
        state.tab_presets = tab_presets;
        Ok(state)
    }

    pub const fn min_region_size(&self) -> i32 {
        self.min_region_size
    }

    pub const fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn list_tab_presets(&self) -> &[TabPreset] {
        &self.tab_presets
    }

    pub fn save_active_tab_preset(
        &mut self,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> DomainResult<TabPreset> {
        let Some(active_tab_id) = self.active_tab_id() else {
            return Err(DomainError::NoActiveTab);
        };
        self.save_tab_preset(active_tab_id, name, programs_by_region_id)
    }

    pub fn save_tab_preset(
        &mut self,
        tab_id: TabId,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> DomainResult<TabPreset> {
        let preset = self.tab_preset_for_tab(tab_id, name, programs_by_region_id)?;

        self.save_tab_preset_value(preset)
    }

    pub fn tab_preset_for_tab(
        &self,
        tab_id: TabId,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> DomainResult<TabPreset> {
        let name = name.into();
        let name = normalize_tab_preset_name(&name);
        let tab = self.workspace.tab(tab_id)?;
        TabPreset::from_layout_and_programs(name, tab.layout(), &programs_by_region_id)
    }

    pub fn save_tab_preset_value(&mut self, preset: TabPreset) -> DomainResult<TabPreset> {
        upsert_tab_preset(&mut self.tab_presets, preset.clone());

        Ok(preset)
    }

    pub fn delete_tab_preset(&mut self, preset_name: &str) -> DomainResult<TabPreset> {
        remove_tab_preset(&mut self.tab_presets, preset_name)
    }

    pub fn update_tab_preset(&mut self, preset: TabPreset) -> DomainResult<TabPreset> {
        let index = self
            .tab_presets
            .iter()
            .position(|existing| existing.name() == preset.name())
            .ok_or_else(|| DomainError::TabPresetNotFound(preset.name().to_owned()))?;
        self.tab_presets[index] = preset.clone();
        Ok(preset)
    }

    pub fn replace_tab_preset(
        &mut self,
        original_name: &str,
        preset: TabPreset,
    ) -> DomainResult<TabPreset> {
        let original_name = normalize_tab_preset_name(original_name);
        let index = self
            .tab_presets
            .iter()
            .position(|existing| existing.name() == original_name)
            .ok_or_else(|| DomainError::TabPresetNotFound(original_name.clone()))?;

        if original_name == preset.name() {
            self.tab_presets[index] = preset.clone();
        } else {
            self.tab_presets.remove(index);
            upsert_tab_preset(&mut self.tab_presets, preset.clone());
        }

        Ok(preset)
    }

    pub fn apply_tab_preset_to_tab(
        &mut self,
        preset_name: &str,
        target_tab_id: TabId,
        bounds: Rect,
    ) -> DomainResult<TabPresetApplication> {
        let target_has_placements = !self.workspace.placements_for_tab(target_tab_id)?.is_empty();
        if target_has_placements {
            return Err(DomainError::TabPresetTargetHasPlacements(target_tab_id));
        }

        let preset_name = normalize_tab_preset_name(preset_name);
        validate_tab_preset_name(&preset_name)?;
        let preset = self
            .tab_presets
            .iter()
            .find(|preset| preset.name() == preset_name)
            .cloned()
            .ok_or_else(|| DomainError::TabPresetNotFound(preset_name.clone()))?;
        let preset_name = preset.name().to_owned();

        let applied_to_active_tab = self.active_tab_id() == Some(target_tab_id);
        let previous_layout = self.workspace.tab(target_tab_id)?.layout().clone();
        let previous_next_region_id = self.workspace.next_region_id();
        let (layout, program_placements) = self
            .workspace
            .layout_and_programs_from_tab_preset(&preset)?;

        if let Err(error) = self.workspace.replace_tab_layout(target_tab_id, layout) {
            self.workspace
                .restore_next_region_id(previous_next_region_id);
            return Err(error);
        }

        let active_regions = if applied_to_active_tab {
            match self.layout_for_tab(target_tab_id, bounds) {
                Ok(regions) => Some(regions),
                Err(error) => {
                    self.rollback_preset_layout_application(
                        target_tab_id,
                        previous_layout,
                        previous_next_region_id,
                    )?;
                    return Err(error);
                }
            }
        } else {
            None
        };

        if let Err(error) = self
            .workspace
            .rename_tab(target_tab_id, preset_name.clone())
        {
            self.rollback_preset_layout_application(
                target_tab_id,
                previous_layout,
                previous_next_region_id,
            )?;
            return Err(error);
        }

        Ok(TabPresetApplication::new(
            preset_name,
            target_tab_id,
            applied_to_active_tab,
            active_regions,
            program_placements,
        ))
    }

    fn rollback_preset_layout_application(
        &mut self,
        target_tab_id: TabId,
        previous_layout: LayoutNode,
        previous_next_region_id: u64,
    ) -> DomainResult<()> {
        let result = self
            .workspace
            .replace_tab_layout(target_tab_id, previous_layout);
        self.workspace
            .restore_next_region_id(previous_next_region_id);
        result
    }

    pub fn add_tab(&mut self, name: impl Into<String>) -> DomainResult<TabId> {
        self.workspace.add_tab(name)
    }

    pub fn delete_tab(&mut self, tab_id: TabId) -> DomainResult<TabDeletion> {
        self.workspace.delete_tab(tab_id)
    }

    pub fn rename_tab(&mut self, tab_id: TabId, name: impl Into<String>) -> DomainResult<()> {
        self.workspace.rename_tab(tab_id, name)
    }

    pub fn reorder_tab_before(
        &mut self,
        tab_id: TabId,
        before_tab_id: Option<TabId>,
    ) -> DomainResult<bool> {
        self.workspace.reorder_tab_before(tab_id, before_tab_id)
    }

    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.workspace.active_tab_id()
    }

    pub fn layout_for_tab(&self, tab_id: TabId, bounds: Rect) -> DomainResult<Vec<RegionRect>> {
        self.workspace
            .layout_for_tab(tab_id, bounds, self.min_region_size)
    }

    pub fn hit_test_region(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
    ) -> DomainResult<Option<RegionId>> {
        self.workspace
            .hit_test_region(tab_id, bounds, x, y, self.min_region_size)
    }

    pub fn splitter_rects(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
    ) -> DomainResult<Vec<SplitterRect>> {
        self.workspace
            .splitter_rects(tab_id, bounds, tolerance, self.min_region_size)
    }

    pub fn hit_test_splitter(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
    ) -> DomainResult<Option<SplitterRect>> {
        self.workspace
            .hit_test_splitter(tab_id, bounds, x, y, tolerance, self.min_region_size)
    }

    pub fn settings(&self) -> DomainResult<WorkspaceSettings> {
        self.workspace
            .to_settings_with_tab_presets(self.tab_presets.clone())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndockStatus {
    Restored,
    WindowMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndockReport {
    attempted: usize,
    restored: usize,
    missing: usize,
    failures: Vec<WindowControlError>,
}

impl UndockReport {
    pub const fn new(
        attempted: usize,
        restored: usize,
        missing: usize,
        failures: Vec<WindowControlError>,
    ) -> Self {
        Self {
            attempted,
            restored,
            missing,
            failures,
        }
    }

    pub const fn empty() -> Self {
        Self {
            attempted: 0,
            restored: 0,
            missing: 0,
            failures: Vec::new(),
        }
    }

    pub const fn attempted(&self) -> usize {
        self.attempted
    }

    pub const fn restored(&self) -> usize {
        self.restored
    }

    pub const fn missing(&self) -> usize {
        self.missing
    }

    pub fn failures(&self) -> &[WindowControlError] {
        &self.failures
    }

    fn record(&mut self, result: Result<UndockStatus, WindowControlError>) {
        self.attempted += 1;

        match result {
            Ok(UndockStatus::Restored) => self.restored += 1,
            Ok(UndockStatus::WindowMissing) => self.missing += 1,
            Err(error) => self.failures.push(error),
        }
    }
}

pub type ShutdownReport = UndockReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabSwitchReport {
    change: ActiveTabChange,
    removed_stale_target_placements: usize,
    removed_stale_previous_placements: usize,
}

impl TabSwitchReport {
    pub const fn new(change: ActiveTabChange, removed_stale_target_placements: usize) -> Self {
        Self {
            change,
            removed_stale_target_placements,
            removed_stale_previous_placements: 0,
        }
    }

    pub const fn with_stale_placements(
        change: ActiveTabChange,
        removed_stale_target_placements: usize,
        removed_stale_previous_placements: usize,
    ) -> Self {
        Self {
            change,
            removed_stale_target_placements,
            removed_stale_previous_placements,
        }
    }

    pub const fn change(self) -> ActiveTabChange {
        self.change
    }

    pub const fn previous(self) -> Option<TabId> {
        self.change.previous()
    }

    pub const fn current(self) -> TabId {
        self.change.current()
    }

    pub const fn removed_stale_target_placements(self) -> usize {
        self.removed_stale_target_placements
    }

    pub const fn removed_stale_previous_placements(self) -> usize {
        self.removed_stale_previous_placements
    }

    pub const fn removed_stale_placements(self) -> usize {
        self.removed_stale_target_placements + self.removed_stale_previous_placements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementRegistration {
    Placed {
        region_id: RegionId,
    },
    Moved {
        from_region_id: RegionId,
        to_region_id: RegionId,
    },
    Resynced {
        region_id: RegionId,
    },
}

impl PlacementRegistration {
    pub const fn target_region_id(self) -> RegionId {
        match self {
            Self::Placed { region_id } | Self::Resynced { region_id } => region_id,
            Self::Moved { to_region_id, .. } => to_region_id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CachedActiveTabLayout<'a> {
    bounds: Rect,
    regions: &'a [RegionRect],
    rects_by_region_id: Option<&'a HashMap<RegionId, Rect>>,
}

impl<'a> CachedActiveTabLayout<'a> {
    pub const fn new(bounds: Rect, regions: &'a [RegionRect]) -> Self {
        Self {
            bounds,
            regions,
            rects_by_region_id: None,
        }
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) const fn with_region_rects(
        bounds: Rect,
        regions: &'a [RegionRect],
        rects_by_region_id: &'a HashMap<RegionId, Rect>,
    ) -> Self {
        Self {
            bounds,
            regions,
            rects_by_region_id: Some(rects_by_region_id),
        }
    }

    fn offset_to(self, bounds: Rect) -> Option<(i32, i32)> {
        if self.bounds.width() != bounds.width() || self.bounds.height() != bounds.height() {
            return None;
        }

        Some((
            bounds.left().checked_sub(self.bounds.left())?,
            bounds.top().checked_sub(self.bounds.top())?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SplitterResizeOutcome {
    Unchanged,
    Changed {
        target_regions: Option<Vec<RegionRect>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTabSyncReport {
    removed_stale_placements: usize,
    computed_regions: Option<Vec<RegionRect>>,
}

impl ActiveTabSyncReport {
    const fn new(
        removed_stale_placements: usize,
        computed_regions: Option<Vec<RegionRect>>,
    ) -> Self {
        Self {
            removed_stale_placements,
            computed_regions,
        }
    }

    pub const fn removed_stale_placements(&self) -> usize {
        self.removed_stale_placements
    }

    pub fn into_computed_regions(self) -> Option<Vec<RegionRect>> {
        self.computed_regions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabDeletionReport {
    deleted_tab_id: TabId,
    previous_active_tab: Option<TabId>,
    current_active_tab: Option<TabId>,
    undock: UndockReport,
}

impl TabDeletionReport {
    pub const fn new(
        deleted_tab_id: TabId,
        previous_active_tab: Option<TabId>,
        current_active_tab: Option<TabId>,
    ) -> Self {
        Self {
            deleted_tab_id,
            previous_active_tab,
            current_active_tab,
            undock: UndockReport::empty(),
        }
    }

    pub const fn deleted_tab_id(&self) -> TabId {
        self.deleted_tab_id
    }

    pub const fn previous_active_tab(&self) -> Option<TabId> {
        self.previous_active_tab
    }

    pub const fn current_active_tab(&self) -> Option<TabId> {
        self.current_active_tab
    }

    pub const fn undock(&self) -> &UndockReport {
        &self.undock
    }

    fn record_undock(&mut self, result: Result<UndockStatus, WindowControlError>) {
        self.undock.record(result);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabPresetApplication {
    preset_name: String,
    target_tab_id: TabId,
    applied_to_active_tab: bool,
    active_regions: Option<Vec<RegionRect>>,
    program_placements: Vec<TabPresetProgramPlacement>,
}

impl TabPresetApplication {
    pub fn new(
        preset_name: String,
        target_tab_id: TabId,
        applied_to_active_tab: bool,
        active_regions: Option<Vec<RegionRect>>,
        program_placements: Vec<TabPresetProgramPlacement>,
    ) -> Self {
        Self {
            preset_name,
            target_tab_id,
            applied_to_active_tab,
            active_regions,
            program_placements,
        }
    }

    pub fn preset_name(&self) -> &str {
        &self.preset_name
    }

    pub const fn target_tab_id(&self) -> TabId {
        self.target_tab_id
    }

    pub const fn applied_to_active_tab(&self) -> bool {
        self.applied_to_active_tab
    }

    pub fn active_regions(&self) -> Option<&[RegionRect]> {
        self.active_regions.as_deref()
    }

    pub fn program_placements(&self) -> &[TabPresetProgramPlacement] {
        &self.program_placements
    }
}

struct PlannedPositionChange {
    snapshot: WindowSnapshot,
    target_rect: Rect,
    rollback_rect: Rect,
}

struct TabSwitchWindow {
    region_id: RegionId,
    snapshot: WindowSnapshot,
}

struct PlannedWindowPosition {
    region_id: RegionId,
    snapshot: WindowSnapshot,
    rect: Rect,
}

struct ShowPositionReport {
    shown: Vec<WindowSnapshot>,
    removed_stale_placements: usize,
    placement_rollback: PlacementRemovalRollback,
}

#[derive(Default)]
struct PlacementRemovalRollback {
    removed: Vec<RemovedPlacement>,
}

impl PlacementRemovalRollback {
    fn push(&mut self, removed: RemovedPlacement) {
        self.removed.push(removed);
    }

    fn restore(self, state: &mut AppState) -> Result<(), AppError> {
        for removed in self.removed.into_iter().rev() {
            state
                .workspace
                .restore_removed_placement(removed)
                .map_err(AppError::from)?;
        }

        Ok(())
    }
}

struct FailedSwitchRollback<'a> {
    target_shown: &'a [WindowSnapshot],
    previous_tab_id: TabId,
    bounds: Rect,
    previous_windows: &'a [TabSwitchWindow],
    hidden_previous: &'a [WindowSnapshot],
    placement_rollback: PlacementRemovalRollback,
}

#[derive(Clone, Copy)]
enum ShowPositionRollback {
    KeepShownOnError,
    HideShownOnError,
}

impl ShowPositionRollback {
    const fn hide_shown_on_error(self) -> bool {
        match self {
            Self::KeepShownOnError => false,
            Self::HideShownOnError => true,
        }
    }
}

struct WindowPlacementSync<'a, C>
where
    C: WindowController,
{
    state: &'a mut AppState,
    controller: &'a mut C,
}

impl<'a, C> WindowPlacementSync<'a, C>
where
    C: WindowController,
{
    fn new(state: &'a mut AppState, controller: &'a mut C) -> Self {
        Self { state, controller }
    }

    fn show_and_position_positions(
        &mut self,
        tab_id: TabId,
        positions: &[(Placement, Rect)],
        rollback: ShowPositionRollback,
    ) -> Result<ShowPositionReport, AppError> {
        let mut shown = Vec::new();
        let mut removed_stale_placements = 0usize;
        let mut placement_rollback = PlacementRemovalRollback::default();

        for (placement, rect) in positions {
            match self.is_current_placement(placement) {
                Ok(true) => {}
                Ok(false) => {
                    match self
                        .state
                        .workspace
                        .remove_placement_for_rollback(tab_id, placement.region_id())
                        .map_err(AppError::from)
                    {
                        Ok(removed) => placement_rollback.push(removed),
                        Err(error) => {
                            if rollback.hide_shown_on_error() {
                                self.hide_windows_best_effort(&shown);
                            }
                            return rollback_removed_placements(
                                &mut *self.state,
                                placement_rollback,
                                error,
                            );
                        }
                    }
                    removed_stale_placements += 1;
                    continue;
                }
                Err(error) => {
                    if rollback.hide_shown_on_error() {
                        self.hide_windows_best_effort(&shown);
                    }
                    return rollback_removed_placements(
                        &mut *self.state,
                        placement_rollback,
                        error,
                    );
                }
            }

            if let Err(error) = self
                .controller
                .show(placement.snapshot(), DEFAULT_WINDOW_ACTIVATION_POLICY)
                .map_err(AppError::from)
            {
                if rollback.hide_shown_on_error() {
                    self.hide_windows_best_effort(&shown);
                }
                return rollback_removed_placements(&mut *self.state, placement_rollback, error);
            }

            shown.push(placement.snapshot().clone());

            if let Err(error) = self
                .controller
                .set_position(placement.snapshot(), *rect)
                .map_err(AppError::from)
            {
                if rollback.hide_shown_on_error() {
                    self.hide_windows_best_effort(&shown);
                }
                return rollback_removed_placements(&mut *self.state, placement_rollback, error);
            }
        }

        Ok(ShowPositionReport {
            shown,
            removed_stale_placements,
            placement_rollback,
        })
    }

    fn show_and_position_window_positions(
        &mut self,
        tab_id: TabId,
        positions: &[PlannedWindowPosition],
        rollback: ShowPositionRollback,
    ) -> Result<ShowPositionReport, AppError> {
        let mut shown = Vec::new();
        let mut removed_stale_placements = 0usize;
        let mut placement_rollback = PlacementRemovalRollback::default();

        for position in positions {
            match self.is_current_snapshot(&position.snapshot) {
                Ok(true) => {}
                Ok(false) => {
                    match self
                        .state
                        .workspace
                        .remove_placement_for_rollback(tab_id, position.region_id)
                        .map_err(AppError::from)
                    {
                        Ok(removed) => placement_rollback.push(removed),
                        Err(error) => {
                            if rollback.hide_shown_on_error() {
                                self.hide_windows_best_effort(&shown);
                            }
                            return rollback_removed_placements(
                                &mut *self.state,
                                placement_rollback,
                                error,
                            );
                        }
                    }
                    removed_stale_placements += 1;
                    continue;
                }
                Err(error) => {
                    if rollback.hide_shown_on_error() {
                        self.hide_windows_best_effort(&shown);
                    }
                    return rollback_removed_placements(
                        &mut *self.state,
                        placement_rollback,
                        error,
                    );
                }
            }

            if let Err(error) = self
                .controller
                .show(&position.snapshot, DEFAULT_WINDOW_ACTIVATION_POLICY)
                .map_err(AppError::from)
            {
                if rollback.hide_shown_on_error() {
                    self.hide_windows_best_effort(&shown);
                }
                return rollback_removed_placements(&mut *self.state, placement_rollback, error);
            }

            shown.push(position.snapshot.clone());

            if let Err(error) = self
                .controller
                .set_position(&position.snapshot, position.rect)
                .map_err(AppError::from)
            {
                if rollback.hide_shown_on_error() {
                    self.hide_windows_best_effort(&shown);
                }
                return rollback_removed_placements(&mut *self.state, placement_rollback, error);
            }
        }

        Ok(ShowPositionReport {
            shown,
            removed_stale_placements,
            placement_rollback,
        })
    }

    fn hide_windows_best_effort(&mut self, snapshots: &[WindowSnapshot]) {
        for snapshot in snapshots {
            let _ = self.controller.hide(snapshot);
        }
    }

    fn show_and_position_positions_best_effort(
        &mut self,
        positions: &[(Placement, Rect)],
        snapshots: &[WindowSnapshot],
    ) {
        for (placement, rect) in positions {
            if !snapshots
                .iter()
                .any(|snapshot| snapshot.hwnd() == placement.hwnd())
            {
                continue;
            }

            if self.ensure_current_placement(placement).is_err() {
                continue;
            }

            if self
                .controller
                .show(placement.snapshot(), DEFAULT_WINDOW_ACTIVATION_POLICY)
                .is_err()
            {
                continue;
            }

            let _ = self.controller.set_position(placement.snapshot(), *rect);
        }
    }

    fn is_current_placement(&mut self, placement: &Placement) -> Result<bool, AppError> {
        self.is_current_snapshot(placement.snapshot())
    }

    fn is_current_snapshot(&mut self, snapshot: &WindowSnapshot) -> Result<bool, AppError> {
        self.controller
            .is_same_external_window(snapshot)
            .map_err(AppError::from)
    }

    fn ensure_current_placement(&mut self, placement: &Placement) -> Result<(), AppError> {
        if self.is_current_placement(placement)? {
            Ok(())
        } else {
            Err(AppError::from(DomainError::InvalidWindowHandle))
        }
    }
}

fn rollback_removed_placements<T>(
    state: &mut AppState,
    rollback: PlacementRemovalRollback,
    error: AppError,
) -> Result<T, AppError> {
    rollback.restore(state)?;
    Err(error)
}

struct ActiveTabPositionSync {
    first_error: Option<AppError>,
    stale_regions: Vec<RegionId>,
}

struct ActiveTabWindowSync {
    position_sync: ActiveTabPositionSync,
    computed_regions: Option<Vec<RegionRect>>,
}

struct SplitRegionLayoutChange {
    new_region: RegionId,
    rollback: SplitRegionRollback,
    target_regions: Vec<RegionRect>,
}

struct SplitRegionRollback {
    tab_id: TabId,
    previous_layout: LayoutNode,
    previous_next_region_id: u64,
}

struct RegionDeletionLayoutChange {
    rollback: RegionDeletionRollback,
}

struct RegionDeletionRollback {
    tab_id: TabId,
    previous_layout: LayoutNode,
    removed: Option<Placement>,
}

struct RegionDeletionWindowRollback {
    previous_positions: Vec<(Placement, Rect)>,
}

struct TabPresetReplacementRollback {
    tab_id: TabId,
    previous_name: String,
    previous_layout: LayoutNode,
    previous_next_region_id: u64,
    previous_placements: Vec<Placement>,
}

impl TabPresetReplacementRollback {
    fn new(state: &AppState, tab_id: TabId, placement_capacity: usize) -> DomainResult<Self> {
        let tab = state.workspace.tab(tab_id)?;

        Ok(Self {
            tab_id,
            previous_name: tab.name().to_owned(),
            previous_layout: tab.layout().clone(),
            previous_next_region_id: state.workspace.next_region_id(),
            previous_placements: Vec::with_capacity(placement_capacity),
        })
    }

    fn push_previous_placement(&mut self, placement: Placement) {
        self.previous_placements.push(placement);
    }

    fn capture_current_placements(&mut self, state: &AppState) -> DomainResult<()> {
        for placement in state.workspace.placements_for_tab(self.tab_id)? {
            if self
                .previous_placements
                .iter()
                .any(|previous| previous.region_id() == placement.region_id())
            {
                continue;
            }

            self.previous_placements.push(placement.clone());
        }

        Ok(())
    }

    fn previous_placement(&self, region_id: RegionId) -> DomainResult<&Placement> {
        self.previous_placements
            .iter()
            .find(|placement| placement.region_id() == region_id)
            .ok_or(DomainError::PlacementNotFound {
                tab_id: self.tab_id,
                region_id,
            })
    }
}

struct TabPresetUndock {
    region_id: RegionId,
    status: UndockStatus,
}

struct ActiveTabRegionPlan<'a> {
    rects: RegionRectPlan<'a>,
    offset: ActiveTabRegionOffset,
}

#[derive(Clone, Copy)]
enum ActiveTabRegionOffset {
    None,
    Translate { dx: i32, dy: i32 },
}

enum RegionRectPlan<'a> {
    Owned(HashMap<RegionId, Rect>),
    Borrowed(&'a HashMap<RegionId, Rect>),
}

#[derive(Default)]
struct RegionRectPair {
    target: Option<Rect>,
    target_generation: u64,
    rollback: Option<Rect>,
    #[cfg(any(test, target_os = "windows"))]
    rollback_index: Option<usize>,
}

struct RegionRectChangePlan {
    rects_by_region_id: HashMap<RegionId, RegionRectPair>,
    generation: u64,
}

#[cfg(any(test, target_os = "windows"))]
type SplitterResizeOwnedSyncResult = (Option<Vec<RegionRect>>, Option<Vec<RegionRect>>);

impl<'a> ActiveTabRegionPlan<'a> {
    fn from_regions(regions: &[RegionRect]) -> Self {
        Self {
            rects: RegionRectPlan::from_regions(regions),
            offset: ActiveTabRegionOffset::None,
        }
    }

    fn from_cached_layout(cached_layout: CachedActiveTabLayout<'a>, bounds: Rect) -> Option<Self> {
        if cached_layout.regions.is_empty() {
            return None;
        }

        let (dx, dy) = cached_layout.offset_to(bounds)?;
        Some(Self {
            rects: RegionRectPlan::from_cached_layout(cached_layout),
            offset: ActiveTabRegionOffset::Translate { dx, dy },
        })
    }

    fn rect_for(&self, region_id: RegionId) -> Result<Rect, AppError> {
        let rect = self.rects.rect_for(region_id)?;
        match self.offset {
            ActiveTabRegionOffset::None => Ok(rect),
            ActiveTabRegionOffset::Translate { dx, dy } => {
                rect.translated(dx, dy).map_err(AppError::from)
            }
        }
    }
}

impl<'a> RegionRectPlan<'a> {
    fn from_regions(regions: &[RegionRect]) -> Self {
        let mut rects_by_region_id = HashMap::with_capacity(regions.len());
        for region in regions {
            rects_by_region_id
                .entry(region.region_id())
                .or_insert(region.rect());
        }

        Self::Owned(rects_by_region_id)
    }

    fn from_cached_layout(cached_layout: CachedActiveTabLayout<'a>) -> Self {
        match cached_layout.rects_by_region_id {
            Some(rects_by_region_id) => Self::Borrowed(rects_by_region_id),
            None => Self::from_regions(cached_layout.regions),
        }
    }

    fn rect_for(&self, region_id: RegionId) -> Result<Rect, AppError> {
        let rects_by_region_id = match self {
            Self::Owned(rects_by_region_id) => rects_by_region_id,
            Self::Borrowed(rects_by_region_id) => *rects_by_region_id,
        };
        rects_by_region_id
            .get(&region_id)
            .copied()
            .ok_or_else(|| AppError::from(DomainError::RegionNotFound(region_id)))
    }
}

impl RegionRectChangePlan {
    fn new() -> Self {
        Self {
            rects_by_region_id: HashMap::new(),
            generation: 0,
        }
    }

    fn from_regions(target_regions: &[RegionRect], rollback_regions: &[RegionRect]) -> Self {
        let mut plan = Self::new();
        plan.replace_from_regions(target_regions, rollback_regions);
        plan
    }

    fn replace_from_regions(
        &mut self,
        target_regions: &[RegionRect],
        rollback_regions: &[RegionRect],
    ) {
        self.rects_by_region_id.clear();
        self.rects_by_region_id
            .reserve(target_regions.len().max(rollback_regions.len()));
        self.advance_generation();
        let generation = self.generation;

        for region in target_regions {
            let pair = self
                .rects_by_region_id
                .entry(region.region_id())
                .or_default();
            pair.set_target_once_for_generation(region.rect(), generation);
        }

        #[cfg(any(test, target_os = "windows"))]
        for (index, region) in rollback_regions.iter().enumerate() {
            let pair = self
                .rects_by_region_id
                .entry(region.region_id())
                .or_default();
            pair.rollback.get_or_insert(region.rect());
            pair.rollback_index.get_or_insert(index);
        }

        #[cfg(not(any(test, target_os = "windows")))]
        for region in rollback_regions {
            let pair = self
                .rects_by_region_id
                .entry(region.region_id())
                .or_default();
            pair.rollback.get_or_insert(region.rect());
        }
    }

    #[cfg(any(test, target_os = "windows"))]
    fn replace_from_partial_target_regions(
        &mut self,
        target_regions: &[RegionRect],
        rollback_regions: &[RegionRect],
        placements: &[Placement],
    ) {
        if self.generation == u64::MAX
            || !self.can_reuse_for_partial_target_regions(
                target_regions,
                rollback_regions,
                placements,
            )
        {
            self.replace_from_regions(target_regions, rollback_regions);
            return;
        }

        self.advance_generation();
        let generation = self.generation;
        for region in target_regions {
            let Some(pair) = self.rects_by_region_id.get_mut(&region.region_id()) else {
                continue;
            };
            let Some(rollback_index) = pair.rollback_index else {
                continue;
            };
            let Some(rollback_region) = rollback_regions.get(rollback_index) else {
                continue;
            };

            pair.rollback = Some(rollback_region.rect());
            pair.set_target_once_for_generation(region.rect(), generation);
        }
    }

    fn target_and_validated_rollback_rect_for(
        &self,
        region_id: RegionId,
    ) -> Result<(Option<Rect>, Rect), AppError> {
        let pair = self
            .rects_by_region_id
            .get(&region_id)
            .ok_or_else(|| AppError::from(DomainError::RegionNotFound(region_id)))?;
        let rollback = pair
            .rollback
            .ok_or_else(|| AppError::from(DomainError::RegionNotFound(region_id)))?;
        let target = if pair.target_generation == self.generation {
            pair.target
        } else {
            None
        };

        Ok((target, rollback))
    }

    #[cfg(any(test, target_os = "windows"))]
    fn replace_region_rects(&self, regions: &mut [RegionRect], updates: &[RegionRect]) -> bool {
        for update in updates {
            let Some(index) = self
                .rects_by_region_id
                .get(&update.region_id())
                .and_then(|pair| pair.rollback_index)
            else {
                return false;
            };

            let Some(region) = regions.get_mut(index) else {
                return false;
            };
            if region.region_id() != update.region_id() {
                return false;
            }

            *region = *update;
        }

        true
    }

    #[cfg(any(test, target_os = "windows"))]
    fn can_reuse_for_partial_target_regions(
        &self,
        target_regions: &[RegionRect],
        rollback_regions: &[RegionRect],
        placements: &[Placement],
    ) -> bool {
        for placement in placements {
            let Some(pair) = self.rects_by_region_id.get(&placement.region_id()) else {
                return false;
            };
            if pair.rollback.is_none() {
                return false;
            }
        }

        for region in target_regions {
            let Some(pair) = self.rects_by_region_id.get(&region.region_id()) else {
                return false;
            };
            let Some(rollback_index) = pair.rollback_index else {
                return false;
            };
            let Some(rollback_region) = rollback_regions.get(rollback_index) else {
                return false;
            };
            if rollback_region.region_id() != region.region_id() {
                return false;
            }
        }

        true
    }

    fn advance_generation(&mut self) {
        if self.generation == u64::MAX {
            self.generation = 1;
        } else {
            self.generation += 1;
        }
    }
}

impl RegionRectPair {
    fn set_target_once_for_generation(&mut self, rect: Rect, generation: u64) {
        if self.target_generation != generation {
            self.target = Some(rect);
            self.target_generation = generation;
        }
    }
}

pub struct App<C>
where
    C: WindowController,
{
    state: AppState,
    controller: C,
    splitter_region_plan: RegionRectChangePlan,
}

impl<C> App<C>
where
    C: WindowController,
{
    pub fn new(controller: C) -> Self {
        Self {
            state: AppState::new(),
            controller,
            splitter_region_plan: RegionRectChangePlan::new(),
        }
    }

    pub fn with_state(controller: C, state: AppState) -> Self {
        Self {
            state,
            controller,
            splitter_region_plan: RegionRectChangePlan::new(),
        }
    }

    pub fn create_initial_tab(&mut self, name: impl Into<String>) -> Result<TabId, AppError> {
        self.add_tab(name)
    }

    pub fn add_tab(&mut self, name: impl Into<String>) -> Result<TabId, AppError> {
        self.state.add_tab(name).map_err(AppError::from)
    }

    pub fn delete_tab(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<TabDeletionReport, AppError> {
        let previous_active_tab = self.state.active_tab_id();
        let rollback_positions = if previous_active_tab == Some(tab_id) {
            self.planned_positions_for_tab(tab_id, bounds)?
        } else {
            Vec::new()
        };
        let deletion = self.state.delete_tab(tab_id)?;
        let previous_active_tab = deletion.previous_active_tab();
        let current_active_tab = deletion.current_active_tab();
        let placements = deletion.removed_tab().placements().to_vec();
        let inactive_when_removed = previous_active_tab != Some(tab_id);
        let mut restored_snapshots = Vec::new();
        let mut report = TabDeletionReport::new(tab_id, previous_active_tab, current_active_tab);

        for placement in placements {
            let status = match self.undock_placement(&placement, inactive_when_removed) {
                Ok(status) => status,
                Err(error) => {
                    let mut rollback_snapshots = restored_snapshots;
                    if error.operation() == WindowOperation::Restore {
                        rollback_snapshots.push(placement.snapshot().clone());
                    }
                    self.rollback_tab_deletion_undocks(
                        &rollback_positions,
                        &rollback_snapshots,
                        inactive_when_removed,
                    );
                    self.state.workspace.restore_deleted_tab(deletion);
                    return Err(AppError::from(error));
                }
            };
            if status == UndockStatus::Restored {
                restored_snapshots.push(placement.snapshot().clone());
            }
            report.record_undock(Ok(status));
        }

        if previous_active_tab == Some(tab_id)
            && let Some(active_tab) = current_active_tab
            && let Err(error) = self.show_and_position_tab(active_tab, bounds)
        {
            self.rollback_tab_deletion_undocks(
                &rollback_positions,
                &restored_snapshots,
                inactive_when_removed,
            );
            self.state.workspace.restore_deleted_tab(deletion);
            return Err(error);
        }

        Ok(report)
    }

    pub fn rename_tab(&mut self, tab_id: TabId, name: impl Into<String>) -> Result<(), AppError> {
        self.state.rename_tab(tab_id, name).map_err(AppError::from)
    }

    pub fn reorder_tab_before(
        &mut self,
        tab_id: TabId,
        before_tab_id: Option<TabId>,
    ) -> Result<bool, AppError> {
        self.state
            .reorder_tab_before(tab_id, before_tab_id)
            .map_err(AppError::from)
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.state.active_tab_id()
    }

    pub fn layout_for_tab(&self, tab_id: TabId, bounds: Rect) -> Result<Vec<RegionRect>, AppError> {
        self.state
            .layout_for_tab(tab_id, bounds)
            .map_err(AppError::from)
    }

    fn rect_for_region(regions: &[RegionRect], region_id: RegionId) -> Result<Rect, AppError> {
        regions
            .iter()
            .find(|region| region.region_id() == region_id)
            .map(|region| region.rect())
            .ok_or_else(|| AppError::from(DomainError::RegionNotFound(region_id)))
    }

    pub fn list_tab_presets(&self) -> &[TabPreset] {
        self.state.list_tab_presets()
    }

    pub fn save_active_tab_preset(
        &mut self,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> Result<TabPreset, AppError> {
        self.state
            .save_active_tab_preset(name, programs_by_region_id)
            .map_err(AppError::from)
    }

    pub fn save_tab_preset(
        &mut self,
        tab_id: TabId,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> Result<TabPreset, AppError> {
        self.state
            .save_tab_preset(tab_id, name, programs_by_region_id)
            .map_err(AppError::from)
    }

    pub fn tab_preset_for_tab(
        &self,
        tab_id: TabId,
        name: impl Into<String>,
        programs_by_region_id: HashMap<RegionId, ExternalProgramSpec>,
    ) -> Result<TabPreset, AppError> {
        self.state
            .tab_preset_for_tab(tab_id, name, programs_by_region_id)
            .map_err(AppError::from)
    }

    pub fn save_tab_preset_value(&mut self, preset: TabPreset) -> Result<TabPreset, AppError> {
        self.state
            .save_tab_preset_value(preset)
            .map_err(AppError::from)
    }

    pub fn delete_tab_preset(&mut self, preset_name: &str) -> Result<TabPreset, AppError> {
        self.state
            .delete_tab_preset(preset_name)
            .map_err(AppError::from)
    }

    pub fn update_tab_preset(&mut self, preset: TabPreset) -> Result<TabPreset, AppError> {
        self.state.update_tab_preset(preset).map_err(AppError::from)
    }

    pub fn replace_tab_preset(
        &mut self,
        original_name: &str,
        preset: TabPreset,
    ) -> Result<TabPreset, AppError> {
        self.state
            .replace_tab_preset(original_name, preset)
            .map_err(AppError::from)
    }

    pub fn apply_tab_preset_to_tab(
        &mut self,
        preset_name: &str,
        target_tab_id: TabId,
        bounds: Rect,
    ) -> Result<TabPresetApplication, AppError> {
        self.state
            .apply_tab_preset_to_tab(preset_name, target_tab_id, bounds)
            .map_err(AppError::from)
    }

    pub fn apply_tab_preset_to_tab_replacing_existing_placements(
        &mut self,
        preset_name: &str,
        target_tab_id: TabId,
        bounds: Rect,
    ) -> Result<(TabPresetApplication, usize), AppError> {
        let placement_count = self
            .state
            .workspace()
            .placements_for_tab(target_tab_id)?
            .len();
        if placement_count == 0 {
            return self
                .apply_tab_preset_to_tab(preset_name, target_tab_id, bounds)
                .map(|application| (application, 0));
        }

        let mut rollback =
            TabPresetReplacementRollback::new(&self.state, target_tab_id, placement_count)?;
        let mut undocked = Vec::with_capacity(placement_count);

        while let Some(placement) = self.next_tab_preset_replacement_placement(target_tab_id)? {
            let region_id = placement.region_id();
            match self.unregister_placement(target_tab_id, region_id) {
                Ok(status) => {
                    rollback.push_previous_placement(placement);
                    undocked.push(TabPresetUndock { region_id, status });
                }
                Err(error) => {
                    rollback.capture_current_placements(&self.state)?;
                    self.rollback_tab_preset_undocked_placements(&undocked, bounds, &rollback)?;
                    return Err(error);
                }
            }
        }

        match self.apply_tab_preset_to_tab(preset_name, target_tab_id, bounds) {
            Ok(application) => Ok((application, undocked.len())),
            Err(error) => {
                self.rollback_tab_preset_undocked_placements(&undocked, bounds, &rollback)?;
                Err(error)
            }
        }
    }

    pub fn hit_test_region(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
    ) -> Result<Option<RegionId>, AppError> {
        self.state
            .hit_test_region(tab_id, bounds, x, y)
            .map_err(AppError::from)
    }

    pub fn splitter_rects(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
    ) -> Result<Vec<SplitterRect>, AppError> {
        self.state
            .splitter_rects(tab_id, bounds, tolerance)
            .map_err(AppError::from)
    }

    pub fn hit_test_splitter(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
    ) -> Result<Option<SplitterRect>, AppError> {
        self.state
            .hit_test_splitter(tab_id, bounds, x, y, tolerance)
            .map_err(AppError::from)
    }

    pub fn split_region(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        direction: SplitDirection,
        bounds: Rect,
    ) -> Result<RegionId, AppError> {
        if self.state.active_tab_id() == Some(tab_id) {
            let change =
                self.split_active_tab_region_layout(tab_id, region_id, direction, bounds)?;

            if let Err(error) = self.sync_split_region_layout_change(bounds, &change) {
                self.rollback_split_region_state(change.rollback)?;
                return Err(error);
            }

            return Ok(change.new_region);
        }

        self.state
            .workspace
            .split_region(tab_id, region_id, direction)
            .map_err(AppError::from)
    }

    pub fn delete_region(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        bounds: Rect,
    ) -> Result<Option<UndockStatus>, AppError> {
        let active_tab = self.state.active_tab_id();
        let window_rollback = self.region_deletion_window_rollback(tab_id, bounds, active_tab)?;
        let change = self.delete_region_layout(tab_id, region_id)?;
        let inactive_hidden = active_tab != Some(tab_id);

        let undock_status = match self.undock_deleted_region_placement(&change, inactive_hidden) {
            Ok(status) => status,
            Err(error) => {
                self.rollback_region_deletion_state(change.rollback)?;
                return Err(AppError::from(error));
            }
        };

        if let Some(window_rollback) = window_rollback.as_ref()
            && let Err(error) =
                self.sync_region_deletion_layout_change(tab_id, bounds, window_rollback)
        {
            self.rollback_region_deletion_state(change.rollback)?;
            return Err(error);
        }

        Ok(undock_status)
    }

    fn split_active_tab_region_layout(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        direction: SplitDirection,
        bounds: Rect,
    ) -> Result<SplitRegionLayoutChange, AppError> {
        let rollback = SplitRegionRollback {
            tab_id,
            previous_layout: self.state.workspace.tab(tab_id)?.layout().clone(),
            previous_next_region_id: self.state.workspace.next_region_id(),
        };
        let new_region = self
            .state
            .workspace
            .split_region(tab_id, region_id, direction)?;

        let target_regions = match Self::validate_tab_layout_in_state(&self.state, tab_id, bounds) {
            Ok(target_regions) => target_regions,
            Err(error) => {
                self.rollback_split_region_state(rollback)?;
                return Err(error);
            }
        };

        Ok(SplitRegionLayoutChange {
            new_region,
            rollback,
            target_regions,
        })
    }

    fn sync_split_region_layout_change(
        &mut self,
        bounds: Rect,
        change: &SplitRegionLayoutChange,
    ) -> Result<(), AppError> {
        self.sync_tab_positions_after_layout_change(
            change.rollback.tab_id,
            bounds,
            &change.rollback.previous_layout,
            Some(&change.target_regions),
        )
    }

    fn region_deletion_window_rollback(
        &self,
        tab_id: TabId,
        bounds: Rect,
        active_tab: Option<TabId>,
    ) -> Result<Option<RegionDeletionWindowRollback>, AppError> {
        if active_tab != Some(tab_id) {
            return Ok(None);
        }

        Ok(Some(RegionDeletionWindowRollback {
            previous_positions: self.planned_positions_for_tab(tab_id, bounds)?,
        }))
    }

    fn delete_region_layout(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> Result<RegionDeletionLayoutChange, AppError> {
        let previous_layout = self.state.workspace.tab(tab_id)?.layout().clone();
        let removed = self.state.workspace.delete_region(tab_id, region_id)?;

        Ok(RegionDeletionLayoutChange {
            rollback: RegionDeletionRollback {
                tab_id,
                previous_layout,
                removed,
            },
        })
    }

    fn undock_deleted_region_placement(
        &mut self,
        change: &RegionDeletionLayoutChange,
        inactive_hidden: bool,
    ) -> Result<Option<UndockStatus>, WindowControlError> {
        let Some(placement) = change.rollback.removed.as_ref() else {
            return Ok(None);
        };

        self.undock_placement_with_inactive_restore_rollback(placement, inactive_hidden)
            .map(Some)
    }

    fn sync_region_deletion_layout_change(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        window_rollback: &RegionDeletionWindowRollback,
    ) -> Result<(), AppError> {
        let target_positions = self.planned_positions_for_tab(tab_id, bounds)?;
        self.set_positions_with_rollback(&target_positions, &window_rollback.previous_positions)
    }

    pub fn resize_splitter(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
    ) -> Result<(), AppError> {
        self.resize_splitter_with_cached_regions(tab_id, path, bounds, pointer_x, pointer_y, None)
            .map(|_| ())
    }

    pub(crate) fn resize_splitter_with_cached_regions(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        cached_rollback_regions: Option<&[RegionRect]>,
    ) -> Result<SplitterResizeOutcome, AppError> {
        let min_region_size = self.state.min_region_size();
        let resize_rollback = self.state.workspace.resize_splitter_if_changed(
            tab_id,
            path,
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )?;

        let Some(resize_rollback) = resize_rollback else {
            return Ok(SplitterResizeOutcome::Unchanged);
        };

        if self.state.active_tab_id() == Some(tab_id) {
            return match self.sync_tab_positions_after_splitter_resize(
                tab_id,
                bounds,
                path,
                resize_rollback,
                cached_rollback_regions,
            ) {
                Ok(target_regions) => Ok(SplitterResizeOutcome::Changed { target_regions }),
                Err(error) => {
                    self.state
                        .workspace
                        .rollback_splitter_resize(tab_id, path, resize_rollback)?;
                    Err(error)
                }
            };
        }

        Ok(SplitterResizeOutcome::Changed {
            target_regions: None,
        })
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn resize_splitter_with_owned_cached_regions(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        cached_rollback_regions: Option<Vec<RegionRect>>,
    ) -> Result<(SplitterResizeOutcome, Option<Vec<RegionRect>>), AppError> {
        let min_region_size = self.state.min_region_size();
        let resize_rollback = self.state.workspace.resize_splitter_if_changed(
            tab_id,
            path,
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )?;

        let Some(resize_rollback) = resize_rollback else {
            return Ok((SplitterResizeOutcome::Unchanged, cached_rollback_regions));
        };

        if self.state.active_tab_id() == Some(tab_id) {
            return match self.sync_tab_positions_after_splitter_resize_owned(
                tab_id,
                bounds,
                path,
                resize_rollback,
                cached_rollback_regions,
            ) {
                Ok((target_regions, retained_cache)) => Ok((
                    SplitterResizeOutcome::Changed { target_regions },
                    retained_cache,
                )),
                Err(error) => {
                    self.state
                        .workspace
                        .rollback_splitter_resize(tab_id, path, resize_rollback)?;
                    Err(error)
                }
            };
        }

        Ok((
            SplitterResizeOutcome::Changed {
                target_regions: None,
            },
            cached_rollback_regions,
        ))
    }

    pub fn switch_tab(&mut self, tab_id: TabId, bounds: Rect) -> Result<TabSwitchReport, AppError> {
        let previous_tab = self.state.active_tab_id();
        self.state.workspace.tab(tab_id)?;
        let change = ActiveTabChange::new(previous_tab, tab_id);

        if previous_tab == Some(tab_id) {
            let report = self.show_and_position_active_tab(tab_id, bounds)?;
            return Ok(TabSwitchReport::new(
                change,
                report.removed_stale_placements,
            ));
        }

        let previous_windows = if let Some(previous_tab) = previous_tab {
            self.tab_switch_windows_for_tab(previous_tab)?
        } else {
            Vec::new()
        };
        let target_positions = self.planned_window_positions_for_tab(tab_id, bounds)?;
        let target_report =
            self.show_and_position_window_positions_with_rollback(tab_id, &target_positions)?;
        let target_shown = target_report.shown;
        let removed_stale_target_placements = target_report.removed_stale_placements;
        let mut switch_placement_rollback = target_report.placement_rollback;
        let mut hidden_previous = Vec::new();
        let mut removed_stale_previous_placements = 0usize;
        let Some(previous_tab_id) = previous_tab else {
            self.state.workspace.set_active_tab(tab_id)?;
            return Ok(TabSwitchReport::with_stale_placements(
                change,
                removed_stale_target_placements,
                0,
            ));
        };

        for previous_window in &previous_windows {
            match self.is_current_snapshot(&previous_window.snapshot) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = self.remove_stale_placement_by_region_with_rollback(
                        previous_tab_id,
                        previous_window.region_id,
                        &mut switch_placement_rollback,
                    ) {
                        return self.rollback_failed_switch(
                            FailedSwitchRollback {
                                target_shown: &target_shown,
                                previous_tab_id,
                                bounds,
                                previous_windows: &previous_windows,
                                hidden_previous: &hidden_previous,
                                placement_rollback: switch_placement_rollback,
                            },
                            error,
                        );
                    }
                    removed_stale_previous_placements += 1;
                    continue;
                }
                Err(error) => {
                    return self.rollback_failed_switch(
                        FailedSwitchRollback {
                            target_shown: &target_shown,
                            previous_tab_id,
                            bounds,
                            previous_windows: &previous_windows,
                            hidden_previous: &hidden_previous,
                            placement_rollback: switch_placement_rollback,
                        },
                        error,
                    );
                }
            }

            if let Err(error) = self.controller.hide(&previous_window.snapshot) {
                if let Ok(false) = self.is_current_snapshot(&previous_window.snapshot) {
                    if let Err(error) = self.remove_stale_placement_by_region_with_rollback(
                        previous_tab_id,
                        previous_window.region_id,
                        &mut switch_placement_rollback,
                    ) {
                        return self.rollback_failed_switch(
                            FailedSwitchRollback {
                                target_shown: &target_shown,
                                previous_tab_id,
                                bounds,
                                previous_windows: &previous_windows,
                                hidden_previous: &hidden_previous,
                                placement_rollback: switch_placement_rollback,
                            },
                            error,
                        );
                    }
                    removed_stale_previous_placements += 1;
                    continue;
                }

                return self.rollback_failed_switch(
                    FailedSwitchRollback {
                        target_shown: &target_shown,
                        previous_tab_id,
                        bounds,
                        previous_windows: &previous_windows,
                        hidden_previous: &hidden_previous,
                        placement_rollback: switch_placement_rollback,
                    },
                    AppError::from(error),
                );
            }

            hidden_previous.push(previous_window.snapshot.clone());
        }

        self.state.workspace.set_active_tab(tab_id)?;

        Ok(TabSwitchReport::with_stale_placements(
            change,
            removed_stale_target_placements,
            removed_stale_previous_placements,
        ))
    }

    pub fn register_placement(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        bounds: Rect,
    ) -> Result<PlacementRegistration, AppError> {
        self.place_window_with_report(tab_id, region_id, hwnd, bounds)
    }

    pub fn place_window(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        bounds: Rect,
    ) -> Result<(), AppError> {
        self.place_window_with_report(tab_id, region_id, hwnd, bounds)
            .map(|_| ())
    }

    fn rollback_state_on_error<T>(
        &mut self,
        placement_rollback: PlacementRemovalRollback,
        error: AppError,
    ) -> Result<T, AppError> {
        rollback_removed_placements(&mut self.state, placement_rollback, error)
    }

    fn rollback_failed_switch<T>(
        &mut self,
        rollback: FailedSwitchRollback<'_>,
        error: AppError,
    ) -> Result<T, AppError> {
        self.hide_windows_best_effort(rollback.target_shown);
        self.show_and_position_tab_windows_best_effort(
            rollback.previous_tab_id,
            rollback.bounds,
            rollback.previous_windows,
            rollback.hidden_previous,
        );
        self.rollback_state_on_error(rollback.placement_rollback, error)
    }

    fn remove_stale_placement_by_region_with_rollback(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        placement_rollback: &mut PlacementRemovalRollback,
    ) -> Result<(), AppError> {
        let removed = self
            .state
            .workspace
            .remove_placement_for_rollback(tab_id, region_id)
            .map_err(AppError::from)?;
        placement_rollback.push(removed);
        Ok(())
    }

    fn place_window_with_report(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        bounds: Rect,
    ) -> Result<PlacementRegistration, AppError> {
        self.ensure_valid_window(hwnd)?;

        let conflicts = self.placement_conflicts_for_new_placement(tab_id, region_id, hwnd);
        let mut placement_rollback = PlacementRemovalRollback::default();
        if let Err(error) =
            self.remove_stale_placement_conflicts(conflicts, &mut placement_rollback)
        {
            return self.rollback_state_on_error(placement_rollback, error);
        }

        let existing_placement = match self.placement_for_window(tab_id, hwnd) {
            Ok(placement) => placement,
            Err(error) => return self.rollback_state_on_error(placement_rollback, error),
        };
        if let Some(placement) = existing_placement {
            return match self.move_existing_placement(tab_id, placement, region_id, bounds) {
                Ok(registration) => Ok(registration),
                Err(error) => self.rollback_state_on_error(placement_rollback, error),
            };
        }

        if let Err(error) = self
            .state
            .workspace
            .ensure_can_place(tab_id, region_id, hwnd)
            .map_err(AppError::from)
        {
            return self.rollback_state_on_error(placement_rollback, error);
        }

        let snapshot = match self.controller.snapshot(hwnd) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return self.rollback_state_on_error(placement_rollback, AppError::from(error));
            }
        };
        let active_tab = self.state.active_tab_id();

        if active_tab == Some(tab_id) {
            let rect = match self.state.workspace.find_region_rect(
                tab_id,
                region_id,
                bounds,
                self.state.min_region_size(),
            ) {
                Ok(rect) => rect,
                Err(error) => {
                    return self.rollback_state_on_error(placement_rollback, AppError::from(error));
                }
            };
            if let Err(error) = self
                .controller
                .show(&snapshot, DEFAULT_WINDOW_ACTIVATION_POLICY)
            {
                self.restore_snapshot_best_effort(&snapshot);
                return self.rollback_state_on_error(placement_rollback, AppError::from(error));
            }
            if let Err(error) = self.controller.set_position(&snapshot, rect) {
                self.restore_snapshot_best_effort(&snapshot);
                return self.rollback_state_on_error(placement_rollback, AppError::from(error));
            }
        } else if let Err(error) = self.controller.hide(&snapshot) {
            return self.rollback_state_on_error(placement_rollback, AppError::from(error));
        }

        if let Err(error) = self
            .state
            .workspace
            .place_window(tab_id, region_id, hwnd, snapshot)
            .map_err(AppError::from)
        {
            return self.rollback_state_on_error(placement_rollback, error);
        }

        Ok(PlacementRegistration::Placed { region_id })
    }

    fn move_existing_placement(
        &mut self,
        tab_id: TabId,
        placement: Placement,
        target_region_id: RegionId,
        bounds: Rect,
    ) -> Result<PlacementRegistration, AppError> {
        self.ensure_current_placement(&placement)?;

        let source_region_id = placement.region_id();
        let moved = self.state.workspace.ensure_can_move_placement(
            tab_id,
            source_region_id,
            target_region_id,
        )?;
        let regions = self.state.layout_for_tab(tab_id, bounds)?;
        let target_rect = Self::rect_for_region(&regions, target_region_id)?;

        let active_tab = self.state.active_tab_id();
        if active_tab == Some(tab_id) {
            let source_rect = Self::rect_for_region(&regions, source_region_id)?;
            if let Err(error) = self
                .controller
                .set_position(placement.snapshot(), target_rect)
            {
                let _ = self
                    .controller
                    .set_position(placement.snapshot(), source_rect);
                return Err(AppError::from(error));
            }
        }

        if moved {
            self.state
                .workspace
                .move_placement(tab_id, source_region_id, target_region_id)?;
            Ok(PlacementRegistration::Moved {
                from_region_id: source_region_id,
                to_region_id: target_region_id,
            })
        } else {
            Ok(PlacementRegistration::Resynced {
                region_id: target_region_id,
            })
        }
    }

    fn rollback_tab_preset_undocked_placements(
        &mut self,
        undocked: &[TabPresetUndock],
        bounds: Rect,
        rollback: &TabPresetReplacementRollback,
    ) -> Result<(), AppError> {
        let restore_result =
            self.restore_tab_preset_undocked_placements(undocked, bounds, rollback);
        let rollback_result = self.rollback_tab_preset_replacement_state(rollback);
        restore_result?;

        rollback_result
    }

    fn restore_tab_preset_undocked_placements(
        &mut self,
        undocked: &[TabPresetUndock],
        bounds: Rect,
        rollback: &TabPresetReplacementRollback,
    ) -> Result<(), AppError> {
        for undocked in undocked.iter().rev() {
            if undocked.status == UndockStatus::WindowMissing {
                continue;
            }

            let placement = rollback.previous_placement(undocked.region_id)?;
            self.place_window(
                rollback.tab_id,
                placement.region_id(),
                placement.hwnd(),
                bounds,
            )?;
        }

        Ok(())
    }

    fn rollback_tab_preset_replacement_state(
        &mut self,
        rollback: &TabPresetReplacementRollback,
    ) -> Result<(), AppError> {
        let layout_result = self
            .state
            .workspace
            .replace_tab_layout(rollback.tab_id, rollback.previous_layout.clone());
        self.state
            .workspace
            .restore_next_region_id(rollback.previous_next_region_id);
        layout_result?;
        self.state
            .workspace
            .rename_tab(rollback.tab_id, rollback.previous_name.clone())?;

        let tab = self.state.workspace.tab_mut(rollback.tab_id)?;
        tab.take_placements();
        for placement in rollback.previous_placements.iter().cloned() {
            tab.add_placement(placement)?;
        }

        Ok(())
    }

    fn next_tab_preset_replacement_placement(
        &self,
        tab_id: TabId,
    ) -> Result<Option<Placement>, AppError> {
        Ok(self
            .state
            .workspace
            .placements_for_tab(tab_id)?
            .first()
            .cloned())
    }

    pub fn unregister_placement(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> Result<UndockStatus, AppError> {
        let active_tab = self.state.active_tab_id();
        let placement = self.placement_for_region(tab_id, region_id)?;
        let status = self.undock_placement_with_inactive_restore_rollback(
            &placement,
            active_tab != Some(tab_id),
        )?;

        self.state.workspace.remove_placement(tab_id, region_id)?;

        Ok(status)
    }

    pub fn detach_active_placement_at(
        &mut self,
        hwnd: WindowHandle,
        rect: Rect,
    ) -> Result<Option<UndockStatus>, AppError> {
        let Some(tab_id) = self.state.active_tab_id() else {
            return Ok(None);
        };
        let Some(placement) = self.placement_for_window(tab_id, hwnd)? else {
            return Ok(None);
        };

        let status = self.detach_placement_at_current_rect(&placement, rect)?;
        self.state
            .workspace
            .remove_placement(tab_id, placement.region_id())?;

        Ok(Some(status))
    }

    pub fn active_tab_region_for_window(
        &mut self,
        hwnd: WindowHandle,
    ) -> Result<Option<RegionId>, AppError> {
        let Some(tab_id) = self.state.active_tab_id() else {
            return Ok(None);
        };

        self.region_for_placed_window(tab_id, hwnd)
    }

    fn region_for_placed_window(
        &mut self,
        tab_id: TabId,
        hwnd: WindowHandle,
    ) -> Result<Option<RegionId>, AppError> {
        let Some(placement) = self.placement_for_window(tab_id, hwnd)? else {
            return Ok(None);
        };

        if self
            .controller
            .is_same_external_window(placement.snapshot())?
        {
            Ok(Some(placement.region_id()))
        } else {
            Ok(None)
        }
    }

    pub fn shutdown(&mut self) -> ShutdownReport {
        self.shutdown_with_active_tab_hidden(false)
    }

    pub fn shutdown_with_active_tab_hidden(&mut self, active_tab_hidden: bool) -> ShutdownReport {
        let active_tab = self.state.active_tab_id();
        let placements = self.shutdown_placements();
        let mut report = ShutdownReport::empty();

        for (tab_id, placement) in placements {
            let inactive_hidden = active_tab != Some(tab_id) || active_tab_hidden;
            let result =
                self.undock_placement_with_inactive_restore_rollback(&placement, inactive_hidden);
            if result.is_ok() {
                let _ = self
                    .state
                    .workspace
                    .remove_placement(tab_id, placement.region_id());
            }
            report.record(result);
        }

        report
    }

    fn shutdown_placements(&self) -> Vec<(TabId, Placement)> {
        self.state
            .workspace
            .tabs()
            .iter()
            .flat_map(|tab| {
                let tab_id = tab.id();
                tab.placements()
                    .iter()
                    .cloned()
                    .map(move |placement| (tab_id, placement))
            })
            .collect()
    }

    pub const fn state(&self) -> &AppState {
        &self.state
    }

    pub fn settings(&self) -> DomainResult<WorkspaceSettings> {
        self.state.settings()
    }

    pub const fn controller(&self) -> &C {
        &self.controller
    }

    pub fn controller_mut(&mut self) -> &mut C {
        &mut self.controller
    }

    fn window_placement_sync(&mut self) -> WindowPlacementSync<'_, C> {
        WindowPlacementSync::new(&mut self.state, &mut self.controller)
    }

    pub fn sync_active_tab(&mut self, bounds: Rect) -> Result<usize, AppError> {
        Ok(self
            .sync_active_tab_with_cached_layout(bounds, None)?
            .removed_stale_placements())
    }

    pub fn sync_active_tab_with_cached_layout(
        &mut self,
        bounds: Rect,
        cached_layout: Option<CachedActiveTabLayout<'_>>,
    ) -> Result<ActiveTabSyncReport, AppError> {
        let Some(tab_id) = self.state.active_tab_id() else {
            return Ok(ActiveTabSyncReport::new(0, None));
        };

        let Some(window_sync) =
            self.position_active_tab_with_layout(tab_id, bounds, cached_layout)?
        else {
            return Ok(ActiveTabSyncReport::new(0, None));
        };
        let ActiveTabWindowSync {
            position_sync,
            computed_regions,
        } = window_sync;
        let ActiveTabPositionSync {
            first_error,
            stale_regions,
        } = position_sync;

        if let Some(error) = first_error {
            return Err(error);
        }

        let removed_stale_placements = stale_regions.len();
        let mut placement_rollback = PlacementRemovalRollback::default();
        for region_id in stale_regions {
            match self
                .state
                .workspace
                .remove_placement_for_rollback(tab_id, region_id)
                .map_err(AppError::from)
            {
                Ok(removed) => placement_rollback.push(removed),
                Err(error) => return self.rollback_state_on_error(placement_rollback, error),
            }
        }

        Ok(ActiveTabSyncReport::new(
            removed_stale_placements,
            computed_regions,
        ))
    }

    fn position_active_tab_with_layout(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        cached_layout: Option<CachedActiveTabLayout<'_>>,
    ) -> Result<Option<ActiveTabWindowSync>, AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok(None);
        }

        if let Some(plan) =
            cached_layout.and_then(|layout| ActiveTabRegionPlan::from_cached_layout(layout, bounds))
            && Self::validate_active_tab_positions(placements, &plan).is_ok()
        {
            return Ok(Some(ActiveTabWindowSync {
                position_sync: Self::position_active_tab_windows(
                    &mut self.controller,
                    placements,
                    &plan,
                )?,
                computed_regions: None,
            }));
        }

        let regions = self.state.layout_for_tab(tab_id, bounds)?;
        let plan = ActiveTabRegionPlan::from_regions(&regions);
        let position_sync =
            Self::sync_active_tab_positions(&mut self.controller, placements, &plan)?;

        Ok(Some(ActiveTabWindowSync {
            position_sync,
            computed_regions: Some(regions),
        }))
    }

    fn sync_active_tab_positions(
        controller: &mut C,
        placements: &[Placement],
        plan: &ActiveTabRegionPlan<'_>,
    ) -> Result<ActiveTabPositionSync, AppError> {
        Self::validate_active_tab_positions(placements, plan)?;
        Self::position_active_tab_windows(controller, placements, plan)
    }

    fn validate_active_tab_positions(
        placements: &[Placement],
        plan: &ActiveTabRegionPlan<'_>,
    ) -> Result<(), AppError> {
        for placement in placements {
            plan.rect_for(placement.region_id())?;
        }
        Ok(())
    }

    fn position_active_tab_windows(
        controller: &mut C,
        placements: &[Placement],
        plan: &ActiveTabRegionPlan<'_>,
    ) -> Result<ActiveTabPositionSync, AppError> {
        let mut first_error = None;
        let mut stale_regions = Vec::new();
        let mut positions = Vec::with_capacity(placements.len());

        for placement in placements {
            let rect = plan.rect_for(placement.region_id())?;
            positions.push(WindowPositionRequest::new(placement.snapshot(), rect));
        }

        let position_results = controller.set_positions_if_same_external_windows(&positions);
        debug_assert_eq!(position_results.len(), placements.len());

        for (placement, result) in placements.iter().zip(position_results) {
            match result {
                WindowPositionResult::Positioned => {}
                WindowPositionResult::Stale => stale_regions.push(placement.region_id()),
                WindowPositionResult::Failed(error) => {
                    if first_error.is_none() {
                        first_error = Some(AppError::from(error));
                    }
                }
            }
        }

        Ok(ActiveTabPositionSync {
            first_error,
            stale_regions,
        })
    }

    pub fn hide_active_tab(&mut self) -> Result<(), AppError> {
        let Some(tab_id) = self.state.active_tab_id() else {
            return Ok(());
        };

        self.hide_tab(tab_id)
    }

    pub fn show_active_tab(&mut self, bounds: Rect) -> Result<usize, AppError> {
        let Some(tab_id) = self.state.active_tab_id() else {
            return Ok(0);
        };

        let report = self.show_and_position_active_tab(tab_id, bounds)?;
        Ok(report.removed_stale_placements)
    }

    fn hide_tab(&mut self, tab_id: TabId) -> Result<(), AppError> {
        let placements = self.placements_for_tab(tab_id)?;
        Self::hide_placement_windows(&mut self.controller, &placements)
    }

    fn hide_placement_windows(
        controller: &mut C,
        placements: &[Placement],
    ) -> Result<(), AppError> {
        let mut first_error = None;

        for placement in placements {
            if let Err(error) = controller.hide(placement.snapshot())
                && first_error.is_none()
            {
                first_error = Some(AppError::from(error));
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn show_and_position_tab(&mut self, tab_id: TabId, bounds: Rect) -> Result<(), AppError> {
        let positions = self.planned_positions_for_tab(tab_id, bounds)?;
        self.show_and_position_positions_with_rollback(tab_id, &positions)?;
        Ok(())
    }

    fn show_and_position_active_tab(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<ShowPositionReport, AppError> {
        let positions = self.planned_positions_for_tab(tab_id, bounds)?;
        self.show_and_position_positions(tab_id, &positions, false)
    }

    fn show_and_position_positions_with_rollback(
        &mut self,
        tab_id: TabId,
        positions: &[(Placement, Rect)],
    ) -> Result<ShowPositionReport, AppError> {
        self.show_and_position_positions(tab_id, positions, true)
    }

    fn show_and_position_window_positions_with_rollback(
        &mut self,
        tab_id: TabId,
        positions: &[PlannedWindowPosition],
    ) -> Result<ShowPositionReport, AppError> {
        self.window_placement_sync()
            .show_and_position_window_positions(
                tab_id,
                positions,
                ShowPositionRollback::HideShownOnError,
            )
    }

    fn show_and_position_positions(
        &mut self,
        tab_id: TabId,
        positions: &[(Placement, Rect)],
        hide_shown_on_error: bool,
    ) -> Result<ShowPositionReport, AppError> {
        let rollback = if hide_shown_on_error {
            ShowPositionRollback::HideShownOnError
        } else {
            ShowPositionRollback::KeepShownOnError
        };
        self.window_placement_sync()
            .show_and_position_positions(tab_id, positions, rollback)
    }

    fn hide_windows_best_effort(&mut self, snapshots: &[WindowSnapshot]) {
        self.window_placement_sync()
            .hide_windows_best_effort(snapshots);
    }

    fn show_and_position_positions_best_effort(
        &mut self,
        positions: &[(Placement, Rect)],
        snapshots: &[WindowSnapshot],
    ) {
        self.window_placement_sync()
            .show_and_position_positions_best_effort(positions, snapshots);
    }

    fn show_and_position_tab_windows_best_effort(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        windows: &[TabSwitchWindow],
        snapshots: &[WindowSnapshot],
    ) {
        if snapshots.is_empty() {
            return;
        }

        let Ok(regions) = self.state.layout_for_tab(tab_id, bounds) else {
            return;
        };
        let region_plan = RegionRectPlan::from_regions(&regions);

        for window in windows {
            if !snapshots
                .iter()
                .any(|snapshot| snapshot.hwnd() == window.snapshot.hwnd())
            {
                continue;
            }

            let Ok(rect) = region_plan.rect_for(window.region_id) else {
                continue;
            };

            if self.ensure_current_snapshot(&window.snapshot).is_err() {
                continue;
            }

            if self
                .controller
                .show(&window.snapshot, DEFAULT_WINDOW_ACTIVATION_POLICY)
                .is_err()
            {
                continue;
            }

            let _ = self.controller.set_position(&window.snapshot, rect);
        }
    }

    fn rollback_tab_deletion_undocks(
        &mut self,
        positions: &[(Placement, Rect)],
        snapshots: &[WindowSnapshot],
        inactive_hidden: bool,
    ) {
        if inactive_hidden {
            self.hide_windows_best_effort(snapshots);
        } else {
            self.show_and_position_positions_best_effort(positions, snapshots);
        }
    }

    fn rollback_split_region_state(
        &mut self,
        rollback: SplitRegionRollback,
    ) -> Result<(), AppError> {
        self.state
            .workspace
            .replace_tab_layout(rollback.tab_id, rollback.previous_layout)?;
        self.state
            .workspace
            .restore_next_region_id(rollback.previous_next_region_id);
        Ok(())
    }

    fn rollback_region_deletion_state(
        &mut self,
        rollback: RegionDeletionRollback,
    ) -> Result<(), AppError> {
        self.state
            .workspace
            .replace_tab_layout(rollback.tab_id, rollback.previous_layout)?;
        if let Some(placement) = rollback.removed {
            self.state
                .workspace
                .tab_mut(rollback.tab_id)?
                .add_placement(placement)?;
        }

        Ok(())
    }

    fn validate_tab_layout_in_state(
        state: &AppState,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<Vec<RegionRect>, AppError> {
        state.layout_for_tab(tab_id, bounds).map_err(AppError::from)
    }

    fn sync_tab_positions_after_layout_change(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        rollback_layout: &LayoutNode,
        cached_target_regions: Option<&[RegionRect]>,
    ) -> Result<(), AppError> {
        self.sync_tab_positions_after_layout_change_with_cached_regions(
            tab_id,
            bounds,
            rollback_layout,
            None,
            cached_target_regions,
        )
        .map(|_| ())
    }

    fn sync_tab_positions_after_layout_change_with_cached_regions(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        rollback_layout: &LayoutNode,
        cached_rollback_regions: Option<&[RegionRect]>,
        cached_target_regions: Option<&[RegionRect]>,
    ) -> Result<Option<Vec<RegionRect>>, AppError> {
        let (position_changes, target_regions) = self.planned_position_changes_for_tab_layout(
            tab_id,
            bounds,
            rollback_layout,
            cached_rollback_regions,
            cached_target_regions,
        )?;
        self.set_position_changes_with_rollback(&position_changes)
            .map(|_| target_regions)
    }

    fn sync_tab_positions_after_splitter_resize(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        path: &SplitterPath,
        resize_rollback: SplitterResizeRollback,
        cached_rollback_regions: Option<&[RegionRect]>,
    ) -> Result<Option<Vec<RegionRect>>, AppError> {
        self.apply_splitter_resize_position_changes(
            tab_id,
            bounds,
            path,
            resize_rollback,
            cached_rollback_regions,
        )
    }

    #[cfg(any(test, target_os = "windows"))]
    fn sync_tab_positions_after_splitter_resize_owned(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        path: &SplitterPath,
        resize_rollback: SplitterResizeRollback,
        cached_rollback_regions: Option<Vec<RegionRect>>,
    ) -> Result<SplitterResizeOwnedSyncResult, AppError> {
        let Some(mut rollback_regions) = cached_rollback_regions else {
            return self
                .apply_splitter_resize_position_changes(tab_id, bounds, path, resize_rollback, None)
                .map(|target_regions| (target_regions, None));
        };

        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok((None, Some(rollback_regions)));
        }

        let min_region_size = self.state.min_region_size();
        let target_updates = self.state.workspace.region_rects_for_splitter_resize(
            tab_id,
            path,
            bounds,
            min_region_size,
        )?;
        self.splitter_region_plan
            .replace_from_partial_target_regions(&target_updates, &rollback_regions, placements);

        if !self
            .splitter_region_plan
            .replace_region_rects(&mut rollback_regions, &target_updates)
        {
            return self
                .apply_splitter_resize_position_changes(tab_id, bounds, path, resize_rollback, None)
                .map(|target_regions| (target_regions, None));
        }

        Self::set_partial_region_plan_position_changes_with_rollback(
            &mut self.controller,
            placements,
            &self.splitter_region_plan,
        )
        .map(|_| (Some(rollback_regions), None))
    }

    fn apply_splitter_resize_position_changes(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        path: &SplitterPath,
        resize_rollback: SplitterResizeRollback,
        cached_rollback_regions: Option<&[RegionRect]>,
    ) -> Result<Option<Vec<RegionRect>>, AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok(None);
        }

        let min_region_size = self.state.min_region_size();
        let computed_rollback_regions;
        let rollback_regions = if let Some(cached_rollback_regions) = cached_rollback_regions {
            cached_rollback_regions
        } else {
            computed_rollback_regions = self.state.workspace.region_rects_before_splitter_resize(
                tab_id,
                path,
                bounds,
                resize_rollback,
                min_region_size,
            )?;
            &computed_rollback_regions
        };
        let target_regions = self.state.layout_for_tab(tab_id, bounds)?;
        self.splitter_region_plan
            .replace_from_regions(&target_regions, rollback_regions);
        Self::set_region_plan_position_changes_with_rollback(
            &mut self.controller,
            placements,
            &self.splitter_region_plan,
        )?;
        Ok(Some(target_regions))
    }

    fn set_positions_with_rollback(
        &mut self,
        target_positions: &[(Placement, Rect)],
        rollback_positions: &[(Placement, Rect)],
    ) -> Result<(), AppError> {
        for (placement, rect) in target_positions {
            if let Err(error) = self.ensure_current_placement(placement) {
                self.set_positions_best_effort(rollback_positions);
                return Err(error);
            }

            if let Err(error) = self
                .controller
                .set_position(placement.snapshot(), *rect)
                .map_err(AppError::from)
            {
                self.set_positions_best_effort(rollback_positions);
                return Err(error);
            }
        }

        Ok(())
    }

    fn set_position_changes_with_rollback(
        &mut self,
        position_changes: &[PlannedPositionChange],
    ) -> Result<(), AppError> {
        if position_changes.is_empty() {
            return Ok(());
        }

        let positions = position_changes
            .iter()
            .map(|change| WindowPositionRequest::new(&change.snapshot, change.target_rect));
        let position_results = self
            .controller
            .set_position_requests_if_same_external_windows(positions);
        debug_assert_eq!(position_results.len(), position_changes.len());

        if let Some(error) = Self::first_position_batch_error(position_results) {
            self.set_position_changes_best_effort(position_changes);
            return Err(error);
        }

        Ok(())
    }

    fn set_region_plan_position_changes_with_rollback(
        controller: &mut C,
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) -> Result<(), AppError> {
        let position_count = Self::validate_region_plan_position_changes(placements, region_plan)?;
        if position_count == 0 {
            return Ok(());
        }

        let positions = placements.iter().filter_map(|placement| {
            let Ok((Some(target_rect), rollback_rect)) =
                region_plan.target_and_validated_rollback_rect_for(placement.region_id())
            else {
                return None;
            };
            if target_rect == rollback_rect {
                return None;
            }

            Some(WindowPositionRequest::new(
                placement.snapshot(),
                target_rect,
            ))
        });

        let position_results = controller.set_position_requests_if_same_external_windows(positions);
        debug_assert_eq!(position_results.len(), position_count);

        if let Some(error) = Self::first_position_batch_error(position_results) {
            Self::set_region_plan_position_changes_best_effort(controller, placements, region_plan);
            return Err(error);
        }

        Ok(())
    }

    #[cfg(any(test, target_os = "windows"))]
    fn set_partial_region_plan_position_changes_with_rollback(
        controller: &mut C,
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) -> Result<(), AppError> {
        let position_count =
            Self::validate_partial_region_plan_position_changes(placements, region_plan)?;
        if position_count == 0 {
            return Ok(());
        }

        let positions = placements.iter().filter_map(|placement| {
            let Ok(Some((target_rect, _rollback_rect))) =
                Self::partial_region_plan_change_rects(placement, region_plan)
            else {
                return None;
            };

            Some(WindowPositionRequest::new(
                placement.snapshot(),
                target_rect,
            ))
        });

        let position_results = controller.set_position_requests_if_same_external_windows(positions);
        debug_assert_eq!(position_results.len(), position_count);

        if let Some(error) = Self::first_position_batch_error(position_results) {
            Self::set_partial_region_plan_position_changes_best_effort(
                controller,
                placements,
                region_plan,
            );
            return Err(error);
        }

        Ok(())
    }

    fn first_position_batch_error(position_results: Vec<WindowPositionResult>) -> Option<AppError> {
        for result in position_results {
            match result {
                WindowPositionResult::Positioned => {}
                WindowPositionResult::Stale => {
                    return Some(AppError::from(DomainError::InvalidWindowHandle));
                }
                WindowPositionResult::Failed(error) => return Some(AppError::from(error)),
            }
        }

        None
    }

    fn validate_region_plan_position_changes(
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) -> Result<usize, AppError> {
        let mut position_count = 0;
        let mut missing_target_region = None;

        for placement in placements {
            let region_id = placement.region_id();
            let (target_rect, rollback_rect) =
                region_plan.target_and_validated_rollback_rect_for(region_id)?;
            if missing_target_region.is_some() {
                continue;
            }

            let Some(target_rect) = target_rect else {
                missing_target_region = Some(region_id);
                continue;
            };

            if target_rect != rollback_rect {
                position_count += 1;
            }
        }

        if let Some(region_id) = missing_target_region {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        }

        Ok(position_count)
    }

    #[cfg(any(test, target_os = "windows"))]
    fn validate_partial_region_plan_position_changes(
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) -> Result<usize, AppError> {
        let mut position_count = 0;
        for placement in placements {
            if Self::partial_region_plan_change_rects(placement, region_plan)?.is_some() {
                position_count += 1;
            }
        }

        Ok(position_count)
    }

    fn set_region_plan_position_changes_best_effort(
        controller: &mut C,
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) {
        for placement in placements {
            let Ok((Some(target_rect), rollback_rect)) =
                region_plan.target_and_validated_rollback_rect_for(placement.region_id())
            else {
                continue;
            };
            if target_rect == rollback_rect {
                continue;
            }
            let snapshot = placement.snapshot();

            if Self::ensure_current_snapshot_with_controller(controller, snapshot).is_err() {
                continue;
            }

            let _ = controller.set_position(snapshot, rollback_rect);
        }
    }

    #[cfg(any(test, target_os = "windows"))]
    fn set_partial_region_plan_position_changes_best_effort(
        controller: &mut C,
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) {
        for placement in placements {
            let Ok(Some((_target_rect, rollback_rect))) =
                Self::partial_region_plan_change_rects(placement, region_plan)
            else {
                continue;
            };
            let snapshot = placement.snapshot();

            if Self::ensure_current_snapshot_with_controller(controller, snapshot).is_err() {
                continue;
            }

            let _ = controller.set_position(snapshot, rollback_rect);
        }
    }

    fn set_positions_best_effort(&mut self, positions: &[(Placement, Rect)]) {
        for (placement, rect) in positions {
            if self.ensure_current_placement(placement).is_err() {
                continue;
            }

            if self
                .controller
                .show(placement.snapshot(), DEFAULT_WINDOW_ACTIVATION_POLICY)
                .is_err()
            {
                continue;
            }

            let _ = self.controller.set_position(placement.snapshot(), *rect);
        }
    }

    fn set_position_changes_best_effort(&mut self, position_changes: &[PlannedPositionChange]) {
        for change in position_changes {
            if self.ensure_current_snapshot(&change.snapshot).is_err() {
                continue;
            }

            let _ = self
                .controller
                .set_position(&change.snapshot, change.rollback_rect);
        }
    }

    fn restore_snapshot_best_effort(&mut self, snapshot: &WindowSnapshot) {
        let _ = self.controller.restore(snapshot);
    }

    fn ensure_valid_window(&mut self, hwnd: WindowHandle) -> Result<(), AppError> {
        if self.controller.is_valid_external_window(hwnd)? {
            Ok(())
        } else {
            Err(AppError::from(DomainError::InvalidWindowHandle))
        }
    }

    fn is_current_placement(&mut self, placement: &Placement) -> Result<bool, AppError> {
        self.is_current_snapshot(placement.snapshot())
    }

    fn is_current_snapshot(&mut self, snapshot: &WindowSnapshot) -> Result<bool, AppError> {
        self.controller
            .is_same_external_window(snapshot)
            .map_err(AppError::from)
    }

    fn ensure_current_placement(&mut self, placement: &Placement) -> Result<(), AppError> {
        if self.is_current_placement(placement)? {
            Ok(())
        } else {
            Err(AppError::from(DomainError::InvalidWindowHandle))
        }
    }

    fn ensure_current_snapshot(&mut self, snapshot: &WindowSnapshot) -> Result<(), AppError> {
        Self::ensure_current_snapshot_with_controller(&mut self.controller, snapshot)
    }

    fn ensure_current_snapshot_with_controller(
        controller: &mut C,
        snapshot: &WindowSnapshot,
    ) -> Result<(), AppError> {
        if controller
            .is_same_external_window(snapshot)
            .map_err(AppError::from)?
        {
            Ok(())
        } else {
            Err(AppError::from(DomainError::InvalidWindowHandle))
        }
    }

    fn undock_placement(
        &mut self,
        placement: &Placement,
        inactive_hidden: bool,
    ) -> Result<UndockStatus, WindowControlError> {
        if !self
            .controller
            .is_same_external_window(placement.snapshot())?
        {
            return Ok(UndockStatus::WindowMissing);
        }

        if inactive_hidden {
            self.controller
                .show(placement.snapshot(), DEFAULT_WINDOW_ACTIVATION_POLICY)?;
        }

        self.controller.restore_detached(placement.snapshot())?;
        Ok(UndockStatus::Restored)
    }

    fn undock_placement_with_inactive_restore_rollback(
        &mut self,
        placement: &Placement,
        inactive_hidden: bool,
    ) -> Result<UndockStatus, WindowControlError> {
        match self.undock_placement(placement, inactive_hidden) {
            Ok(status) => Ok(status),
            Err(error) => {
                if inactive_hidden && error.operation() == WindowOperation::Restore {
                    self.hide_windows_best_effort(&[placement.snapshot().clone()]);
                }
                Err(error)
            }
        }
    }

    fn detach_placement_at_current_rect(
        &mut self,
        placement: &Placement,
        rect: Rect,
    ) -> Result<UndockStatus, WindowControlError> {
        if !self
            .controller
            .is_same_external_window(placement.snapshot())?
        {
            return Ok(UndockStatus::WindowMissing);
        }

        let snapshot = placement
            .snapshot()
            .clone()
            .with_rect(rect)
            .with_display_state(WindowDisplayState::Normal);
        self.controller.restore_detached(&snapshot)?;

        Ok(UndockStatus::Restored)
    }

    fn remove_stale_placement_conflicts(
        &mut self,
        conflicts: Vec<Placement>,
        placement_rollback: &mut PlacementRemovalRollback,
    ) -> Result<usize, AppError> {
        let mut stale_conflicts = Vec::new();
        for placement in conflicts {
            if self.is_current_placement(&placement)? {
                continue;
            }

            stale_conflicts.push(placement);
        }

        let removed = stale_conflicts.len();
        for placement in stale_conflicts {
            let removed = self
                .state
                .workspace
                .remove_placement_for_rollback(placement.tab_id(), placement.region_id())?;
            placement_rollback.push(removed);
        }

        Ok(removed)
    }

    fn placement_conflicts_for_new_placement(
        &self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
    ) -> Vec<Placement> {
        let mut conflicts = Vec::new();
        let mut conflict_keys = HashSet::new();

        for tab in self.state.workspace.tabs() {
            for placement in tab.placements() {
                let conflicts_with_target_region =
                    tab.id() == tab_id && placement.region_id() == region_id;
                let conflicts_with_target_window = placement.hwnd() == hwnd;

                if !conflicts_with_target_region && !conflicts_with_target_window {
                    continue;
                }

                if !conflict_keys.insert((placement.tab_id(), placement.region_id())) {
                    continue;
                }

                conflicts.push(placement.clone());
            }
        }

        conflicts
    }

    fn planned_positions_for_tab(
        &self,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<Vec<(Placement, Rect)>, AppError> {
        self.planned_positions_for_tab_in_state(&self.state, tab_id, bounds)
    }

    fn tab_switch_windows_for_tab(&self, tab_id: TabId) -> Result<Vec<TabSwitchWindow>, AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        let mut windows = Vec::with_capacity(placements.len());

        for placement in placements {
            windows.push(TabSwitchWindow {
                region_id: placement.region_id(),
                snapshot: placement.snapshot().clone(),
            });
        }

        Ok(windows)
    }

    fn planned_window_positions_for_tab(
        &self,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<Vec<PlannedWindowPosition>, AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok(Vec::new());
        }

        let regions = self.state.layout_for_tab(tab_id, bounds)?;
        Self::planned_window_positions_from_regions(placements, &regions)
    }

    fn planned_positions_for_tab_in_state(
        &self,
        state: &AppState,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<Vec<(Placement, Rect)>, AppError> {
        let placements = state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok(Vec::new());
        }

        let regions = state.layout_for_tab(tab_id, bounds)?;
        Self::planned_positions_from_regions(placements, &regions)
    }

    fn planned_position_changes_for_tab_layout(
        &self,
        tab_id: TabId,
        bounds: Rect,
        rollback_layout: &LayoutNode,
        cached_rollback_regions: Option<&[RegionRect]>,
        cached_target_regions: Option<&[RegionRect]>,
    ) -> Result<(Vec<PlannedPositionChange>, Option<Vec<RegionRect>>), AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        if placements.is_empty() {
            return Ok((Vec::new(), None));
        }

        let min_region_size = self.state.min_region_size();
        let computed_rollback_regions;
        let rollback_regions = if let Some(cached_rollback_regions) = cached_rollback_regions {
            cached_rollback_regions
        } else {
            computed_rollback_regions = rollback_layout.region_rects(bounds, min_region_size)?;
            &computed_rollback_regions
        };

        if let Some(target_regions) = cached_target_regions {
            let position_changes = Self::planned_position_changes_from_regions(
                placements,
                target_regions,
                rollback_regions,
            )?;
            return Ok((position_changes, None));
        }

        let target_regions = self.state.layout_for_tab(tab_id, bounds)?;
        let position_changes = Self::planned_position_changes_from_regions(
            placements,
            &target_regions,
            rollback_regions,
        )?;
        Ok((position_changes, Some(target_regions)))
    }

    fn planned_positions_from_regions(
        placements: &[Placement],
        regions: &[RegionRect],
    ) -> Result<Vec<(Placement, Rect)>, AppError> {
        let region_plan = RegionRectPlan::from_regions(regions);
        let mut positions = Vec::with_capacity(placements.len());

        for placement in placements {
            let rect = region_plan.rect_for(placement.region_id())?;
            positions.push((placement.clone(), rect));
        }

        Ok(positions)
    }

    fn planned_window_positions_from_regions(
        placements: &[Placement],
        regions: &[RegionRect],
    ) -> Result<Vec<PlannedWindowPosition>, AppError> {
        let region_plan = RegionRectPlan::from_regions(regions);
        let mut positions = Vec::with_capacity(placements.len());

        for placement in placements {
            let rect = region_plan.rect_for(placement.region_id())?;
            positions.push(PlannedWindowPosition {
                region_id: placement.region_id(),
                snapshot: placement.snapshot().clone(),
                rect,
            });
        }

        Ok(positions)
    }

    fn planned_position_changes_from_regions(
        placements: &[Placement],
        target_regions: &[RegionRect],
        rollback_regions: &[RegionRect],
    ) -> Result<Vec<PlannedPositionChange>, AppError> {
        let region_plan = RegionRectChangePlan::from_regions(target_regions, rollback_regions);
        Self::planned_position_changes_from_region_plan(placements, &region_plan)
    }

    fn planned_position_changes_from_region_plan(
        placements: &[Placement],
        region_plan: &RegionRectChangePlan,
    ) -> Result<Vec<PlannedPositionChange>, AppError> {
        let mut position_changes = Vec::new();
        let mut missing_target_region = None;

        for placement in placements {
            let region_id = placement.region_id();
            let (target_rect, rollback_rect) =
                region_plan.target_and_validated_rollback_rect_for(region_id)?;
            if missing_target_region.is_some() {
                continue;
            }

            let Some(target_rect) = target_rect else {
                missing_target_region = Some(region_id);
                continue;
            };

            if target_rect == rollback_rect {
                continue;
            }

            if position_changes.is_empty() {
                position_changes.reserve(placements.len());
            }

            position_changes.push(PlannedPositionChange {
                snapshot: placement.snapshot().clone(),
                target_rect,
                rollback_rect,
            });
        }

        if let Some(region_id) = missing_target_region {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        }

        Ok(position_changes)
    }

    #[cfg(any(test, target_os = "windows"))]
    fn partial_region_plan_change_rects(
        placement: &Placement,
        region_plan: &RegionRectChangePlan,
    ) -> Result<Option<(Rect, Rect)>, AppError> {
        let region_id = placement.region_id();
        let (target_rect, rollback_rect) =
            region_plan.target_and_validated_rollback_rect_for(region_id)?;
        let Some(target_rect) = target_rect else {
            return Ok(None);
        };

        if target_rect == rollback_rect {
            return Ok(None);
        }

        Ok(Some((target_rect, rollback_rect)))
    }

    fn placement_for_region(
        &self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> Result<Placement, AppError> {
        let placements = self.state.workspace.placements_for_tab(tab_id)?;
        let placement = placements
            .iter()
            .find(|placement| placement.region_id() == region_id)
            .cloned()
            .ok_or_else(|| AppError::from(DomainError::PlacementNotFound { tab_id, region_id }))?;

        Ok(placement)
    }

    fn placement_for_window(
        &self,
        tab_id: TabId,
        hwnd: WindowHandle,
    ) -> Result<Option<Placement>, AppError> {
        Ok(self
            .state
            .workspace
            .placements_for_tab(tab_id)?
            .iter()
            .find(|placement| placement.hwnd() == hwnd)
            .cloned())
    }

    fn placements_for_tab(&self, tab_id: TabId) -> Result<Vec<Placement>, AppError> {
        Ok(self.state.workspace.placements_for_tab(tab_id)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::{WindowDisplayState, WindowHandle};

    #[derive(Debug, Default)]
    struct RecordingWindowController {
        calls: Vec<String>,
        invalid_handles: Vec<WindowHandle>,
        validation_failures: Vec<WindowHandle>,
        mismatched_handles: Vec<WindowHandle>,
        hide_failures: Vec<WindowHandle>,
        show_failures: Vec<WindowHandle>,
        position_failures: Vec<WindowHandle>,
        position_validates_snapshots: bool,
        restore_failures: Vec<WindowHandle>,
        snapshot_display_states: Vec<(WindowHandle, WindowDisplayState)>,
        restored_snapshots: Vec<WindowSnapshot>,
        position_batch_sizes: Vec<usize>,
    }

    impl RecordingWindowController {
        fn calls(&self) -> &[String] {
            &self.calls
        }

        fn position_batch_sizes(&self) -> &[usize] {
            &self.position_batch_sizes
        }

        fn restored_snapshots(&self) -> &[WindowSnapshot] {
            &self.restored_snapshots
        }

        fn clear_calls(&mut self) {
            self.calls.clear();
            self.restored_snapshots.clear();
            self.position_batch_sizes.clear();
        }

        fn mark_invalid(&mut self, hwnd: WindowHandle) {
            self.invalid_handles.push(hwnd);
        }

        fn fail_validation(&mut self, hwnd: WindowHandle) {
            self.validation_failures.push(hwnd);
        }

        fn mark_mismatched(&mut self, hwnd: WindowHandle) {
            self.mismatched_handles.push(hwnd);
        }

        fn fail_hide(&mut self, hwnd: WindowHandle) {
            self.hide_failures.push(hwnd);
        }

        fn fail_show(&mut self, hwnd: WindowHandle) {
            self.show_failures.push(hwnd);
        }

        fn fail_position(&mut self, hwnd: WindowHandle) {
            self.position_failures.push(hwnd);
        }

        fn validate_position_snapshots(&mut self) {
            self.position_validates_snapshots = true;
        }

        fn fail_restore(&mut self, hwnd: WindowHandle) {
            self.restore_failures.push(hwnd);
        }

        fn snapshot_display_state(
            &mut self,
            hwnd: WindowHandle,
            display_state: WindowDisplayState,
        ) {
            self.snapshot_display_states
                .retain(|(handle, _)| *handle != hwnd);
            self.snapshot_display_states.push((hwnd, display_state));
        }

        fn allow_restore(&mut self, hwnd: WindowHandle) {
            self.restore_failures.retain(|failed| *failed != hwnd);
        }

        fn contains_handle(handles: &[WindowHandle], hwnd: WindowHandle) -> bool {
            handles.contains(&hwnd)
        }

        fn set_position_raw(
            &mut self,
            hwnd: WindowHandle,
            rect: Rect,
        ) -> Result<(), WindowControlError> {
            self.calls.push(format!(
                "set:{}:{}:{}:{}:{}",
                hwnd.raw(),
                rect.left(),
                rect.top(),
                rect.width(),
                rect.height()
            ));

            if Self::contains_handle(&self.position_failures, hwnd) {
                Err(WindowControlError::new(
                    WindowOperation::SetPosition,
                    Some(hwnd),
                    "외부 윈도우 위치를 변경할 수 없습니다.",
                    Some(String::from("injected position failure")),
                ))
            } else {
                Ok(())
            }
        }
    }

    impl WindowController for RecordingWindowController {
        fn is_valid_external_window(
            &mut self,
            hwnd: WindowHandle,
        ) -> Result<bool, WindowControlError> {
            self.calls.push(format!("valid:{}", hwnd.raw()));
            if Self::contains_handle(&self.validation_failures, hwnd) {
                return Err(WindowControlError::new(
                    WindowOperation::Validate,
                    Some(hwnd),
                    "외부 윈도우 상태를 확인할 수 없습니다.",
                    Some(String::from("injected validation failure")),
                ));
            }
            Ok(!Self::contains_handle(&self.invalid_handles, hwnd))
        }

        fn is_same_external_window(
            &mut self,
            snapshot: &WindowSnapshot,
        ) -> Result<bool, WindowControlError> {
            let hwnd = snapshot.hwnd();
            Ok(self.is_valid_external_window(hwnd)?
                && !Self::contains_handle(&self.mismatched_handles, hwnd))
        }

        fn snapshot(&mut self, hwnd: WindowHandle) -> Result<WindowSnapshot, WindowControlError> {
            self.calls.push(format!("snapshot:{}", hwnd.raw()));
            let rect = Rect::new(0, 0, 320, 240).map_err(|error| {
                WindowControlError::new(
                    WindowOperation::Snapshot,
                    Some(hwnd),
                    error.user_message(),
                    Some(error.to_string()),
                )
            })?;

            let display_state = self
                .snapshot_display_states
                .iter()
                .find(|(handle, _)| *handle == hwnd)
                .map(|(_, display_state)| *display_state)
                .unwrap_or(WindowDisplayState::Normal);

            Ok(WindowSnapshot::new(hwnd, rect, display_state))
        }

        fn hide(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
            let hwnd = snapshot.hwnd();
            self.calls.push(format!("hide:{}", hwnd.raw()));

            if Self::contains_handle(&self.hide_failures, hwnd) {
                Err(WindowControlError::new(
                    WindowOperation::Hide,
                    Some(hwnd),
                    "외부 윈도우를 숨길 수 없습니다.",
                    Some(String::from("injected hide failure")),
                ))
            } else {
                Ok(())
            }
        }

        fn show(
            &mut self,
            snapshot: &WindowSnapshot,
            activation: ActivationPolicy,
        ) -> Result<(), WindowControlError> {
            let hwnd = snapshot.hwnd();
            self.calls
                .push(format!("show:{}:{activation:?}", hwnd.raw()));

            if Self::contains_handle(&self.show_failures, hwnd) {
                Err(WindowControlError::new(
                    WindowOperation::Show,
                    Some(hwnd),
                    "외부 윈도우를 표시할 수 없습니다.",
                    Some(String::from("injected show failure")),
                ))
            } else {
                Ok(())
            }
        }

        fn set_position(
            &mut self,
            snapshot: &WindowSnapshot,
            rect: Rect,
        ) -> Result<(), WindowControlError> {
            let hwnd = snapshot.hwnd();
            if self.position_validates_snapshots && !self.is_same_external_window(snapshot)? {
                return Err(WindowControlError::new(
                    WindowOperation::Validate,
                    Some(hwnd),
                    "외부 윈도우 상태를 확인할 수 없습니다.",
                    Some(String::from("injected stale snapshot")),
                ));
            }

            self.set_position_raw(hwnd, rect)
        }

        fn set_position_if_same_external_window(
            &mut self,
            snapshot: &WindowSnapshot,
            rect: Rect,
        ) -> Result<bool, WindowControlError> {
            if !self.is_same_external_window(snapshot)? {
                return Ok(false);
            }

            let hwnd = snapshot.hwnd();
            match self.set_position_raw(hwnd, rect) {
                Ok(()) => Ok(true),
                Err(error) => match self.is_same_external_window(snapshot) {
                    Ok(false) => Ok(false),
                    Ok(true) | Err(_) => Err(error),
                },
            }
        }

        fn set_position_requests_if_same_external_windows<'a, I>(
            &mut self,
            positions: I,
        ) -> Vec<WindowPositionResult>
        where
            I: Clone + IntoIterator<Item = WindowPositionRequest<'a>>,
        {
            let positions = positions.into_iter();
            let (lower_bound, upper_bound) = positions.size_hint();
            let mut results = Vec::with_capacity(upper_bound.unwrap_or(lower_bound));
            let batch_index = self.position_batch_sizes.len();
            self.position_batch_sizes.push(0);

            let mut position_count = 0;
            for position in positions {
                position_count += 1;
                match self
                    .set_position_if_same_external_window(position.snapshot(), position.rect())
                {
                    Ok(true) => results.push(WindowPositionResult::Positioned),
                    Ok(false) => results.push(WindowPositionResult::Stale),
                    Err(error) => results.push(WindowPositionResult::Failed(error)),
                }
            }
            self.position_batch_sizes[batch_index] = position_count;
            results
        }

        fn restore(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
            let hwnd = snapshot.hwnd();
            self.calls.push(format!("restore:{}", hwnd.raw()));

            if Self::contains_handle(&self.restore_failures, hwnd) {
                Err(WindowControlError::new(
                    WindowOperation::Restore,
                    Some(hwnd),
                    "외부 윈도우를 복원할 수 없습니다.",
                    Some(String::from("injected restore failure")),
                ))
            } else {
                self.restored_snapshots.push(snapshot.clone());
                Ok(())
            }
        }
    }

    fn assert_calls(actual: &[String], expected: &[&str]) {
        let actual = actual.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    fn root_region(
        app: &App<RecordingWindowController>,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<RegionId, AppError> {
        let regions = app.layout_for_tab(tab_id, bounds)?;
        match regions.first() {
            Some(region) => Ok(region.region_id()),
            None => Err(AppError::from(DomainError::RegionNotFound(RegionId::new(
                0,
            )))),
        }
    }

    fn region_ids_for_tab(
        app: &App<RecordingWindowController>,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<Vec<RegionId>, AppError> {
        Ok(app
            .layout_for_tab(tab_id, bounds)?
            .into_iter()
            .map(RegionRect::region_id)
            .collect())
    }

    fn placement_for_region(region_id: RegionId, hwnd_raw: isize) -> Result<Placement, AppError> {
        let hwnd = WindowHandle::new(hwnd_raw)?;
        let snapshot =
            WindowSnapshot::new(hwnd, Rect::new(0, 0, 320, 240)?, WindowDisplayState::Normal);
        Placement::new(TabId::new(1), region_id, hwnd, snapshot).map_err(AppError::from)
    }

    #[test]
    fn partial_region_plan_reuses_cached_rollback_index_without_rebuilding_all_regions()
    -> Result<(), AppError> {
        let changed_region = RegionId::new(1);
        let unchanged_region = RegionId::new(2);
        let unrelated_region = RegionId::new(3);
        let initial_rollback = vec![
            RegionRect::new(changed_region, Rect::new(0, 0, 400, 600)?),
            RegionRect::new(unchanged_region, Rect::new(400, 0, 400, 600)?),
        ];
        let initial_target = vec![
            RegionRect::new(changed_region, Rect::new(0, 0, 300, 600)?),
            RegionRect::new(unchanged_region, Rect::new(300, 0, 500, 600)?),
        ];
        let mut plan = RegionRectChangePlan::from_regions(&initial_target, &initial_rollback);
        let current_rollback = vec![
            initial_target[0],
            initial_target[1],
            RegionRect::new(unrelated_region, Rect::new(0, 600, 800, 200)?),
        ];
        let target_updates = vec![RegionRect::new(changed_region, Rect::new(0, 0, 250, 600)?)];
        let placements = vec![placement_for_region(changed_region, 100)?];

        plan.replace_from_partial_target_regions(&target_updates, &current_rollback, &placements);

        assert!(!plan.rects_by_region_id.contains_key(&unrelated_region));
        let (target, rollback) = plan.target_and_validated_rollback_rect_for(changed_region)?;
        assert_eq!(target, Some(Rect::new(0, 0, 250, 600)?));
        assert_eq!(rollback, Rect::new(0, 0, 300, 600)?);
        let (target, _rollback) = plan.target_and_validated_rollback_rect_for(unchanged_region)?;
        assert_eq!(target, None);

        let mut retained_cache = current_rollback;
        assert!(plan.replace_region_rects(&mut retained_cache, &target_updates));
        assert_eq!(retained_cache[0], target_updates[0]);
        assert_eq!(retained_cache[2].region_id(), unrelated_region);

        Ok(())
    }

    #[test]
    fn adding_renaming_and_deleting_tabs_updates_workspace_state() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;

        app.rename_tab(second_tab, "Renamed")?;
        let empty_name = app.rename_tab(second_tab, " ");
        let first_report = app.delete_tab(first_tab, bounds)?;
        let second_report = app.delete_tab(second_tab, bounds)?;

        assert!(matches!(
            empty_name,
            Err(AppError::Domain(DomainError::EmptyTabName))
        ));
        assert_eq!(first_report.previous_active_tab(), Some(first_tab));
        assert_eq!(first_report.current_active_tab(), Some(second_tab));
        assert_eq!(first_report.undock().attempted(), 0);
        assert_eq!(second_report.previous_active_tab(), Some(second_tab));
        assert_eq!(second_report.current_active_tab(), None);
        assert_eq!(second_report.undock().attempted(), 0);
        assert_eq!(app.active_tab_id(), None);
        assert!(app.state().workspace().tabs().is_empty());
        assert!(app.controller().calls().is_empty());

        let restarted_tab = app.add_tab("Tab 0")?;

        assert_eq!(restarted_tab, TabId::new(0));
        assert_eq!(root_region(&app, restarted_tab, bounds)?, RegionId::new(0));

        Ok(())
    }

    #[test]
    fn reorder_tabs_updates_app_state_without_changing_active_tab() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let third_tab = app.add_tab("Third")?;

        app.switch_tab(second_tab, Rect::new(0, 0, 800, 600)?)?;
        let changed = app.reorder_tab_before(third_tab, Some(first_tab))?;
        let settings = app.settings()?;

        assert!(changed);
        assert_eq!(app.active_tab_id(), Some(second_tab));
        assert_eq!(
            app.state()
                .workspace()
                .tabs()
                .iter()
                .map(crate::domain::Tab::id)
                .collect::<Vec<_>>(),
            vec![third_tab, first_tab, second_tab]
        );
        assert_eq!(
            settings
                .tabs()
                .iter()
                .map(crate::domain::TabSettings::id)
                .collect::<Vec<_>>(),
            vec![third_tab, first_tab, second_tab]
        );
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn saving_tab_preset_records_layout_and_program_specs() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Work")?;
        let root_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, root_region, SplitDirection::Vertical, bounds)?;
        let program = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\editor.exe",
            ["--profile", "Work A"],
            Some(String::from("Editor")),
        )?;
        let mut programs = HashMap::new();
        programs.insert(right_region, program.clone());

        let saved = app.save_tab_preset(tab_id, "  Workbench  ", programs)?;

        assert_eq!(saved.name(), "Workbench");
        assert_eq!(app.list_tab_presets(), std::slice::from_ref(&saved));
        assert_eq!(app.settings()?.tab_presets(), std::slice::from_ref(&saved));
        match saved.root() {
            crate::domain::TabPresetNode::Split { first, second, .. } => {
                assert!(matches!(
                    first.as_ref(),
                    crate::domain::TabPresetNode::Region { program: None }
                ));
                assert!(matches!(
                    second.as_ref(),
                    crate::domain::TabPresetNode::Region {
                        program: Some(saved_program)
                    } if saved_program == &program
                ));
            }
            crate::domain::TabPresetNode::Region { .. } => {
                return Err(AppError::from(DomainError::RegionNotFound(right_region)));
            }
        }
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn saving_tab_preset_records_all_docked_program_specs() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_tab = app.add_tab("Source")?;
        let target_tab = app.add_tab("Target")?;
        let root_region = root_region(&app, source_tab, bounds)?;
        let right_region =
            app.split_region(source_tab, root_region, SplitDirection::Vertical, bounds)?;
        let bottom_left_region =
            app.split_region(source_tab, root_region, SplitDirection::Horizontal, bounds)?;

        let root_program = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\editor.exe",
            ["--profile", "Root"],
            Some(String::from("Editor")),
        )?;
        let bottom_program = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\terminal.exe",
            ["--working-directory", r"C:\Work"],
            Some(String::from("Terminal")),
        )?;
        let right_program = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\viewer.exe",
            ["--readonly"],
            Some(String::from("Viewer")),
        )?;
        let mut programs = HashMap::new();
        programs.insert(root_region, root_program.clone());
        programs.insert(bottom_left_region, bottom_program.clone());
        programs.insert(right_region, right_program.clone());

        let saved = app.save_tab_preset(source_tab, "Three Programs", programs)?;
        let report = app.apply_tab_preset_to_tab("Three Programs", target_tab, bounds)?;
        let restored_programs = report
            .program_placements()
            .iter()
            .map(|placement| placement.program().clone())
            .collect::<Vec<_>>();

        assert_eq!(saved.name(), "Three Programs");
        assert_eq!(report.program_placements().len(), 3);
        assert!(restored_programs.contains(&root_program));
        assert!(restored_programs.contains(&bottom_program));
        assert!(restored_programs.contains(&right_program));
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn updating_tab_preset_replaces_saved_program_arguments() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Work")?;
        let root_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, root_region, SplitDirection::Vertical, bounds)?;
        let original_program =
            ExternalProgramSpec::new(r"C:\Tools\editor.exe", Some(String::from("Editor")))?;
        let mut programs = HashMap::new();
        programs.insert(right_region, original_program);

        let mut preset = app.save_tab_preset(tab_id, "Workbench", programs)?;
        let original_programs = preset.program_specs();
        assert_eq!(original_programs.len(), 1);
        let edited_program = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\editor-renamed.exe",
            ["--profile", "Review A"],
            original_programs[0].title().map(str::to_owned),
        )?;
        assert_eq!(preset.replace_program_specs([edited_program]), 1);

        let updated = app.update_tab_preset(preset.clone())?;

        assert_eq!(updated, preset);
        assert_eq!(app.list_tab_presets(), std::slice::from_ref(&preset));
        assert_eq!(app.settings()?.tab_presets(), std::slice::from_ref(&preset));
        let report = app.apply_tab_preset_to_tab("Workbench", tab_id, bounds)?;
        assert_eq!(
            report.program_placements()[0].program().executable_path(),
            r"C:\Tools\editor-renamed.exe"
        );
        assert_eq!(
            report.program_placements()[0]
                .program()
                .arguments()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["--profile", "Review A"]
        );
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn replacing_tab_preset_allows_renaming_saved_preset() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Work")?;

        let mut preset = app.save_tab_preset(tab_id, "Old Name", HashMap::new())?;
        preset.rename("Renamed")?;

        let updated = app.replace_tab_preset("Old Name", preset.clone())?;

        assert_eq!(updated.name(), "Renamed");
        assert!(
            app.list_tab_presets()
                .iter()
                .all(|preset| preset.name() != "Old Name")
        );
        assert_eq!(app.list_tab_presets(), std::slice::from_ref(&preset));
        let report = app.apply_tab_preset_to_tab("Renamed", tab_id, bounds)?;
        assert_eq!(report.program_placements(), &[]);

        Ok(())
    }

    #[test]
    fn applying_tab_preset_returns_new_program_regions() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_tab = app.add_tab("Source")?;
        let target_tab = app.add_tab("Target")?;
        let source_root = root_region(&app, source_tab, bounds)?;
        let source_right =
            app.split_region(source_tab, source_root, SplitDirection::Vertical, bounds)?;
        let target_before = region_ids_for_tab(&app, target_tab, bounds)?;
        let program = ExternalProgramSpec::new(r"C:\Tools\terminal.exe", None)?;
        let mut programs = HashMap::new();
        programs.insert(source_right, program.clone());

        app.save_tab_preset(source_tab, "Workbench", programs)?;
        let report = app.apply_tab_preset_to_tab("Workbench", target_tab, bounds)?;
        let target_after = region_ids_for_tab(&app, target_tab, bounds)?;

        assert_eq!(report.preset_name(), "Workbench");
        assert_eq!(report.target_tab_id(), target_tab);
        assert!(!report.applied_to_active_tab());
        assert_eq!(app.state().workspace().tab(target_tab)?.name(), "Workbench");
        assert!(report.active_regions().is_none());
        assert_eq!(target_after.len(), 2);
        assert!(
            target_after
                .iter()
                .all(|region_id| !target_before.contains(region_id))
        );
        assert_eq!(report.program_placements().len(), 1);
        assert!(
            target_after.contains(
                &report
                    .program_placements()
                    .first()
                    .ok_or_else(|| AppError::from(DomainError::NoActiveTab))?
                    .region_id()
            )
        );
        assert_eq!(report.program_placements()[0].program(), &program);
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn replacing_empty_tab_preset_applies_without_undocking() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_tab = app.add_tab("Source")?;
        let target_tab = app.add_tab("Target")?;
        let source_root = root_region(&app, source_tab, bounds)?;
        let target_before = region_ids_for_tab(&app, target_tab, bounds)?;

        app.split_region(source_tab, source_root, SplitDirection::Vertical, bounds)?;
        app.save_tab_preset(source_tab, "Workbench", HashMap::new())?;

        let (report, undocked) = app.apply_tab_preset_to_tab_replacing_existing_placements(
            "Workbench",
            target_tab,
            bounds,
        )?;
        let target_after = region_ids_for_tab(&app, target_tab, bounds)?;

        assert_eq!(undocked, 0);
        assert_eq!(report.preset_name(), "Workbench");
        assert_eq!(report.target_tab_id(), target_tab);
        assert_eq!(app.state().workspace().tab(target_tab)?.name(), "Workbench");
        assert_eq!(target_after.len(), 2);
        assert!(
            target_after
                .iter()
                .all(|region_id| !target_before.contains(region_id))
        );
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn deleting_tab_preset_removes_runtime_and_saved_settings() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Work")?;
        let root_region = root_region(&app, tab_id, bounds)?;

        app.split_region(tab_id, root_region, SplitDirection::Vertical, bounds)?;
        app.save_tab_preset(tab_id, "Workbench", HashMap::new())?;
        let kept = app.save_tab_preset(tab_id, "Scratch", HashMap::new())?;
        let removed = app.delete_tab_preset("  Workbench  ")?;

        assert_eq!(removed.name(), "Workbench");
        assert_eq!(app.list_tab_presets(), std::slice::from_ref(&kept));
        assert_eq!(app.settings()?.tab_presets(), std::slice::from_ref(&kept));
        assert!(matches!(
            app.apply_tab_preset_to_tab("Workbench", tab_id, bounds),
            Err(AppError::Domain(DomainError::TabPresetNotFound(name))) if name == "Workbench"
        ));
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn applying_tab_preset_to_tab_with_placement_is_rejected() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_tab = app.add_tab("Source")?;
        let target_tab = app.add_tab("Target")?;
        let source_root = root_region(&app, source_tab, bounds)?;
        let target_root = root_region(&app, target_tab, bounds)?;
        let hwnd = WindowHandle::new(201)?;

        app.split_region(source_tab, source_root, SplitDirection::Vertical, bounds)?;
        app.save_tab_preset(source_tab, "Workbench", HashMap::new())?;
        app.place_window(target_tab, target_root, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let result = app.apply_tab_preset_to_tab("Workbench", target_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Domain(
                DomainError::TabPresetTargetHasPlacements(tab_id)
            )) if tab_id == target_tab
        ));
        assert_eq!(
            region_ids_for_tab(&app, target_tab, bounds)?,
            vec![target_root]
        );
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn tab_preset_apply_failure_restores_replaced_placements() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let target_tab = app.add_tab("Target")?;
        let target_root = root_region(&app, target_tab, bounds)?;
        let hwnd = WindowHandle::new(201)?;

        app.place_window(target_tab, target_root, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let result = app
            .apply_tab_preset_to_tab_replacing_existing_placements("Missing", target_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::TabPresetNotFound(name))) if name == "Missing"
        ));
        let placements = app.state().workspace().placements_for_tab(target_tab)?;
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), target_root);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:201",
                "restore:201",
                "valid:201",
                "snapshot:201",
                "show:201:NoActivate",
                "set:201:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn tab_preset_undock_failure_restores_prior_undocks() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Work")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(301)?;
        let right_hwnd = WindowHandle::new(302)?;

        app.save_tab_preset(tab_id, "Workbench", HashMap::new())?;
        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_restore(right_hwnd);
        app.controller_mut().clear_calls();

        let result =
            app.apply_tab_preset_to_tab_replacing_existing_placements("Workbench", tab_id, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(right_hwnd)
        ));
        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .any(|placement| placement.region_id() == left_region
                    && placement.hwnd() == left_hwnd)
        );
        assert!(placements.iter().any(
            |placement| placement.region_id() == right_region && placement.hwnd() == right_hwnd
        ));

        Ok(())
    }

    #[test]
    fn active_and_inactive_placement_registration_use_visibility_policy() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;

        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "snapshot:100",
                "show:100:NoActivate",
                "set:100:0:0:800:600",
            ],
        );

        app.controller_mut().clear_calls();
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;

        assert_calls(
            app.controller().calls(),
            &["valid:200", "snapshot:200", "hide:200"],
        );
        assert_eq!(
            app.state().workspace().placements_for_tab(first_tab)?.len(),
            1
        );
        assert_eq!(
            app.state()
                .workspace()
                .placements_for_tab(second_tab)?
                .len(),
            1
        );

        Ok(())
    }

    #[test]
    fn registering_active_existing_placement_moves_it_to_empty_region() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let source_region = root_region(&app, tab_id, bounds)?;
        let target_region =
            app.split_region(tab_id, source_region, SplitDirection::Vertical, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, source_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let registration = app.register_placement(tab_id, target_region, hwnd, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert_eq!(
            registration,
            PlacementRegistration::Moved {
                from_region_id: source_region,
                to_region_id: target_region
            }
        );
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), target_region);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "valid:100",
                "valid:100",
                "set:100:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn moving_active_existing_placement_position_failure_keeps_source_region()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let source_region = root_region(&app, tab_id, bounds)?;
        let target_region =
            app.split_region(tab_id, source_region, SplitDirection::Vertical, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, source_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();
        app.controller_mut().fail_position(hwnd);

        let result = app.register_placement(tab_id, target_region, hwnd, bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), source_region);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "valid:100",
                "valid:100",
                "set:100:400:0:400:600",
                "set:100:0:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn active_placement_show_failure_restores_snapshot_without_recording_state()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.controller_mut().fail_show(hwnd);
        let result = app.place_window(tab_id, region_id, hwnd, bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Show
                    && error.hwnd() == Some(hwnd)
        ));
        assert!(placements.is_empty());
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "snapshot:100",
                "show:100:NoActivate",
                "restore:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn active_placement_position_failure_restores_snapshot_without_recording_state()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.controller_mut().fail_position(hwnd);
        let result = app.place_window(tab_id, region_id, hwnd, bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert!(placements.is_empty());
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "snapshot:100",
                "show:100:NoActivate",
                "set:100:0:0:800:600",
                "restore:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn switching_tabs_uses_documented_order() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let change = app.switch_tab(second_tab, bounds)?;

        assert_eq!(change.previous(), Some(first_tab));
        assert_eq!(change.current(), second_tab);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
                "valid:100",
                "hide:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_show_failure_keeps_previous_active_and_visible() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_show(second_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Show
                    && error.hwnd() == Some(second_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_calls(
            app.controller().calls(),
            &["valid:200", "show:200:NoActivate"],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_set_position_failure_rolls_back_shown_target() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_position(second_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(second_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
                "hide:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_partial_target_failure_hides_all_shown_target_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let left_region = root_region(&app, second_tab, bounds)?;
        let right_region =
            app.split_region(second_tab, left_region, SplitDirection::Vertical, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let left_hwnd = WindowHandle::new(200)?;
        let right_hwnd = WindowHandle::new(300)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, left_region, left_hwnd, bounds)?;
        app.place_window(second_tab, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);
        let placements = app.state().workspace().placements_for_tab(second_tab)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .any(|placement| placement.region_id() == left_region
                    && placement.hwnd() == left_hwnd)
        );
        assert!(placements.iter().any(
            |placement| placement.region_id() == right_region && placement.hwnd() == right_hwnd
        ));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:400:600",
                "valid:300",
                "show:300:NoActivate",
                "set:300:400:0:400:600",
                "hide:200",
                "hide:300",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_same_active_position_failure_keeps_shown_windows_and_placements()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(tab_id, bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(tab_id));
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .any(|placement| placement.region_id() == left_region
                    && placement.hwnd() == left_hwnd)
        );
        assert!(placements.iter().any(
            |placement| placement.region_id() == right_region && placement.hwnd() == right_hwnd
        ));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:400:600",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_hide_failure_keeps_previous_active_and_hides_target() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_hide(first_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Hide
                    && error.hwnd() == Some(first_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
                "valid:100",
                "hide:100",
                "valid:100",
                "hide:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_hide_failure_restores_stale_target_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let left_region = root_region(&app, second_tab, bounds)?;
        let right_region =
            app.split_region(second_tab, left_region, SplitDirection::Vertical, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let stale_hwnd = WindowHandle::new(200)?;
        let right_hwnd = WindowHandle::new(300)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, left_region, stale_hwnd, bounds)?;
        app.place_window(second_tab, right_region, right_hwnd, bounds)?;
        app.controller_mut().mark_mismatched(stale_hwnd);
        app.controller_mut().fail_hide(first_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Hide
                    && error.hwnd() == Some(first_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        let placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].region_id(), left_region);
        assert_eq!(placements[0].hwnd(), stale_hwnd);
        assert_eq!(placements[1].region_id(), right_region);
        assert_eq!(placements[1].hwnd(), right_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "valid:300",
                "show:300:NoActivate",
                "set:300:400:0:400:600",
                "valid:100",
                "hide:100",
                "valid:100",
                "hide:300",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_removes_stale_target_placement_and_switches() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let left_region = root_region(&app, second_tab, bounds)?;
        let right_region =
            app.split_region(second_tab, left_region, SplitDirection::Vertical, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let stale_hwnd = WindowHandle::new(200)?;
        let right_hwnd = WindowHandle::new(300)?;
        let replacement_hwnd = WindowHandle::new(400)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, left_region, stale_hwnd, bounds)?;
        app.place_window(second_tab, right_region, right_hwnd, bounds)?;
        app.controller_mut().mark_mismatched(stale_hwnd);
        app.controller_mut().clear_calls();

        let change = app.switch_tab(second_tab, bounds)?;

        let placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(change.previous(), Some(first_tab));
        assert_eq!(change.current(), second_tab);
        assert_eq!(change.removed_stale_target_placements(), 1);
        assert_eq!(app.active_tab_id(), Some(second_tab));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), right_region);
        assert_eq!(placements[0].hwnd(), right_hwnd);
        app.state()
            .workspace()
            .ensure_can_place(second_tab, left_region, replacement_hwnd)?;
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "valid:300",
                "show:300:NoActivate",
                "set:300:400:0:400:600",
                "valid:100",
                "hide:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_removes_stale_previous_placement_and_switches() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;
        let replacement_hwnd = WindowHandle::new(300)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().mark_invalid(first_hwnd);
        app.controller_mut().clear_calls();

        let change = app.switch_tab(second_tab, bounds)?;

        assert_eq!(change.previous(), Some(first_tab));
        assert_eq!(change.current(), second_tab);
        assert_eq!(change.removed_stale_target_placements(), 0);
        assert_eq!(change.removed_stale_previous_placements(), 1);
        assert_eq!(change.removed_stale_placements(), 1);
        assert_eq!(app.active_tab_id(), Some(second_tab));
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(first_tab)?
                .is_empty()
        );
        app.state()
            .workspace()
            .ensure_can_place(first_tab, first_region, replacement_hwnd)?;
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
                "valid:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn failed_switch_tab_keeps_stale_target_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let left_region = root_region(&app, second_tab, bounds)?;
        let right_region =
            app.split_region(second_tab, left_region, SplitDirection::Vertical, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let stale_hwnd = WindowHandle::new(200)?;
        let right_hwnd = WindowHandle::new(300)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, left_region, stale_hwnd, bounds)?;
        app.place_window(second_tab, right_region, right_hwnd, bounds)?;
        app.controller_mut().mark_mismatched(stale_hwnd);
        app.controller_mut().fail_position(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        let placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].region_id(), left_region);
        assert_eq!(placements[0].hwnd(), stale_hwnd);
        assert_eq!(placements[1].region_id(), right_region);
        assert_eq!(placements[1].hwnd(), right_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "valid:300",
                "show:300:NoActivate",
                "set:300:400:0:400:600",
                "hide:300",
            ],
        );

        Ok(())
    }

    #[test]
    fn switch_tab_identity_check_failure_keeps_previous_active_and_placement()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_validation(second_hwnd);
        app.controller_mut().clear_calls();

        let result = app.switch_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Validate
                    && error.hwnd() == Some(second_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_eq!(
            app.state()
                .workspace()
                .placements_for_tab(second_tab)?
                .len(),
            1
        );
        assert_calls(app.controller().calls(), &["valid:200"]);

        Ok(())
    }

    #[test]
    fn deleting_inactive_tab_shows_hidden_window_before_restore() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let report = app.delete_tab(second_tab, bounds)?;

        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_eq!(report.current_active_tab(), Some(first_tab));
        assert_eq!(report.undock().attempted(), 1);
        assert_eq!(report.undock().restored(), 1);
        assert_calls(
            app.controller().calls(),
            &["valid:200", "show:200:NoActivate", "restore:200"],
        );

        Ok(())
    }

    #[test]
    fn delete_tab_restore_failure_preserves_tab_and_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_restore(second_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_tab(second_tab, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(second_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        app.state().workspace().tab(second_tab)?;
        let placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].hwnd(), second_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
                "hide:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn delete_tab_restore_failure_after_prior_undock_rolls_back_restored_windows()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let left_region = root_region(&app, first_tab, bounds)?;
        let right_region =
            app.split_region(first_tab, left_region, SplitDirection::Vertical, bounds)?;
        let second_tab = app.add_tab("Second")?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, left_region, left_hwnd, bounds)?;
        app.place_window(first_tab, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_restore(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_tab(first_tab, bounds);
        let placements = app.state().workspace().placements_for_tab(first_tab)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        app.state().workspace().tab(first_tab)?;
        app.state().workspace().tab(second_tab)?;
        assert_eq!(placements.len(), 2);
        assert!(placements.iter().any(|placement| {
            placement.region_id() == left_region && placement.hwnd() == left_hwnd
        }));
        assert!(placements.iter().any(|placement| {
            placement.region_id() == right_region && placement.hwnd() == right_hwnd
        }));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "restore:100",
                "valid:200",
                "restore:200",
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:400:600",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn deleting_active_tab_activates_next_tab_and_resyncs_its_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let report = app.delete_tab(first_tab, bounds)?;

        assert_eq!(app.active_tab_id(), Some(second_tab));
        assert_eq!(report.current_active_tab(), Some(second_tab));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "restore:100",
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn deleting_active_tail_tab_activates_previous_tab_and_resyncs_its_windows()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.switch_tab(second_tab, bounds)?;
        app.controller_mut().clear_calls();

        let report = app.delete_tab(second_tab, bounds)?;

        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_eq!(report.previous_active_tab(), Some(second_tab));
        assert_eq!(report.current_active_tab(), Some(first_tab));
        assert_eq!(report.undock().attempted(), 1);
        assert_eq!(report.undock().restored(), 1);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "restore:200",
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn deleting_last_tab_clears_active_tab_and_undocks_remaining_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("Only")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let report = app.delete_tab(tab_id, bounds)?;
        let settings = app.settings()?;

        assert_eq!(app.active_tab_id(), None);
        assert_eq!(report.previous_active_tab(), Some(tab_id));
        assert_eq!(report.current_active_tab(), None);
        assert_eq!(report.undock().attempted(), 1);
        assert_eq!(report.undock().restored(), 1);
        assert!(app.state().workspace().tabs().is_empty());
        assert!(settings.tabs().is_empty());
        assert_eq!(settings.active_tab_id(), None);
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn delete_tab_target_position_failure_restores_deleted_active_tab() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_position(second_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_tab(first_tab, bounds);
        let first_placements = app.state().workspace().placements_for_tab(first_tab)?;
        let second_placements = app.state().workspace().placements_for_tab(second_tab)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(second_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(first_tab));
        assert_eq!(first_placements.len(), 1);
        assert_eq!(first_placements[0].hwnd(), first_hwnd);
        assert_eq!(second_placements.len(), 1);
        assert_eq!(second_placements[0].hwnd(), second_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "restore:100",
                "valid:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
                "hide:200",
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn shutdown_undocks_active_visible_and_inactive_hidden_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let report = app.shutdown();

        assert_eq!(report.attempted(), 2);
        assert_eq!(report.restored(), 2);
        assert_eq!(report.missing(), 0);
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(first_tab)?
                .is_empty()
        );
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(second_tab)?
                .is_empty()
        );
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "restore:100",
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn shutdown_reports_restore_failures_and_continues() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_restore(first_hwnd);
        app.controller_mut().clear_calls();

        let report = app.shutdown();

        assert_eq!(report.attempted(), 2);
        assert_eq!(report.restored(), 1);
        assert_eq!(report.failures().len(), 1);
        let first_placements = app.state().workspace().placements_for_tab(first_tab)?;
        let second_placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(first_placements.len(), 1);
        assert_eq!(first_placements[0].region_id(), first_region);
        assert_eq!(first_placements[0].hwnd(), first_hwnd);
        assert!(second_placements.is_empty());
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "restore:100",
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
            ],
        );

        app.controller_mut().allow_restore(first_hwnd);
        app.controller_mut().clear_calls();

        let retry_report = app.shutdown();

        assert_eq!(retry_report.attempted(), 1);
        assert_eq!(retry_report.restored(), 1);
        assert!(retry_report.failures().is_empty());
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(first_tab)?
                .is_empty()
        );
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn shutdown_rehides_active_window_hidden_by_minimize_after_restore_failure()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();
        app.hide_active_tab()?;
        assert_calls(app.controller().calls(), &["hide:100"]);

        app.controller_mut().fail_restore(hwnd);
        app.controller_mut().clear_calls();

        let report = app.shutdown_with_active_tab_hidden(true);

        assert_eq!(report.attempted(), 1);
        assert_eq!(report.restored(), 0);
        assert_eq!(report.failures().len(), 1);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "show:100:NoActivate",
                "restore:100",
                "hide:100",
            ],
        );

        Ok(())
    }

    #[test]
    fn shutdown_reports_validation_failures_and_continues() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().fail_validation(first_hwnd);
        app.controller_mut().clear_calls();

        let report = app.shutdown();

        assert_eq!(report.attempted(), 2);
        assert_eq!(report.restored(), 1);
        assert_eq!(report.failures().len(), 1);
        let first_placements = app.state().workspace().placements_for_tab(first_tab)?;
        let second_placements = app.state().workspace().placements_for_tab(second_tab)?;
        assert_eq!(first_placements.len(), 1);
        assert_eq!(first_placements[0].region_id(), first_region);
        assert_eq!(first_placements[0].hwnd(), first_hwnd);
        assert!(second_placements.is_empty());
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn invalid_window_is_rejected_when_registering_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(300)?;

        app.controller_mut().mark_invalid(hwnd);
        let result = app.place_window(tab_id, region_id, hwnd, bounds);

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::InvalidWindowHandle))
        ));

        Ok(())
    }

    #[test]
    fn stale_region_placement_is_removed_when_registering_replacement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let stale_hwnd = WindowHandle::new(100)?;
        let replacement_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, region_id, stale_hwnd, bounds)?;
        app.controller_mut().mark_invalid(stale_hwnd);
        app.controller_mut().clear_calls();

        app.place_window(tab_id, region_id, replacement_hwnd, bounds)?;

        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(placements[0].hwnd(), replacement_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "valid:100",
                "snapshot:200",
                "show:200:NoActivate",
                "set:200:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn failed_replacement_registration_keeps_stale_region_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let stale_hwnd = WindowHandle::new(100)?;
        let replacement_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, region_id, stale_hwnd, bounds)?;
        app.controller_mut().mark_invalid(stale_hwnd);
        app.controller_mut().fail_show(replacement_hwnd);
        app.controller_mut().clear_calls();

        let result = app.place_window(tab_id, region_id, replacement_hwnd, bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Show
                    && error.hwnd() == Some(replacement_hwnd)
        ));
        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(placements[0].hwnd(), stale_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "valid:100",
                "snapshot:200",
                "show:200:NoActivate",
                "restore:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn region_collision_is_reported_when_registering_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, region_id, first_hwnd, bounds)?;
        let result = app.place_window(tab_id, region_id, second_hwnd, bounds);

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::RegionAlreadyOccupied(region))) if region == region_id
        ));

        Ok(())
    }

    #[test]
    fn tab_region_mismatch_is_reported_when_registering_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let hwnd = WindowHandle::new(100)?;
        let result = app.place_window(second_tab, first_region, hwnd, bounds);

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::RegionNotFound(region))) if region == first_region
        ));

        Ok(())
    }

    #[test]
    fn unregister_placement_restores_window_and_removes_state() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let status = app.unregister_placement(tab_id, region_id)?;
        let second_result = app.unregister_placement(tab_id, region_id);

        assert_eq!(status, UndockStatus::Restored);
        assert!(matches!(
            second_result,
            Err(AppError::Domain(DomainError::PlacementNotFound { tab_id: missing_tab, region_id: missing_region }))
                if missing_tab == tab_id && missing_region == region_id
        ));
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn detach_active_placement_at_restores_to_drop_rect_and_removes_state() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let drop_rect = Rect::new(320, 240, 300, 220)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.controller_mut()
            .snapshot_display_state(hwnd, WindowDisplayState::Hidden);
        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let status = app.detach_active_placement_at(hwnd, drop_rect)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        let restored_snapshot = app
            .controller()
            .restored_snapshots()
            .first()
            .ok_or(AppError::from(DomainError::InvalidWindowHandle))?;

        assert_eq!(status, Some(UndockStatus::Restored));
        assert!(placements.is_empty());
        assert_eq!(restored_snapshot.rect(), drop_rect);
        assert_eq!(
            restored_snapshot.display_state(),
            WindowDisplayState::Normal
        );
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn detach_active_placement_at_restore_failure_preserves_state() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let drop_rect = Rect::new(320, 240, 300, 220)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().fail_restore(hwnd);
        app.controller_mut().clear_calls();

        let result = app.detach_active_placement_at(hwnd, drop_rect);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert!(app.controller().restored_snapshots().is_empty());
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn active_tab_region_for_window_finds_existing_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let selected = app.active_tab_region_for_window(right_hwnd)?;

        assert_eq!(selected, Some(right_region));
        assert_calls(app.controller().calls(), &["valid:200"]);

        Ok(())
    }

    #[test]
    fn active_tab_region_for_window_ignores_non_active_tab_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let selected = app.active_tab_region_for_window(hwnd)?;

        assert_eq!(selected, None);
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn active_tab_region_for_window_ignores_reused_hwnd_identity() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().mark_mismatched(hwnd);
        app.controller_mut().clear_calls();

        let selected = app.active_tab_region_for_window(hwnd)?;

        assert_eq!(selected, None);
        assert_calls(app.controller().calls(), &["valid:100"]);

        Ok(())
    }

    #[test]
    fn unregister_placement_restore_failure_preserves_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().fail_restore(hwnd);
        app.controller_mut().clear_calls();

        let result = app.unregister_placement(tab_id, region_id);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(app.controller().calls(), &["valid:100", "restore:100"]);

        Ok(())
    }

    #[test]
    fn unregister_inactive_placement_restore_failure_preserves_state_and_rehides_window()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, hwnd, bounds)?;
        app.controller_mut().fail_restore(hwnd);
        app.controller_mut().clear_calls();

        let result = app.unregister_placement(second_tab, second_region);
        let placements = app.state().workspace().placements_for_tab(second_tab)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), second_region);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
                "hide:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn splitting_active_region_keeps_placement_on_first_child_and_resyncs() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let new_region = app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let regions = app.layout_for_tab(tab_id, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(regions[0].region_id(), region_id);
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 400, 600)?);
        assert_eq!(regions[1].region_id(), new_region);
        assert_eq!(regions[1].rect(), Rect::new(400, 0, 400, 600)?);
        assert_calls(
            app.controller().calls(),
            &["valid:100", "set:100:0:0:400:600"],
        );

        Ok(())
    }

    #[test]
    fn split_region_on_empty_active_tab_validates_layout_before_commit() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 100, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let next_region_id = app.state().workspace().next_region_id();

        let result = app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds);
        let regions = app.layout_for_tab(tab_id, bounds)?;

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::RegionTooSmall {
                direction,
                available,
                min_required,
            })) if direction == SplitDirection::Vertical
                && available == 100
                && min_required == DEFAULT_MIN_REGION_SIZE * 2
        ));
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), region_id);
        assert_eq!(regions[0].rect(), bounds);
        assert_eq!(app.state().workspace().next_region_id(), next_region_id);
        assert_calls(app.controller().calls(), &[]);

        Ok(())
    }

    #[test]
    fn split_region_sync_failure_preserves_layout_and_rolls_back_position() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;
        let next_region_id = app.state().workspace().next_region_id();

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().fail_position(hwnd);
        app.controller_mut().clear_calls();

        let result = app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds);
        let regions = app.layout_for_tab(tab_id, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), region_id);
        assert_eq!(regions[0].rect(), bounds);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), region_id);
        assert_eq!(app.state().workspace().next_region_id(), next_region_id);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:400:600",
                "valid:100",
                "valid:100",
                "set:100:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn planned_tab_layout_position_changes_use_cached_target_regions() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;
        let rollback_layout = app.state().workspace().tab(tab_id)?.layout().clone();
        let rollback_regions =
            rollback_layout.region_rects(bounds, app.state().min_region_size())?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;

        let cached_rect = Rect::new(16, 24, 320, 480)?;
        let cached_target_regions = [RegionRect::new(region_id, cached_rect)];
        let (position_changes, computed_target_regions) = app
            .planned_position_changes_for_tab_layout(
                tab_id,
                bounds,
                &rollback_layout,
                Some(&rollback_regions),
                Some(&cached_target_regions),
            )?;

        assert!(computed_target_regions.is_none());
        assert_eq!(position_changes.len(), 1);
        assert_eq!(position_changes[0].target_rect, cached_rect);
        assert_eq!(position_changes[0].rollback_rect, bounds);

        Ok(())
    }

    #[test]
    fn splitting_inactive_region_keeps_hidden_placement_without_window_calls()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let new_region = app.split_region(
            second_tab,
            second_region,
            SplitDirection::Horizontal,
            bounds,
        )?;
        let placements = app.state().workspace().placements_for_tab(second_tab)?;

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), second_region);
        assert!(
            app.state()
                .workspace()
                .tab(second_tab)?
                .layout()
                .contains_region(new_region)
        );
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn deleting_active_region_undocks_removed_window_and_resyncs_remaining() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let status = app.delete_region(tab_id, right_region, bounds)?;
        let regions = app.layout_for_tab(tab_id, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert_eq!(status, Some(UndockStatus::Restored));
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), left_region);
        assert_eq!(regions[0].rect(), bounds);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].hwnd(), left_hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "restore:200",
                "valid:100",
                "set:100:0:0:800:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn delete_region_restore_failure_preserves_region_and_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_restore(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_region(tab_id, right_region, bounds);
        let regions = app.layout_for_tab(tab_id, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(regions.len(), 2);
        assert!(
            regions
                .iter()
                .any(|region| region.region_id() == right_region)
        );
        assert_eq!(placements.len(), 2);
        assert!(placements.iter().any(|placement| {
            placement.region_id() == right_region && placement.hwnd() == right_hwnd
        }));
        assert_calls(app.controller().calls(), &["valid:200", "restore:200"]);

        Ok(())
    }

    #[test]
    fn delete_region_inactive_restore_failure_preserves_state_and_rehides_window()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let top_region = root_region(&app, second_tab, bounds)?;
        let bottom_region =
            app.split_region(second_tab, top_region, SplitDirection::Horizontal, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, bottom_region, hwnd, bounds)?;
        app.controller_mut().fail_restore(hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_region(second_tab, bottom_region, bounds);
        let regions = app.layout_for_tab(second_tab, bounds)?;
        let placements = app.state().workspace().placements_for_tab(second_tab)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Restore
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(regions.len(), 2);
        assert!(
            regions
                .iter()
                .any(|region| region.region_id() == bottom_region)
        );
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), bottom_region);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "show:200:NoActivate",
                "restore:200",
                "hide:200",
            ],
        );

        Ok(())
    }

    #[test]
    fn delete_region_sync_failure_preserves_region_and_placement_after_undock()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(left_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_region(tab_id, right_region, bounds);
        let regions = app.layout_for_tab(tab_id, bounds)?;
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(left_hwnd)
        ));
        assert_eq!(regions.len(), 2);
        assert!(
            regions
                .iter()
                .any(|region| region.region_id() == left_region)
        );
        assert!(
            regions
                .iter()
                .any(|region| region.region_id() == right_region)
        );
        assert_eq!(placements.len(), 2);
        assert!(placements.iter().any(|placement| {
            placement.region_id() == left_region && placement.hwnd() == left_hwnd
        }));
        assert!(placements.iter().any(|placement| {
            placement.region_id() == right_region && placement.hwnd() == right_hwnd
        }));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "restore:200",
                "valid:100",
                "set:100:0:0:800:600",
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:400:600",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn delete_region_sync_failure_reshows_hidden_removed_placement_on_rollback()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.controller_mut()
            .snapshot_display_state(right_hwnd, WindowDisplayState::Hidden);
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(left_hwnd);
        app.controller_mut().clear_calls();

        let result = app.delete_region(tab_id, right_region, bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(left_hwnd)
        ));
        assert!(placements.iter().any(|placement| {
            placement.region_id() == right_region
                && placement.hwnd() == right_hwnd
                && placement.snapshot().display_state() == WindowDisplayState::Hidden
        }));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:200",
                "restore:200",
                "valid:100",
                "set:100:0:0:800:600",
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:400:600",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn deleting_inactive_region_shows_hidden_window_before_restore() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let top_region = root_region(&app, second_tab, bounds)?;
        let bottom_region =
            app.split_region(second_tab, top_region, SplitDirection::Horizontal, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, bottom_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let status = app.delete_region(second_tab, bottom_region, bounds)?;

        assert_eq!(status, Some(UndockStatus::Restored));
        assert_calls(
            app.controller().calls(),
            &["valid:200", "show:200:NoActivate", "restore:200"],
        );

        Ok(())
    }

    #[test]
    fn resizing_splitter_resyncs_active_tab_window() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;
        app.controller_mut().clear_calls();

        app.resize_splitter(tab_id, splitter.path(), bounds, 300, 200)?;

        assert_calls(
            app.controller().calls(),
            &["valid:100", "set:100:0:0:300:600"],
        );

        Ok(())
    }

    #[test]
    fn splitter_resize_paths_batch_active_tab_position_updates() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;

        app.controller_mut().clear_calls();
        let SplitterResizeOutcome::Changed {
            target_regions: Some(cached_regions),
        } = app.resize_splitter_with_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            None,
        )?
        else {
            return Err(AppError::from(DomainError::RegionNotFound(left_region)));
        };

        assert_eq!(app.controller().position_batch_sizes(), &[2]);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:300:600",
                "valid:200",
                "set:200:300:0:500:600",
            ],
        );

        app.controller_mut().clear_calls();
        let (outcome, retained_cache) = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            250,
            200,
            Some(cached_regions),
        )?;

        assert!(matches!(
            outcome,
            SplitterResizeOutcome::Changed {
                target_regions: Some(_)
            }
        ));
        assert!(retained_cache.is_none());
        assert_eq!(app.controller().position_batch_sizes(), &[2]);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:250:600",
                "valid:200",
                "set:200:250:0:550:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn resizing_splitter_to_same_layout_skips_position_sync() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;
        app.controller_mut().clear_calls();

        let outcome = app.resize_splitter_with_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            400,
            200,
            None,
        )?;

        assert_eq!(outcome, SplitterResizeOutcome::Unchanged);
        assert_calls(app.controller().calls(), &[]);

        Ok(())
    }

    #[test]
    fn resize_splitter_sync_failure_preserves_layout_and_rolls_back_position()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;
        app.controller_mut().fail_position(hwnd);
        app.controller_mut().clear_calls();

        let result = app.resize_splitter(tab_id, splitter.path(), bounds, 300, 200);
        let regions = app.layout_for_tab(tab_id, bounds)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 400, 600)?);
        assert_eq!(regions[1].rect(), Rect::new(400, 0, 400, 600)?);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:300:600",
                "valid:100",
                "valid:100",
                "set:100:0:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn cached_splitter_resize_regions_preserve_failure_rollback_position() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;

        let SplitterResizeOutcome::Changed {
            target_regions: Some(cached_regions),
        } = app.resize_splitter_with_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            None,
        )?
        else {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        };

        app.controller_mut().fail_position(hwnd);
        app.controller_mut().clear_calls();

        let result = app.resize_splitter_with_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            250,
            200,
            Some(&cached_regions),
        );
        let regions = app.layout_for_tab(tab_id, bounds)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 300, 600)?);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:250:600",
                "valid:100",
                "valid:100",
                "set:100:0:0:300:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn owned_cached_splitter_resize_regions_preserve_failure_rollback_position()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;

        let (
            SplitterResizeOutcome::Changed {
                target_regions: Some(cached_regions),
            },
            retained_cache,
        ) = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            None,
        )?
        else {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        };
        assert!(retained_cache.is_none());

        app.controller_mut().fail_position(hwnd);
        app.controller_mut().clear_calls();

        let result = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            250,
            200,
            Some(cached_regions),
        );
        let regions = app.layout_for_tab(tab_id, bounds)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(hwnd)
        ));
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 300, 600)?);
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:250:600",
                "valid:100",
                "valid:100",
                "set:100:0:0:300:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn owned_cached_splitter_resize_uses_single_snapshot_validation_per_position_update()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;

        let (
            SplitterResizeOutcome::Changed {
                target_regions: Some(cached_regions),
            },
            retained_cache,
        ) = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            None,
        )?
        else {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        };
        assert!(retained_cache.is_none());

        app.controller_mut().validate_position_snapshots();
        app.controller_mut().clear_calls();

        let (outcome, retained_cache) = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            250,
            200,
            Some(cached_regions),
        )?;

        assert!(matches!(
            outcome,
            SplitterResizeOutcome::Changed {
                target_regions: Some(_)
            }
        ));
        assert!(retained_cache.is_none());
        assert_calls(
            app.controller().calls(),
            &["valid:100", "set:100:0:0:250:600"],
        );

        Ok(())
    }

    #[test]
    fn owned_cached_splitter_resize_stale_snapshot_rolls_back_without_positioning()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.split_region(tab_id, region_id, SplitDirection::Vertical, bounds)?;
        let splitter = app
            .hit_test_splitter(tab_id, bounds, 400, 200, 5)?
            .ok_or(AppError::from(DomainError::SplitterNotFound))?;

        let (
            SplitterResizeOutcome::Changed {
                target_regions: Some(cached_regions),
            },
            retained_cache,
        ) = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            None,
        )?
        else {
            return Err(AppError::from(DomainError::RegionNotFound(region_id)));
        };
        assert!(retained_cache.is_none());

        app.controller_mut().mark_mismatched(hwnd);
        app.controller_mut().clear_calls();

        let result = app.resize_splitter_with_owned_cached_regions(
            tab_id,
            splitter.path(),
            bounds,
            250,
            200,
            Some(cached_regions),
        );
        let regions = app.layout_for_tab(tab_id, bounds)?;

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::InvalidWindowHandle))
        ));
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 300, 600)?);
        assert_calls(app.controller().calls(), &["valid:100", "valid:100"]);

        Ok(())
    }

    #[test]
    fn hiding_and_showing_active_tab_use_minimize_visibility_policy() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let region_id = root_region(&app, tab_id, bounds)?;
        let hwnd = WindowHandle::new(100)?;

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        app.hide_active_tab()?;
        assert_calls(app.controller().calls(), &["hide:100"]);

        app.controller_mut().clear_calls();
        let removed_stale = app.show_active_tab(bounds)?;
        assert_eq!(removed_stale, 0);
        assert_calls(
            app.controller().calls(),
            &["valid:100", "show:100:NoActivate", "set:100:0:0:800:600"],
        );

        Ok(())
    }

    #[test]
    fn showing_active_tab_removes_identity_mismatch_and_reports_stale_count() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;
        let replacement_hwnd = WindowHandle::new(300)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().mark_mismatched(left_hwnd);
        app.controller_mut().clear_calls();

        let removed_stale = app.show_active_tab(bounds)?;

        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(removed_stale, 1);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), right_region);
        assert_eq!(placements[0].hwnd(), right_hwnd);
        app.state()
            .workspace()
            .ensure_can_place(tab_id, left_region, replacement_hwnd)?;
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn syncing_active_tab_position_failure_attempts_remaining_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let resized_bounds = Rect::new(0, 0, 1000, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(left_hwnd);
        app.controller_mut().clear_calls();

        let result = app.sync_active_tab(resized_bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(left_hwnd)
        ));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:0:0:500:600",
                "valid:100",
                "valid:200",
                "set:200:500:0:500:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn syncing_active_tab_validation_failure_attempts_remaining_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let resized_bounds = Rect::new(0, 0, 1000, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_validation(left_hwnd);
        app.controller_mut().clear_calls();

        let result = app.sync_active_tab(resized_bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Validate
                    && error.hwnd() == Some(left_hwnd)
        ));
        assert_calls(
            app.controller().calls(),
            &["valid:100", "valid:200", "set:200:500:0:500:600"],
        );

        Ok(())
    }

    #[test]
    fn planned_position_changes_validate_all_rollback_regions_before_targets()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;

        let target_regions = app
            .layout_for_tab(tab_id, bounds)?
            .into_iter()
            .filter(|region| region.region_id() != left_region)
            .collect::<Vec<_>>();
        let rollback_regions = app
            .layout_for_tab(tab_id, bounds)?
            .into_iter()
            .filter(|region| region.region_id() != right_region)
            .collect::<Vec<_>>();
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        let result = App::<RecordingWindowController>::planned_position_changes_from_regions(
            placements,
            &target_regions,
            &rollback_regions,
        );

        assert!(matches!(
            result,
            Err(AppError::Domain(DomainError::RegionNotFound(region_id)))
                if region_id == right_region
        ));

        Ok(())
    }

    #[test]
    fn syncing_active_tab_removes_identity_mismatch_without_positioning_reused_hwnd()
    -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let resized_bounds = Rect::new(0, 0, 1000, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;
        let replacement_hwnd = WindowHandle::new(300)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().mark_mismatched(left_hwnd);
        app.controller_mut().clear_calls();

        let removed_stale = app.sync_active_tab(resized_bounds)?;

        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(removed_stale, 1);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), right_region);
        assert_eq!(placements[0].hwnd(), right_hwnd);
        app.state()
            .workspace()
            .ensure_can_place(tab_id, left_region, replacement_hwnd)?;
        assert_calls(
            app.controller().calls(),
            &["valid:100", "valid:200", "set:200:500:0:500:600"],
        );

        Ok(())
    }

    #[test]
    fn failed_active_tab_sync_keeps_stale_region_placement() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let resized_bounds = Rect::new(0, 0, 1000, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(left_hwnd);
        app.controller_mut().mark_mismatched(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.sync_active_tab(resized_bounds);

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(left_hwnd)
        ));
        let placements = app.state().workspace().placements_for_tab(tab_id)?;
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].region_id(), left_region);
        assert_eq!(placements[0].hwnd(), left_hwnd);
        assert_eq!(placements[1].region_id(), right_region);
        assert_eq!(placements[1].hwnd(), right_hwnd);
        assert_calls(
            app.controller().calls(),
            &["valid:100", "set:100:0:0:500:600", "valid:100", "valid:200"],
        );

        Ok(())
    }

    #[test]
    fn syncing_active_tab_uses_cached_layout_offset_for_window_move() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let moved_bounds = Rect::new(20, 30, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        let cached_regions = app.layout_for_tab(tab_id, bounds)?;
        let mut cached_region_lookup = HashMap::with_capacity(cached_regions.len());
        for region in &cached_regions {
            cached_region_lookup
                .entry(region.region_id())
                .or_insert(region.rect());
        }
        app.controller_mut().clear_calls();

        let report = app.sync_active_tab_with_cached_layout(
            moved_bounds,
            Some(CachedActiveTabLayout::with_region_rects(
                bounds,
                &cached_regions,
                &cached_region_lookup,
            )),
        )?;

        assert_eq!(report.removed_stale_placements(), 0);
        assert!(report.into_computed_regions().is_none());
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "set:100:20:30:400:600",
                "valid:200",
                "set:200:420:30:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn hiding_active_tab_hide_failure_attempts_remaining_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_hide(left_hwnd);
        app.controller_mut().clear_calls();

        let result = app.hide_active_tab();

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::Hide
                    && error.hwnd() == Some(left_hwnd)
        ));
        assert_calls(app.controller().calls(), &["hide:100", "hide:200"]);

        Ok(())
    }

    #[test]
    fn showing_active_tab_position_failure_keeps_shown_windows() -> Result<(), AppError> {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let tab_id = app.add_tab("First")?;
        let left_region = root_region(&app, tab_id, bounds)?;
        let right_region =
            app.split_region(tab_id, left_region, SplitDirection::Vertical, bounds)?;
        let left_hwnd = WindowHandle::new(100)?;
        let right_hwnd = WindowHandle::new(200)?;

        app.place_window(tab_id, left_region, left_hwnd, bounds)?;
        app.place_window(tab_id, right_region, right_hwnd, bounds)?;
        app.controller_mut().fail_position(right_hwnd);
        app.controller_mut().clear_calls();

        let result = app.show_active_tab(bounds);
        let placements = app.state().workspace().placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(AppError::Window(error))
                if error.operation() == WindowOperation::SetPosition
                    && error.hwnd() == Some(right_hwnd)
        ));
        assert_eq!(app.active_tab_id(), Some(tab_id));
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .any(|placement| placement.region_id() == left_region
                    && placement.hwnd() == left_hwnd)
        );
        assert!(placements.iter().any(
            |placement| placement.region_id() == right_region && placement.hwnd() == right_hwnd
        ));
        assert_calls(
            app.controller().calls(),
            &[
                "valid:100",
                "show:100:NoActivate",
                "set:100:0:0:400:600",
                "valid:200",
                "show:200:NoActivate",
                "set:200:400:0:400:600",
            ],
        );

        Ok(())
    }

    #[test]
    fn unregistering_inactive_placement_shows_hidden_window_before_restore() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let _first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let hwnd = WindowHandle::new(200)?;

        app.place_window(second_tab, second_region, hwnd, bounds)?;
        app.controller_mut().clear_calls();

        let status = app.unregister_placement(second_tab, second_region)?;

        assert_eq!(status, UndockStatus::Restored);
        assert_calls(
            app.controller().calls(),
            &["valid:200", "show:200:NoActivate", "restore:200"],
        );

        Ok(())
    }

    #[test]
    fn loaded_settings_do_not_auto_restore_saved_hwnds() -> Result<(), AppError> {
        let tab_id = TabId::new(3);
        let region_id = RegionId::new(4);
        let hwnd = WindowHandle::new(777)?;
        let snapshot = WindowSnapshot::new(
            hwnd,
            Rect::new(10, 20, 300, 200)?,
            WindowDisplayState::Normal,
        );
        let saved_placement = crate::domain::SavedPlacement::new(
            region_id,
            hwnd,
            snapshot,
            crate::domain::SavedWindowRestorePolicy::SessionOnlyNoAutoRestore,
        )?;
        let settings = WorkspaceSettings::new(
            vec![crate::domain::TabSettings::new(
                tab_id,
                "Loaded",
                crate::domain::LayoutNode::single_region(region_id),
                vec![saved_placement],
            )?],
            Some(tab_id),
            4,
            5,
        )?;
        let (state, deferred_placements) =
            AppState::from_settings_layout_only(settings, DEFAULT_MIN_REGION_SIZE)?;
        let mut app = App::with_state(RecordingWindowController::default(), state);

        app.sync_active_tab(Rect::new(0, 0, 800, 600)?)?;

        assert_eq!(deferred_placements, 1);
        assert_eq!(app.active_tab_id(), Some(tab_id));
        assert!(app.controller().calls().is_empty());

        Ok(())
    }

    #[test]
    fn shutdown_reports_missing_windows_and_continues_restoring_remaining() -> Result<(), AppError>
    {
        let controller = RecordingWindowController::default();
        let mut app = App::new(controller);
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_tab = app.add_tab("First")?;
        let second_tab = app.add_tab("Second")?;
        let first_region = root_region(&app, first_tab, bounds)?;
        let second_region = root_region(&app, second_tab, bounds)?;
        let first_hwnd = WindowHandle::new(100)?;
        let second_hwnd = WindowHandle::new(200)?;

        app.place_window(first_tab, first_region, first_hwnd, bounds)?;
        app.place_window(second_tab, second_region, second_hwnd, bounds)?;
        app.controller_mut().mark_invalid(second_hwnd);
        app.controller_mut().clear_calls();

        let report = app.shutdown();

        assert_eq!(report.attempted(), 2);
        assert_eq!(report.restored(), 1);
        assert_eq!(report.missing(), 1);
        assert!(report.failures().is_empty());
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(first_tab)?
                .is_empty()
        );
        assert!(
            app.state()
                .workspace()
                .placements_for_tab(second_tab)?
                .is_empty()
        );
        assert_calls(
            app.controller().calls(),
            &["valid:100", "restore:100", "valid:200"],
        );

        Ok(())
    }
}
