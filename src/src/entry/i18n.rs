use std::borrow::Cow;
use std::fmt::Write as _;

use crate::app::{
    AppError, ShutdownReport, TabDeletionReport, TabSwitchReport, WindowControlError,
    WindowOperation,
};
use crate::domain::{DomainError, RegionId, TabId, UiLanguage};
use crate::infra::SettingsFileError;

use super::{
    CMD_ABOUT, CMD_OPTIONS, CMD_REGION_DELETE, CMD_SPLIT_HORIZONTAL, CMD_SPLIT_VERTICAL,
    CMD_UNDOCK, EntryError, ShutdownSettingsSaveError,
};

pub(super) fn text(
    language: UiLanguage,
    english: &'static str,
    korean: &'static str,
) -> &'static str {
    match language {
        UiLanguage::English => english,
        UiLanguage::Korean => korean,
    }
}

pub(super) fn localized_message<'a>(language: UiLanguage, message: &'a str) -> Cow<'a, str> {
    if language == UiLanguage::Korean {
        return Cow::Borrowed(message);
    }

    let english = match message {
        "외부 윈도우 drop 감지 timer를 시작할 수 없습니다." => {
            "External window drop detection timer could not be started."
        }
        "j3GridDocker window title을 설정할 수 없습니다." => {
            "j3GridDocker window title could not be set."
        }
        "최상위 메뉴를 생성할 수 없습니다." => "Main menu could not be created.",
        "최상위 메뉴를 적용할 수 없습니다." => "Main menu could not be applied.",
        "최상위 메뉴를 숨길 수 없습니다." => "Main menu could not be hidden.",
        "탭 순서 변경 실패: 대상 탭을 찾을 수 없습니다." => {
            "Tab reorder failed: target tab could not be found."
        }
        "옵션 메뉴 위치를 계산할 수 없습니다." => {
            "Options menu position could not be calculated."
        }
        "탭 메뉴 위치를 계산할 수 없습니다." => {
            "Tab menu position could not be calculated."
        }
        "탭바 메뉴 위치를 계산할 수 없습니다." => {
            "Tab bar menu position could not be calculated."
        }
        "영역 메뉴를 열 수 없습니다." => "Region menu could not be opened.",
        "탭바 메뉴를 열 수 없습니다." => "Tab bar menu could not be opened.",
        "탭 메뉴를 열 수 없습니다." => "Tab menu could not be opened.",
        "레이아웃 메뉴 위치를 계산할 수 없습니다." => {
            "Layout menu position could not be calculated."
        }
        "레이아웃 메뉴를 열 수 없습니다." => "Layout menu could not be opened.",
        "레이아웃 메뉴 선택을 처리할 수 없습니다." => {
            "Layout menu selection could not be handled."
        }
        "저장할 탭이 없습니다." => "There is no tab to save.",
        "탭 preset 저장을 취소했습니다." => "Tab preset save was canceled.",
        "탭 preset을 적용할 탭이 없습니다." => {
            "There is no tab to load the tab preset into."
        }
        "탭 preset을 적용할 활성 탭이 없습니다." => {
            "There is no active tab to load the tab preset into."
        }
        "저장된 탭 preset이 없습니다." => "There are no saved tab presets.",
        "탭 preset 목록 위치를 계산할 수 없습니다." => {
            "Tab preset list position could not be calculated."
        }
        "탭 preset 목록 command를 만들 수 없습니다." => {
            "Tab preset list command could not be created."
        }
        "탭 preset 목록을 열 수 없습니다." => "Tab preset list could not be opened.",
        "탭 preset 선택을 처리할 수 없습니다." => {
            "Tab preset selection could not be handled."
        }
        "삭제할 저장된 탭 preset이 없습니다." => {
            "There are no saved tab presets to delete."
        }
        "탭 preset 삭제 목록 위치를 계산할 수 없습니다." => {
            "Tab preset delete list position could not be calculated."
        }
        "탭 preset 삭제 목록 command를 만들 수 없습니다." => {
            "Tab preset delete list command could not be created."
        }
        "탭 preset 삭제 목록을 열 수 없습니다." => {
            "Tab preset delete list could not be opened."
        }
        "탭 preset 삭제 선택을 처리할 수 없습니다." => {
            "Tab preset delete selection could not be handled."
        }
        "탭 preset 적용 실패: 작업 영역 좌표를 계산할 수 없습니다." => {
            "Tab preset load failed: workspace bounds could not be calculated."
        }
        "옵션 메뉴를 열 수 없습니다." => "Options menu could not be opened.",
        "숨김 상태에서도 Dock을 허용합니다." => {
            "Docking while the workspace UI is hidden is enabled."
        }
        "숨김 상태 Dock을 비활성화했습니다." => {
            "Docking while the workspace UI is hidden is disabled."
        }
        "작업 영역 UI를 표시했습니다." => "Workspace UI is now visible.",
        "작업 영역 UI를 숨겼습니다." => "Workspace UI is now hidden.",
        "이름을 변경할 탭 메뉴 대상이 없습니다." => {
            "There is no tab menu target to rename."
        }
        "닫을 탭 메뉴 대상이 없습니다." => "There is no tab menu target to close.",
        "다른 탭을 닫을 탭 메뉴 대상이 없습니다." => {
            "There is no tab menu target for closing other tabs."
        }
        "닫을 다른 탭이 없습니다." => "There are no other tabs to close.",
        "활성 탭이 없습니다." => "There is no active tab.",
        "분할할 영역을 먼저 선택하세요." => "Select a region to split first.",
        "영역을 분할했습니다." => "Region split complete.",
        "삭제할 영역을 먼저 선택하세요." => "Select a region to delete first.",
        "영역을 삭제했습니다." => "Region deleted.",
        "해제할 영역을 먼저 선택하세요." => "Select a region to undock first.",
        "외부 윈도우 배치를 해제했습니다." => {
            "External window placement was undocked."
        }
        "외부 윈도우를 영역에 배치했습니다." => {
            "External window was docked into the region."
        }
        "외부 윈도우를 다른 영역으로 이동했습니다." => {
            "External window was moved to another region."
        }
        "외부 윈도우를 현재 영역에 다시 맞췄습니다." => {
            "External window was fitted to the current region again."
        }
        "외부 윈도우 detach 실패: 현재 위치를 조회할 수 없습니다." => {
            "External window detach failed: current position could not be read."
        }
        "외부 윈도우를 현재 위치에서 배치 해제했습니다." => {
            "External window was undocked at its current position."
        }
        "외부 윈도우가 유효하지 않아 배치 정보를 제거했습니다." => {
            "External window is no longer valid, so the placement was removed."
        }
        "탭 overflow 목록을 열 수 없습니다." => "Tab overflow list could not be opened.",
        "숨겨진 탭이 없습니다." => "There are no hidden tabs.",
        "탭 overflow 목록 위치를 계산할 수 없습니다." => {
            "Tab overflow list position could not be calculated."
        }
        "작업 영역 UI window region 너비를 계산할 수 없습니다." => {
            "Workspace UI window region width could not be calculated."
        }
        "작업 영역 UI window region 상단 좌표를 계산할 수 없습니다." => {
            "Workspace UI window region top coordinate could not be calculated."
        }
        "작업 영역 UI window region 높이를 계산할 수 없습니다." => {
            "Workspace UI window region height could not be calculated."
        }
        "작업 영역 UI window region 너비가 올바르지 않습니다." => {
            "Workspace UI window region width is invalid."
        }
        "작업 영역 UI window region 높이가 올바르지 않습니다." => {
            "Workspace UI window region height is invalid."
        }
        "프로세스 module handle을 가져올 수 없습니다." => {
            "Process module handle could not be retrieved."
        }
        "j3GridDocker window class를 등록할 수 없습니다." => {
            "j3GridDocker window class could not be registered."
        }
        "탭 이름 입력 창을 열 수 없습니다." => {
            "Tab name input window could not be opened."
        }
        "탭 이름 입력 메시지를 가져올 수 없습니다." => {
            "Tab name input message could not be retrieved."
        }
        "입력한 탭 이름을 읽을 수 없습니다." => "Entered tab name could not be read.",
        "탭 이름 입력 창 class를 등록할 수 없습니다." => {
            "Tab name input window class could not be registered."
        }
        "Win32 message를 가져올 수 없습니다." => "Win32 message could not be retrieved.",
        _ => return Cow::Borrowed(message),
    };

    Cow::Borrowed(english)
}

pub(super) fn workspace_ui_toggle_button_label(
    language: UiLanguage,
    workspace_ui_visible: bool,
) -> &'static str {
    match (language, workspace_ui_visible) {
        (UiLanguage::English, true) => "Hide",
        (UiLanguage::English, false) => "Show",
        (UiLanguage::Korean, true) => "숨기기",
        (UiLanguage::Korean, false) => "표시",
    }
}

pub(super) fn workspace_ui_toggle_menu_label(
    language: UiLanguage,
    workspace_ui_visible: bool,
) -> &'static str {
    match (language, workspace_ui_visible) {
        (UiLanguage::English, true) => "Hide Workspace Controls",
        (UiLanguage::English, false) => "Show Workspace Controls",
        (UiLanguage::Korean, true) => "작업 영역 컨트롤 숨기기",
        (UiLanguage::Korean, false) => "작업 영역 컨트롤 표시",
    }
}

pub(super) fn command_button_label(language: UiLanguage, command: u16) -> &'static str {
    match command {
        CMD_SPLIT_VERTICAL => text(language, "Split vertical", "세로 분할"),
        CMD_SPLIT_HORIZONTAL => text(language, "Split horizontal", "가로 분할"),
        CMD_REGION_DELETE => text(language, "Delete region", "영역 삭제"),
        CMD_UNDOCK => text(language, "Undock", "창 해제"),
        CMD_OPTIONS => text(language, "Options", "옵션"),
        CMD_ABOUT => text(language, "About", "정보"),
        _ => "",
    }
}

pub(super) fn startup_saved_workspace_skipped_status_text(
    language: UiLanguage,
    saved_tab_count: usize,
    saved_tab_preset_count: usize,
) -> String {
    match language {
        UiLanguage::English => format!(
            "Started with a new workspace. {saved_tab_count} saved tab(s) were not applied, and {saved_tab_preset_count} tab preset(s) were loaded."
        ),
        UiLanguage::Korean => format!(
            "새 워크스페이스로 시작합니다. 저장된 탭 {saved_tab_count}개는 적용하지 않았고, tab preset {saved_tab_preset_count}개는 불러왔습니다."
        ),
    }
}

pub(super) fn settings_load_failure_status_text(
    language: UiLanguage,
    error: &SettingsFileError,
) -> String {
    match language {
        UiLanguage::English => format!(
            "{} Starting with a new workspace.",
            settings_error_message(language, error)
        ),
        UiLanguage::Korean => format!("{} 새 워크스페이스로 시작합니다.", error.user_message()),
    }
}

pub(super) fn settings_error_message(
    language: UiLanguage,
    error: &SettingsFileError,
) -> &'static str {
    if language == UiLanguage::Korean {
        return error.user_message();
    }

    match error {
        SettingsFileError::ExecutablePath { .. }
        | SettingsFileError::ExecutableDirectoryMissing { .. }
        | SettingsFileError::ExecutableFileNameMissing { .. } => {
            "Settings file path could not be resolved from the executable path."
        }
        SettingsFileError::Io { .. } => "Settings file could not be read or written.",
        SettingsFileError::TomlDeserialize { .. } => "Settings file format could not be parsed.",
        SettingsFileError::TomlSerialize { .. } => "Settings file could not be serialized as TOML.",
        SettingsFileError::InvalidDomain { .. } => "Settings file content is invalid.",
        SettingsFileError::FileTooLarge { .. } => "Settings file size is invalid.",
        SettingsFileError::UnsupportedVersion { .. } => "Settings file version is not supported.",
    }
}

fn domain_error_message(language: UiLanguage, error: &DomainError) -> &'static str {
    if language == UiLanguage::Korean {
        return error.user_message();
    }

    match error {
        DomainError::EmptyTabName => "Tab name cannot be empty.",
        DomainError::EmptyTabPresetName => "Tab preset name cannot be empty.",
        DomainError::EmptyProgramExecutablePath => "Program executable path cannot be empty.",
        DomainError::InvalidProgramArgument => "Program argument contains an invalid character.",
        DomainError::NoActiveTab => "There is no active tab.",
        DomainError::TabPresetNotFound(_) => "Requested tab preset could not be found.",
        DomainError::TabPresetTargetHasPlacements(_) => {
            "Tab preset cannot be applied to a tab with docked external windows."
        }
        DomainError::TabNotFound(_) => "Requested tab could not be found.",
        DomainError::DuplicateTab(_) => "Tab ID already exists.",
        DomainError::RegionNotFound(_) => "Requested region could not be found.",
        DomainError::DuplicateRegion(_) => "Region ID already exists.",
        DomainError::PlacementTabMismatch { .. } => "Placement tab does not match the target tab.",
        DomainError::PlacementNotFound { .. } => {
            "There is no external window placement in the requested region."
        }
        DomainError::RegionAlreadyOccupied(_) => {
            "That region already has an external window docked."
        }
        DomainError::WindowAlreadyPlaced(_) => "That external window is already docked.",
        DomainError::InvalidWindowHandle => "External window is not valid.",
        DomainError::WindowSnapshotMismatch { .. } => {
            "Saved window state does not match the target window."
        }
        DomainError::InvalidRect { .. } => "Region coordinates are invalid.",
        DomainError::InvalidSplitRatio(_) => "Splitter ratio is invalid.",
        DomainError::InvalidSplitPosition { .. } => {
            "Splitter ratio could not be calculated from that position."
        }
        DomainError::InvalidMinimumRegionSize(_) => "Minimum region size is invalid.",
        DomainError::RegionTooSmall { .. } => "Region is too small to calculate splitters.",
        DomainError::SplitterNotFound => "Requested splitter could not be found.",
        DomainError::RootRegionCannotBeDeleted(_) => "Root region cannot be deleted.",
        DomainError::LayoutDepthExceeded { .. } => "Settings layout nesting is too deep.",
        DomainError::CoordinateOverflow => "Region coordinate calculation overflowed.",
        DomainError::IdExhausted(_) => "A new ID could not be generated.",
    }
}

fn window_error_message(language: UiLanguage, error: &WindowControlError) -> String {
    if language == UiLanguage::Korean {
        return error.user_message().to_owned();
    }

    match error.operation() {
        WindowOperation::Validate => "External window state could not be checked.",
        WindowOperation::Snapshot => "External window state could not be captured.",
        WindowOperation::InspectProgram => "External program information could not be captured.",
        WindowOperation::Hide => "External window could not be hidden.",
        WindowOperation::Show => "External window could not be shown.",
        WindowOperation::SetPosition => "External window position could not be changed.",
        WindowOperation::Restore => "External window could not be restored.",
    }
    .to_owned()
}

pub(super) fn app_error_message(language: UiLanguage, error: &AppError) -> String {
    match error {
        AppError::Domain(error) => domain_error_message(language, error).to_owned(),
        AppError::Window(error) => window_error_message(language, error),
    }
}

pub(super) fn shutdown_settings_save_error_message(
    language: UiLanguage,
    error: &ShutdownSettingsSaveError,
) -> String {
    match error {
        ShutdownSettingsSaveError::App(error) => app_error_message(language, error),
        ShutdownSettingsSaveError::Settings(error) => {
            settings_error_message(language, error).to_owned()
        }
    }
}

fn entry_error_message(language: UiLanguage, error: &EntryError) -> String {
    match error {
        EntryError::App(error) => app_error_message(language, error),
        EntryError::Settings(error) => settings_error_message(language, error).to_owned(),
        EntryError::Win32 { user_message, .. } => {
            if language == UiLanguage::Korean {
                (*user_message).to_owned()
            } else {
                localized_message(language, user_message).into_owned()
            }
        }
    }
}

pub(super) fn entry_error_status_text(
    language: UiLanguage,
    operation: &str,
    error: &EntryError,
) -> String {
    if language == UiLanguage::English {
        return format!(
            "{} failed: {}",
            operation,
            entry_error_message(language, error)
        );
    }

    format!("{operation} 실패: {}", entry_error_message(language, error))
}

pub(super) fn tab_operation_error_status_text(
    language: UiLanguage,
    tab_id: TabId,
    operation: &str,
    error: &AppError,
) -> String {
    if language == UiLanguage::English {
        return format!(
            "Tab {} {} failed: {}",
            tab_id.value(),
            operation_label_for(language, operation),
            app_error_message(language, error)
        );
    }

    format!(
        "탭 {} {} 실패: {}",
        tab_id.value(),
        operation_label_for(language, operation),
        app_error_message(language, error)
    )
}

#[cfg(test)]
pub(super) fn docked_window_selection_status_text() -> &'static str {
    docked_window_selection_status_text_for(UiLanguage::Korean)
}

pub(super) fn docked_window_selection_status_text_for(language: UiLanguage) -> &'static str {
    text(
        language,
        "Selected the docked external window region. Drag to an empty region to move it, or outside to undock it.",
        "배치된 외부 윈도우 영역을 선택했습니다. 빈 영역으로 끌면 이동, 바깥으로 끌면 배치 해제합니다.",
    )
}

fn drop_registration_operation_name_for(
    language: UiLanguage,
    source_region_id: Option<RegionId>,
    target_region_id: RegionId,
) -> &'static str {
    match source_region_id {
        None => text(language, "dock", "배치"),
        Some(source_region_id) if source_region_id == target_region_id => {
            text(language, "refit current region", "현재 영역 재맞춤")
        }
        Some(_) => text(language, "move", "이동"),
    }
}

#[cfg(test)]
pub(super) fn drop_registration_error_status_text(
    source_region_id: Option<RegionId>,
    target_region_id: RegionId,
    error: &AppError,
) -> String {
    drop_registration_error_status_text_for(
        UiLanguage::Korean,
        source_region_id,
        target_region_id,
        error,
    )
}

pub(super) fn drop_registration_error_status_text_for(
    language: UiLanguage,
    source_region_id: Option<RegionId>,
    target_region_id: RegionId,
    error: &AppError,
) -> String {
    let operation =
        drop_registration_operation_name_for(language, source_region_id, target_region_id);

    if language == UiLanguage::English {
        return match error {
            AppError::Domain(DomainError::RegionAlreadyOccupied(region_id))
                if *region_id == target_region_id =>
            {
                format!(
                    "External window {operation} failed: target region already has another external window."
                )
            }
            AppError::Domain(DomainError::InvalidWindowHandle) if source_region_id.is_some() => {
                format!(
                    "External window {operation} failed: docked window can no longer be verified."
                )
            }
            AppError::Window(error)
                if source_region_id.is_some()
                    && error.operation() == WindowOperation::SetPosition =>
            {
                format!(
                    "External window {operation} failed: kept the previous region. {}",
                    window_error_message(language, error)
                )
            }
            _ => format!(
                "External window {operation} failed: {}",
                app_error_message(language, error)
            ),
        };
    }

    match error {
        AppError::Domain(DomainError::RegionAlreadyOccupied(region_id))
            if *region_id == target_region_id =>
        {
            format!("외부 윈도우 {operation} 실패: 대상 영역에 이미 다른 외부 윈도우가 있습니다.")
        }
        AppError::Domain(DomainError::InvalidWindowHandle) if source_region_id.is_some() => {
            format!("외부 윈도우 {operation} 실패: 배치된 창을 더 이상 확인할 수 없습니다.")
        }
        AppError::Window(error)
            if source_region_id.is_some() && error.operation() == WindowOperation::SetPosition =>
        {
            format!(
                "외부 윈도우 {operation} 실패: 기존 영역을 유지했습니다. {}",
                error.user_message()
            )
        }
        _ => format!("외부 윈도우 {operation} 실패: {}", error.user_message()),
    }
}

#[cfg(test)]
pub(super) fn tab_deletion_status_text(report: &TabDeletionReport) -> String {
    tab_deletion_status_text_for(UiLanguage::Korean, report)
}

pub(super) fn tab_deletion_status_text_for(
    language: UiLanguage,
    report: &TabDeletionReport,
) -> String {
    if language == UiLanguage::English {
        return format!(
            "Tab {} deleted. Current active tab: {}. {}",
            report.deleted_tab_id().value(),
            active_tab_status_text_for(language, report.current_active_tab()),
            undock_summary_text_for(language, report.undock())
        );
    }

    format!(
        "탭 {} 삭제 완료. 현재 활성 탭: {}. {}",
        report.deleted_tab_id().value(),
        active_tab_status_text(report.current_active_tab()),
        undock_summary_text(report.undock())
    )
}

pub(super) fn tab_rename_success_status_text(language: UiLanguage, tab_id: TabId) -> String {
    match language {
        UiLanguage::English => format!("Tab {} renamed.", tab_id.value()),
        UiLanguage::Korean => format!("탭 {} 이름을 변경했습니다.", tab_id.value()),
    }
}

pub(super) fn tab_rename_cancel_status_text(language: UiLanguage, tab_id: TabId) -> String {
    match language {
        UiLanguage::English => format!("Tab {} rename canceled.", tab_id.value()),
        UiLanguage::Korean => format!("탭 {} 이름 변경을 취소했습니다.", tab_id.value()),
    }
}

pub(super) fn close_other_target_missing_status_text(
    language: UiLanguage,
    target_tab_id: TabId,
) -> String {
    match language {
        UiLanguage::English => format!(
            "Close other tabs failed: base tab {} could not be found.",
            target_tab_id.value()
        ),
        UiLanguage::Korean => format!(
            "Close other tabs 실패: 기준 탭 {}을 찾을 수 없습니다.",
            target_tab_id.value()
        ),
    }
}

pub(super) fn close_other_bounds_failure_status_text(language: UiLanguage) -> &'static str {
    text(
        language,
        "Close other tabs failed: workspace bounds could not be calculated. Undock: not attempted",
        "Close other tabs 실패: 작업 영역 좌표를 계산할 수 없습니다. Undock: 시도하지 않음",
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabStatusLabel {
    pub(super) tab_id: TabId,
    pub(super) name: Option<String>,
}

impl TabStatusLabel {
    pub(super) fn new(tab_id: TabId, name: Option<String>) -> Self {
        Self { tab_id, name }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabSwitchStatusContext {
    pub(super) target: TabStatusLabel,
    pub(super) previous_active: Option<TabStatusLabel>,
}

impl TabSwitchStatusContext {
    fn is_reselecting_active_tab(&self) -> bool {
        self.previous_active
            .as_ref()
            .is_some_and(|previous| previous.tab_id == self.target.tab_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabDeletionStatusContext {
    pub(super) deleted: TabStatusLabel,
    pub(super) previous_active: Option<TabStatusLabel>,
    pub(super) automatic_target: Option<TabStatusLabel>,
}

#[cfg(test)]
pub(super) fn switch_tab_success_status_text(
    context: &TabSwitchStatusContext,
    report: TabSwitchReport,
) -> String {
    switch_tab_success_status_text_for(UiLanguage::Korean, context, report)
}

pub(super) fn switch_tab_success_status_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    report: TabSwitchReport,
) -> String {
    if language == UiLanguage::English {
        let action = if context.is_reselecting_active_tab() {
            "Tab redisplayed"
        } else {
            "Switched tab"
        };
        let mut text = format!(
            "{action}: {}",
            tab_status_label_text_for(language, &context.target)
        );

        if !context.is_reselecting_active_tab() {
            text.push_str(&format!(
                ". Previous active tab: {}",
                optional_tab_status_label_text_for(language, context.previous_active.as_ref())
            ));
        }

        let removed = report.removed_stale_target_placements();
        if removed > 0 {
            text.push_str(&format!(
                ". Removed {removed} invalid target window(s) from placements"
            ));
        }
        let removed = report.removed_stale_previous_placements();
        if removed > 0 {
            text.push_str(&format!(
                ". Removed {removed} invalid previous active window(s) from placements"
            ));
        }

        return text;
    }

    let action = if context.is_reselecting_active_tab() {
        "탭을 다시 표시했습니다"
    } else {
        "탭을 전환했습니다"
    };
    let mut text = format!("{action}: {}", tab_status_label_text(&context.target));

    if !context.is_reselecting_active_tab() {
        text.push_str(&format!(
            ". 이전 활성 탭: {}",
            optional_tab_status_label_text(context.previous_active.as_ref())
        ));
    }

    let removed = report.removed_stale_target_placements();
    if removed > 0 {
        text.push_str(&format!(
            ". 유효하지 않은 대상 창 {removed}개를 배치에서 제거했습니다"
        ));
    }
    let removed = report.removed_stale_previous_placements();
    if removed > 0 {
        text.push_str(&format!(
            ". 유효하지 않은 이전 활성 탭 창 {removed}개를 배치에서 제거했습니다"
        ));
    }

    text
}

#[cfg(test)]
pub(super) fn switch_tab_failure_status_text(
    context: &TabSwitchStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    switch_tab_failure_status_text_for(UiLanguage::Korean, context, current_active, error)
}

pub(super) fn switch_tab_failure_status_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    let operation = switch_tab_failure_operation_text_for(language, context, error);
    let target = tab_status_label_text_for(language, &context.target);
    let result = switch_tab_failure_result_text_for(language, context, current_active, error);

    if language == UiLanguage::English {
        return format!(
            "{operation}: {target}. {result} Cause: {}",
            app_error_message(language, error)
        );
    }

    format!(
        "{operation}: {target}. {result} 원인: {}",
        error.user_message()
    )
}

fn switch_tab_failure_operation_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    error: &AppError,
) -> &'static str {
    if language == UiLanguage::English {
        if context.is_reselecting_active_tab() {
            return "Same tab redisplay failed";
        }

        return match error {
            AppError::Window(error) => match error.operation() {
                WindowOperation::Validate => "Target tab window validation failed",
                WindowOperation::Show => "Target tab window show failed",
                WindowOperation::SetPosition => "Target tab window placement failed",
                WindowOperation::Hide => "Previous tab window hide failed",
                WindowOperation::Snapshot
                | WindowOperation::InspectProgram
                | WindowOperation::Restore => "Window state handling failed during tab switch",
            },
            AppError::Domain(_) => "Tab switch failed",
        };
    }

    if context.is_reselecting_active_tab() {
        return "같은 탭 재표시 실패";
    }

    match error {
        AppError::Window(error) => match error.operation() {
            WindowOperation::Validate => "대상 탭 창 확인 실패",
            WindowOperation::Show => "대상 탭 창 표시 실패",
            WindowOperation::SetPosition => "대상 탭 창 배치 실패",
            WindowOperation::Hide => "이전 탭 창 숨김 실패",
            WindowOperation::Snapshot
            | WindowOperation::InspectProgram
            | WindowOperation::Restore => "탭 전환 중 창 상태 처리 실패",
        },
        AppError::Domain(_) => "탭 전환 실패",
    }
}

fn switch_tab_failure_result_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    let active_result = current_active_result_text_for(language, context, current_active);
    let rollback = match error {
        AppError::Window(error)
            if !context.is_reselecting_active_tab()
                && matches!(
                    error.operation(),
                    WindowOperation::SetPosition | WindowOperation::Hide
                ) =>
        {
            text(
                language,
                " Tried to roll back by hiding the target tab window.",
                " 대상 탭 창 숨김 롤백을 시도했습니다.",
            )
        }
        _ => "",
    };

    format!("{active_result}.{rollback}")
}

fn current_active_result_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    current_active: Option<TabId>,
) -> String {
    if language == UiLanguage::English {
        if context.is_reselecting_active_tab() {
            return match current_active {
                Some(tab_id) if tab_id == context.target.tab_id => {
                    format!(
                        "Kept active tab {}",
                        tab_status_label_text_for(language, &context.target)
                    )
                }
                Some(tab_id) => format!("Current active tab is tab {}", tab_id.value()),
                None => "There is no current active tab".to_owned(),
            };
        }

        return match current_active {
            Some(tab_id)
                if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) =>
            {
                format!(
                    "Kept previous active tab {}",
                    known_tab_status_label_text_for(language, context, tab_id)
                )
            }
            Some(tab_id) if tab_id == context.target.tab_id => {
                format!(
                    "Target tab {} is active",
                    known_tab_status_label_text_for(language, context, tab_id)
                )
            }
            Some(tab_id) => format!("Current active tab is tab {}", tab_id.value()),
            None => "There is no current active tab".to_owned(),
        };
    }

    if context.is_reselecting_active_tab() {
        return match current_active {
            Some(tab_id) if tab_id == context.target.tab_id => {
                format!(
                    "활성 탭 {}을 그대로 유지했습니다",
                    tab_status_label_text(&context.target)
                )
            }
            Some(tab_id) => format!("현재 활성 탭은 탭 {}입니다", tab_id.value()),
            None => "현재 활성 탭은 없습니다".to_owned(),
        };
    }

    match current_active {
        Some(tab_id) if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) => {
            format!(
                "이전 활성 탭 {}을 유지했습니다",
                known_tab_status_label_text(context, tab_id)
            )
        }
        Some(tab_id) if tab_id == context.target.tab_id => {
            format!(
                "대상 탭 {}이 활성 상태입니다",
                known_tab_status_label_text(context, tab_id)
            )
        }
        Some(tab_id) => format!("현재 활성 탭은 탭 {}입니다", tab_id.value()),
        None => "현재 활성 탭은 없습니다".to_owned(),
    }
}

#[cfg(test)]
pub(super) fn tab_deletion_error_status_text(
    context: &TabDeletionStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    tab_deletion_error_status_text_for(UiLanguage::Korean, context, current_active, error)
}

pub(super) fn tab_deletion_error_status_text_for(
    language: UiLanguage,
    context: &TabDeletionStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    if language == UiLanguage::English {
        if let Some(target) = &context.automatic_target
            && is_tab_activation_window_error(error)
        {
            return format!(
                "Automatic switch after tab delete failed: delete target {}, switch target {}. Rolled back the delete; current active tab: {}. Cause: {}. Undock: not completed because of failure",
                tab_status_label_text_for(language, &context.deleted),
                tab_status_label_text_for(language, target),
                tab_deletion_current_active_text_for(language, context, current_active),
                app_error_message(language, error)
            );
        }

        return format!(
            "Tab delete failed: delete target {}. Rolled back the delete; current active tab: {}. Cause: {}. Undock: not completed because of failure",
            tab_status_label_text_for(language, &context.deleted),
            tab_deletion_current_active_text_for(language, context, current_active),
            app_error_message(language, error)
        );
    }

    if let Some(target) = &context.automatic_target
        && is_tab_activation_window_error(error)
    {
        return format!(
            "탭 삭제 후 자동 전환 실패: 삭제 대상 {}, 전환 대상 {}. 삭제를 롤백했고 현재 활성 탭: {}. 원인: {}. Undock: 실패로 완료되지 않음",
            tab_status_label_text(&context.deleted),
            tab_status_label_text(target),
            tab_deletion_current_active_text(context, current_active),
            error.user_message()
        );
    }

    format!(
        "탭 삭제 실패: 삭제 대상 {}. 삭제를 롤백했고 현재 활성 탭: {}. 원인: {}. Undock: 실패로 완료되지 않음",
        tab_status_label_text(&context.deleted),
        tab_deletion_current_active_text(context, current_active),
        error.user_message()
    )
}

fn is_tab_activation_window_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Window(error)
            if matches!(
                error.operation(),
                WindowOperation::Validate | WindowOperation::Show | WindowOperation::SetPosition
            )
    )
}

fn tab_deletion_current_active_text(
    context: &TabDeletionStatusContext,
    current_active: Option<TabId>,
) -> String {
    tab_deletion_current_active_text_for(UiLanguage::Korean, context, current_active)
}

fn tab_deletion_current_active_text_for(
    language: UiLanguage,
    context: &TabDeletionStatusContext,
    current_active: Option<TabId>,
) -> String {
    if language == UiLanguage::English {
        return match current_active {
            Some(tab_id)
                if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) =>
            {
                known_tab_status_label_text_for_deletion_for(language, context, tab_id)
            }
            Some(tab_id)
                if Some(tab_id) == context.automatic_target.as_ref().map(|tab| tab.tab_id) =>
            {
                known_tab_status_label_text_for_deletion_for(language, context, tab_id)
            }
            Some(tab_id) => format!("tab {}", tab_id.value()),
            None => "none".to_owned(),
        };
    }

    match current_active {
        Some(tab_id) if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) => {
            known_tab_status_label_text_for_deletion(context, tab_id)
        }
        Some(tab_id) if Some(tab_id) == context.automatic_target.as_ref().map(|tab| tab.tab_id) => {
            known_tab_status_label_text_for_deletion(context, tab_id)
        }
        Some(tab_id) => format!("탭 {}", tab_id.value()),
        None => "없음".to_owned(),
    }
}

fn tab_status_label_text(label: &TabStatusLabel) -> String {
    tab_status_label_text_for(UiLanguage::Korean, label)
}

fn tab_status_label_text_for(language: UiLanguage, label: &TabStatusLabel) -> String {
    let tab_word = text(language, "tab", "탭");
    match label.name.as_deref() {
        Some(name) => format!("{name} ({tab_word} {})", label.tab_id.value()),
        None => format!("{tab_word} {}", label.tab_id.value()),
    }
}

fn optional_tab_status_label_text(label: Option<&TabStatusLabel>) -> String {
    optional_tab_status_label_text_for(UiLanguage::Korean, label)
}

fn optional_tab_status_label_text_for(
    language: UiLanguage,
    label: Option<&TabStatusLabel>,
) -> String {
    match label {
        Some(label) => tab_status_label_text_for(language, label),
        None => text(language, "none", "없음").to_owned(),
    }
}

fn known_tab_status_label_text(context: &TabSwitchStatusContext, tab_id: TabId) -> String {
    known_tab_status_label_text_for(UiLanguage::Korean, context, tab_id)
}

fn known_tab_status_label_text_for(
    language: UiLanguage,
    context: &TabSwitchStatusContext,
    tab_id: TabId,
) -> String {
    if tab_id == context.target.tab_id {
        return tab_status_label_text_for(language, &context.target);
    }
    if let Some(previous) = &context.previous_active
        && tab_id == previous.tab_id
    {
        return tab_status_label_text_for(language, previous);
    }
    format!("{} {}", text(language, "tab", "탭"), tab_id.value())
}

fn known_tab_status_label_text_for_deletion(
    context: &TabDeletionStatusContext,
    tab_id: TabId,
) -> String {
    known_tab_status_label_text_for_deletion_for(UiLanguage::Korean, context, tab_id)
}

fn known_tab_status_label_text_for_deletion_for(
    language: UiLanguage,
    context: &TabDeletionStatusContext,
    tab_id: TabId,
) -> String {
    if tab_id == context.deleted.tab_id {
        return tab_status_label_text_for(language, &context.deleted);
    }
    if let Some(previous) = &context.previous_active
        && tab_id == previous.tab_id
    {
        return tab_status_label_text_for(language, previous);
    }
    if let Some(target) = &context.automatic_target
        && tab_id == target.tab_id
    {
        return tab_status_label_text_for(language, target);
    }
    format!("{} {}", text(language, "tab", "탭"), tab_id.value())
}

fn active_tab_status_text(tab_id: Option<TabId>) -> String {
    active_tab_status_text_for(UiLanguage::Korean, tab_id)
}

fn active_tab_status_text_for(language: UiLanguage, tab_id: Option<TabId>) -> String {
    match tab_id {
        Some(tab_id) => tab_id.value().to_string(),
        None => text(language, "none", "없음").to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UndockCounts {
    pub(super) attempted: usize,
    pub(super) restored: usize,
    pub(super) missing: usize,
    pub(super) failures: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabOperationFailure {
    pub(super) tab_id: TabId,
    pub(super) operation: &'static str,
    pub(super) message: String,
}

#[cfg(test)]
pub(super) fn close_other_tabs_status_text(
    target_tab_id: TabId,
    total: usize,
    closed_count: usize,
    active_tab: Option<TabId>,
    undock: UndockCounts,
    failures: &[TabOperationFailure],
) -> String {
    close_other_tabs_status_text_for(
        UiLanguage::Korean,
        target_tab_id,
        total,
        closed_count,
        active_tab,
        undock,
        failures,
    )
}

pub(super) fn close_other_tabs_status_text_for(
    language: UiLanguage,
    target_tab_id: TabId,
    total: usize,
    closed_count: usize,
    active_tab: Option<TabId>,
    undock: UndockCounts,
    failures: &[TabOperationFailure],
) -> String {
    if language == UiLanguage::English {
        let base = if failures.is_empty() {
            format!(
                "Close other tabs complete: closed {closed_count} tab(s) except tab {}.",
                target_tab_id.value()
            )
        } else {
            format!(
                "Close other tabs partially failed: kept tab {}, success {closed_count}/{total}. Failures: {}.",
                target_tab_id.value(),
                tab_operation_failures_text_for(language, failures)
            )
        };

        return format!(
            "{base} Current active tab: {}. Undock: attempted {}, restored {}, missing {}, failures {}",
            active_tab_status_text_for(language, active_tab),
            undock.attempted,
            undock.restored,
            undock.missing,
            undock.failures
        );
    }

    let base = if failures.is_empty() {
        format!(
            "Close other tabs 완료: 탭 {} 외 {}개 탭을 닫았습니다.",
            target_tab_id.value(),
            closed_count
        )
    } else {
        format!(
            "Close other tabs 일부 실패: 탭 {} 유지, 성공 {}/{}. 실패: {}.",
            target_tab_id.value(),
            closed_count,
            total,
            tab_operation_failures_text(failures)
        )
    };

    format!(
        "{base} 현재 활성 탭: {}. Undock: attempted {}, restored {}, missing {}, failures {}",
        active_tab_status_text(active_tab),
        undock.attempted,
        undock.restored,
        undock.missing,
        undock.failures
    )
}

#[cfg(test)]
pub(super) fn tab_reorder_status_text(
    tab_id: TabId,
    before_tab_id: Option<TabId>,
    changed: bool,
) -> String {
    tab_reorder_status_text_for(UiLanguage::Korean, tab_id, before_tab_id, changed)
}

pub(super) fn tab_reorder_status_text_for(
    language: UiLanguage,
    tab_id: TabId,
    before_tab_id: Option<TabId>,
    changed: bool,
) -> String {
    let destination = tab_reorder_destination_text_for(language, before_tab_id);
    if language == UiLanguage::English {
        if changed {
            return format!("Tab order changed: tab {} -> {destination}", tab_id.value());
        }

        return format!(
            "Tab order was not changed: tab {} stayed in the same position.",
            tab_id.value()
        );
    }

    if changed {
        format!(
            "탭 순서를 변경했습니다: 탭 {} -> {}",
            tab_id.value(),
            destination
        )
    } else {
        format!(
            "탭 순서를 변경하지 않았습니다: 탭 {} 위치가 그대로입니다.",
            tab_id.value()
        )
    }
}

fn tab_reorder_destination_text_for(language: UiLanguage, before_tab_id: Option<TabId>) -> String {
    match before_tab_id {
        Some(tab_id) => {
            if language == UiLanguage::English {
                format!("before tab {}", tab_id.value())
            } else {
                format!("탭 {} 앞", tab_id.value())
            }
        }
        None => text(language, "last position", "마지막 위치").to_owned(),
    }
}

fn tab_operation_failures_text(failures: &[TabOperationFailure]) -> String {
    tab_operation_failures_text_for(UiLanguage::Korean, failures)
}

fn tab_operation_failures_text_for(
    language: UiLanguage,
    failures: &[TabOperationFailure],
) -> String {
    let mut result = String::new();
    for (index, failure) in failures.iter().enumerate() {
        if index > 0 {
            result.push_str(", ");
        }
        let operation = operation_label_for(language, failure.operation);
        result.push_str(&format!(
            "{} {} {}({})",
            text(language, "tab", "탭"),
            failure.tab_id.value(),
            operation,
            failure.message
        ));
    }
    result
}

fn operation_label_for(language: UiLanguage, operation: &str) -> &'static str {
    if language == UiLanguage::Korean {
        return match operation {
            "이름 변경" => "이름 변경",
            "순서 변경" => "순서 변경",
            "삭제" => "삭제",
            "rename" => "이름 변경",
            "reorder" => "순서 변경",
            "delete" => "삭제",
            _ => "작업",
        };
    }

    match operation {
        "이름 변경" | "rename" => "rename",
        "순서 변경" | "reorder" => "reorder",
        "삭제" | "delete" => "delete",
        _ => "operation",
    }
}

fn undock_summary_text(report: &ShutdownReport) -> String {
    undock_summary_text_for(UiLanguage::Korean, report)
}

pub(super) fn undock_summary_text_for(language: UiLanguage, report: &ShutdownReport) -> String {
    let _ = language;
    format!(
        "Undock: attempted {}, restored {}, missing {}, failures {}",
        report.attempted(),
        report.restored(),
        report.missing(),
        report.failures().len()
    )
}

pub(super) const fn window_maximize_restore_menu_label(
    language: UiLanguage,
    is_maximized: bool,
) -> &'static str {
    match (language, is_maximized) {
        (UiLanguage::English, true) => "Restore window",
        (UiLanguage::English, false) => "Maximize window",
        (UiLanguage::Korean, true) => "창 복원",
        (UiLanguage::Korean, false) => "창 최대화",
    }
}

pub(super) fn write_region_title_text(
    output: &mut String,
    language: UiLanguage,
    region_id: RegionId,
    is_occupied: bool,
) {
    let result = match (language, is_occupied) {
        (UiLanguage::English, true) => write!(output, "Region {} - docked", region_id.value()),
        (UiLanguage::English, false) => write!(output, "Region {}", region_id.value()),
        (UiLanguage::Korean, true) => write!(output, "영역 {} - 배치됨", region_id.value()),
        (UiLanguage::Korean, false) => write!(output, "영역 {}", region_id.value()),
    };
    let _ = result;
}
