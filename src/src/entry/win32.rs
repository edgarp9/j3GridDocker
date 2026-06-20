use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::iter::once;
use std::mem::{size_of, take, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::{Child, Command};
use std::ptr::{null, null_mut};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicIsize, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, GetLastError, HINSTANCE, HWND,
    LPARAM, LRESULT, POINT, RECT, SetLastError, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, ClientToScreen, CreateRectRgn, DEFAULT_GUI_FONT, DT_CENTER, DT_LEFT,
    DeleteObject, EndPaint, GetMonitorInfoW, GetStockObject, GetSysColorBrush, HDC, HGDIOBJ,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
    SetBkMode, SetWindowRgn, TRANSPARENT, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ICC_LINK_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, NM_CLICK, NM_RETURN, NMHDR, WC_LINK,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetAsyncKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_LBUTTON,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, DrawMenuBar, ES_AUTOHSCROLL,
    EnumWindows, GA_ROOT, GW_HWNDPREV, GW_OWNER, GWL_STYLE, GWLP_USERDATA, GetAncestor,
    GetClientRect, GetCursorPos, GetMenu, GetMessageW, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HICON, HMENU, HTCAPTION,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, IsWindow, IsWindowVisible, IsZoomed, KillTimer,
    LR_LOADFROMFILE, LoadCursorW, LoadImageW, MB_ICONINFORMATION, MB_OK, MF_CHECKED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, SIZE_MINIMIZED, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
    SW_SHOW, SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SendMessageW, SetMenu, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_ACTIVATE, WM_APP, WM_CANCELMODE, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOVE, WM_NCCREATE, WM_NCDESTROY,
    WM_NCLBUTTONDOWN, WM_NOTIFY, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFONT, WM_SETICON,
    WM_SIZE, WM_TIMER, WM_WINDOWPOSCHANGED, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_OVERLAPPEDWINDOW,
    WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WindowFromPoint,
};

type HWinEventHook = *mut core::ffi::c_void;
type Win32Handle = *mut core::ffi::c_void;
type WinEventProc = Option<unsafe extern "system" fn(HWinEventHook, u32, HWND, i32, i32, u32, u32)>;

const EVENT_SYSTEM_MOVESIZESTART: u32 = 0x000A;
const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const EVENT_OBJECT_CREATE: u32 = 0x8000;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
const OBJID_WINDOW: i32 = 0;
const CHILDID_SELF: i32 = 0;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const TH32CS_SNAPTHREAD: u32 = 0x00000004;
const MAX_PATH_CHARS: usize = 260;

#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "SetWinEventHook"]
    fn set_win_event_hook(
        event_min: u32,
        event_max: u32,
        event_hook_module: HINSTANCE,
        event_hook: WinEventProc,
        process_id: u32,
        thread_id: u32,
        flags: u32,
    ) -> HWinEventHook;

    #[link_name = "UnhookWinEvent"]
    fn unhook_win_event(event_hook: HWinEventHook) -> i32;

    #[link_name = "EnumThreadWindows"]
    fn enum_thread_windows(
        thread_id: u32,
        enum_func: Option<unsafe extern "system" fn(HWND, LPARAM) -> i32>,
        lparam: LPARAM,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "CreateToolhelp32Snapshot"]
    fn create_toolhelp32_snapshot(flags: u32, process_id: u32) -> Win32Handle;

    #[link_name = "Thread32First"]
    fn thread32_first(snapshot: Win32Handle, entry: *mut ThreadEntry32) -> i32;

    #[link_name = "Thread32Next"]
    fn thread32_next(snapshot: Win32Handle, entry: *mut ThreadEntry32) -> i32;

    #[link_name = "Process32FirstW"]
    fn process32_first(snapshot: Win32Handle, entry: *mut ProcessEntry32) -> i32;

    #[link_name = "Process32NextW"]
    fn process32_next(snapshot: Win32Handle, entry: *mut ProcessEntry32) -> i32;

    #[link_name = "CloseHandle"]
    fn close_handle(handle: Win32Handle) -> i32;
}

use crate::app::{
    App, AppError, AppState, CachedActiveTabLayout, PlacementRegistration, ShutdownReport,
    SplitterResizeOutcome, TabDeletionReport, TabPresetApplication, UndockStatus,
};
#[cfg(test)]
use crate::app::{TabSwitchReport, WindowOperation};
#[cfg(test)]
use crate::domain::DomainError;
use crate::domain::{
    DEFAULT_MIN_REGION_SIZE, ExternalProgramSpec, Rect, RegionId, RegionRect, SplitDirection,
    SplitterPath, SplitterRect, TabId, TabPreset, UiLanguage, WindowHandle, WorkspaceOptions,
};
use crate::infra::{
    PreservedStartupSessionSettings, SettingsFileError, SettingsFileStore, Win32WindowController,
};

#[path = "gdi.rs"]
mod gdi;
#[path = "i18n.rs"]
mod i18n;
#[path = "program_edit_dialog.rs"]
mod program_edit_dialog;
#[path = "shutdown.rs"]
mod shutdown;
#[path = "tooltip.rs"]
mod tooltip;
#[path = "ui.rs"]
mod ui;

use gdi::{PaintBuffer, draw_box, draw_text, draw_text_wide, fill, set_text};
use i18n::{
    TabDeletionStatusContext, TabOperationFailure, TabStatusLabel, TabSwitchStatusContext,
    UndockCounts, app_error_message, close_other_bounds_failure_status_text,
    close_other_tabs_status_text_for, close_other_target_missing_status_text, command_button_label,
    docked_window_selection_status_text_for, drop_registration_error_status_text_for,
    entry_error_status_text, localized_message, settings_error_message,
    settings_load_failure_status_text, shutdown_settings_save_error_message,
    startup_saved_workspace_skipped_status_text, switch_tab_failure_status_text_for,
    switch_tab_success_status_text_for, tab_deletion_error_status_text_for,
    tab_deletion_status_text_for, tab_operation_error_status_text, tab_rename_cancel_status_text,
    tab_rename_success_status_text, tab_reorder_status_text_for, text as ui_text,
    undock_summary_text_for, window_maximize_restore_menu_label, workspace_ui_toggle_button_label,
    workspace_ui_toggle_menu_label, write_region_title_text,
};
#[cfg(test)]
use i18n::{
    about_dialog_text, close_other_tabs_status_text, docked_window_selection_status_text,
    drop_registration_error_status_text, switch_tab_failure_status_text,
    switch_tab_success_status_text, tab_deletion_error_status_text, tab_deletion_status_text,
    tab_reorder_status_text,
};
use program_edit_dialog::prompt_tab_preset_edit;
#[cfg(test)]
use program_edit_dialog::{
    program_edit_dialog_button_bottom, program_edit_dialog_client_height,
    program_edit_dialog_content_height, program_edit_dialog_max_scroll_position,
    program_edit_dialog_min_client_height, program_edit_dialog_viewport_height_for_test,
};
#[cfg(test)]
use shutdown::SettingsSaveMode;
use shutdown::{
    SettingsSavePolicy, ShutdownAttemptReport, ShutdownMode, ShutdownSettingsSaveError,
    ShutdownSettingsSaver, log_undock_failures, shutdown_report_after_settings_save,
    shutdown_report_is_complete,
};
use tooltip::{TabTooltip, TabTooltipSpec, text_from_titles as tab_tooltip_text_from_titles};
use ui::{
    ClientPoint, ScreenPoint, TabHitTarget, TabOverflowHitTarget, TabReorderAutoScroll,
    TabStripLayout, UiRect, hit_test_tab_overflow, hit_test_tab_strip, hit_test_tab_strip_empty,
    layout_bounds_for_client_rect, layout_metrics, layout_rect_to_client_rect, new_tab_button_rect,
    tab_close_button_rect, tab_insertion_target, tab_label_rect, tab_rect_for_index,
    tab_reorder_auto_scroll, tab_strip_layout, toolbar_toggle_rect, top_bar_height,
};

const CLASS_NAME: &str = "j3GridDocker.MainWindow";
const TEXT_INPUT_DIALOG_CLASS_NAME: &str = "j3GridDocker.TextInputDialog";
const ABOUT_DIALOG_CLASS_NAME: &str = "j3GridDocker.AboutDialog";
const SPLITTER_OVERLAY_CLASS_NAME: &str = "j3GridDocker.SplitterOverlay";
const WINDOW_TITLE: &str = "j3GridDocker";
const ABOUT_LINK_URL: &str = "https://github.com/edgarp9";
const APP_ICON_RESOURCE_ID: usize = 1;
const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 700;
const TAB_BAR_HEIGHT: i32 = 34;
const COMMAND_BAR_HEIGHT: i32 = 36;
const STATUS_BAR_HEIGHT: i32 = 24;
const TOOLBAR_TOGGLE_LEFT: i32 = 8;
const TOOLBAR_TOGGLE_WIDTH: i32 = 64;
const TOOLBAR_TOGGLE_GAP: i32 = 8;
const TOP_BAR_NEW_TAB_LEFT: i32 = TOOLBAR_TOGGLE_LEFT + TOOLBAR_TOGGLE_WIDTH + TOOLBAR_TOGGLE_GAP;
const TOP_BAR_NEW_TAB_WIDTH: i32 = 48;
const TAB_BAR_LEFT: i32 = TOP_BAR_NEW_TAB_LEFT + TOP_BAR_NEW_TAB_WIDTH + TOOLBAR_TOGGLE_GAP;
const TAB_DRAG_MOVE_THRESHOLD: i32 = 4;
const TIMER_DROP_POLL: usize = 1;
const TIMER_SPLITTER_OVERLAY_POLL: usize = 2;
const TIMER_TAB_PRESET_PROGRAM_RESTORE: usize = 3;
const DROP_POLL_INTERVAL_MS: u32 = 125;
const SPLITTER_OVERLAY_POLL_INTERVAL_MS: u32 = 50;
const WM_DROP_MOVE_SIZE_EVENT: u32 = WM_APP + 1;
const WM_SPLITTER_OVERLAY_LBUTTONDOWN: u32 = WM_APP + 2;
const WM_WINDOW_NAME_CHANGE_EVENT: u32 = WM_APP + 3;
const WM_TAB_PRESET_PROGRAM_WINDOW_EVENT: u32 = WM_APP + 4;
const DROP_WINDOW_MOVE_THRESHOLD: i32 = 4;
const SPLITTER_HIT_TOLERANCE: i32 = 5;
const MAX_WINDOW_TEXT_CHARS: usize = 32_767;
const ABOUT_DIALOG_WIDTH: i32 = 360;
const ABOUT_DIALOG_HEIGHT: i32 = 170;
const ABOUT_DIALOG_OK_ID: u16 = 1;
const ABOUT_DIALOG_LINK_ID: u16 = 2;

// WinEventProc has no per-hook user data, so keep the required process-global
// route private to the DropMoveEventHook owner.
static DROP_MOVE_EVENT_ROUTER: DropMoveEventRouter = DropMoveEventRouter::new();
static WINDOW_NAME_CHANGE_EVENT_ROUTER: WindowNameChangeEventRouter =
    WindowNameChangeEventRouter::new();
static TAB_PRESET_PROGRAM_WINDOW_EVENT_ROUTER: TabPresetProgramWindowEventRouter =
    TabPresetProgramWindowEventRouter::new();

struct DropMoveEventRouter {
    target_hwnd: AtomicIsize,
}

impl DropMoveEventRouter {
    const fn new() -> Self {
        Self {
            target_hwnd: AtomicIsize::new(0),
        }
    }

    fn publish_target(&self, hwnd: HWND) -> DropMoveEventTargetOwner {
        self.target_hwnd.store(hwnd as isize, Ordering::Relaxed);
        DropMoveEventTargetOwner { hwnd }
    }

    fn clear_target(&self, hwnd: HWND) {
        let _ = self.target_hwnd.compare_exchange(
            hwnd as isize,
            0,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    fn post_move_size_event(&self, event: u32, hwnd: HWND) {
        let target = self.target_hwnd.load(Ordering::Relaxed) as HWND;
        if target.is_null() {
            return;
        }

        unsafe {
            PostMessageW(
                target,
                WM_DROP_MOVE_SIZE_EVENT,
                event as WPARAM,
                hwnd as LPARAM,
            );
        }
    }
}

struct DropMoveEventTargetOwner {
    hwnd: HWND,
}

impl DropMoveEventTargetOwner {
    fn clear(&mut self) {
        if self.hwnd.is_null() {
            return;
        }

        DROP_MOVE_EVENT_ROUTER.clear_target(self.hwnd);
        self.hwnd = null_mut();
    }
}

impl Drop for DropMoveEventTargetOwner {
    fn drop(&mut self) {
        self.clear();
    }
}

struct DropMoveEventHook {
    hook: HWinEventHook,
    target: DropMoveEventTargetOwner,
}

impl DropMoveEventHook {
    fn install(target_hwnd: HWND) -> Option<Self> {
        let target = DROP_MOVE_EVENT_ROUTER.publish_target(target_hwnd);
        let hook = unsafe {
            set_win_event_hook(
                EVENT_SYSTEM_MOVESIZESTART,
                EVENT_SYSTEM_MOVESIZEEND,
                null_mut(),
                Some(drop_move_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_null() {
            drop(target);
            return None;
        }

        Some(Self { hook, target })
    }

    fn uninstall(&mut self) {
        self.target.clear();
        if self.hook.is_null() {
            return;
        }

        let hook = self.hook;
        self.hook = null_mut();
        unsafe {
            unhook_win_event(hook);
        }
    }
}

impl Drop for DropMoveEventHook {
    fn drop(&mut self) {
        self.uninstall();
    }
}

struct WindowNameChangeEventRouter {
    target_hwnd: AtomicIsize,
    interested_hwnd_filter: AtomicUsize,
    interested_hwnds: Mutex<Vec<isize>>,
}

impl WindowNameChangeEventRouter {
    const fn new() -> Self {
        Self {
            target_hwnd: AtomicIsize::new(0),
            interested_hwnd_filter: AtomicUsize::new(0),
            interested_hwnds: Mutex::new(Vec::new()),
        }
    }

    fn publish_target(&self, hwnd: HWND) -> WindowNameChangeEventTargetOwner {
        self.clear_interested_hwnds();
        self.target_hwnd.store(hwnd as isize, Ordering::Relaxed);
        WindowNameChangeEventTargetOwner { hwnd }
    }

    fn clear_target(&self, hwnd: HWND) {
        if self
            .target_hwnd
            .compare_exchange(hwnd as isize, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.clear_interested_hwnds();
        }
    }

    fn replace_interested_hwnds(&self, hwnds: impl IntoIterator<Item = isize>) {
        let mut next = hwnds
            .into_iter()
            .filter(|hwnd| *hwnd != 0)
            .collect::<Vec<_>>();
        next.sort_unstable();
        next.dedup();
        let filter = interested_hwnd_filter(&next);

        self.interested_hwnd_filter
            .store(usize::MAX, Ordering::Relaxed);
        let mut interested = self.lock_interested_hwnds();
        *interested = next;
        self.interested_hwnd_filter.store(filter, Ordering::Relaxed);
    }

    fn clear_interested_hwnds(&self) {
        self.interested_hwnd_filter
            .store(usize::MAX, Ordering::Relaxed);
        let mut interested = self.lock_interested_hwnds();
        interested.clear();
        self.interested_hwnd_filter.store(0, Ordering::Relaxed);
    }

    fn should_post_name_change_event(&self, hwnd: HWND) -> Option<HWND> {
        let target = self.target_hwnd.load(Ordering::Relaxed) as HWND;
        if target.is_null() || hwnd.is_null() {
            return None;
        }

        if !self.contains_interested_hwnd(hwnd as isize) {
            return None;
        }

        Some(target)
    }

    fn contains_interested_hwnd(&self, hwnd: isize) -> bool {
        let bits = hwnd_filter_bits(hwnd);
        if self.interested_hwnd_filter.load(Ordering::Relaxed) & bits != bits {
            return false;
        }

        self.lock_interested_hwnds().binary_search(&hwnd).is_ok()
    }

    fn lock_interested_hwnds(&self) -> MutexGuard<'_, Vec<isize>> {
        match self.interested_hwnds.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn post_name_change_event(&self, hwnd: HWND) {
        let Some(target) = self.should_post_name_change_event(hwnd) else {
            return;
        };

        unsafe {
            PostMessageW(target, WM_WINDOW_NAME_CHANGE_EVENT, 0, hwnd as LPARAM);
        }
    }
}

fn interested_hwnd_filter(hwnds: &[isize]) -> usize {
    hwnds
        .iter()
        .fold(0, |filter, hwnd| filter | hwnd_filter_bits(*hwnd))
}

fn hwnd_filter_bits(hwnd: isize) -> usize {
    let bit_count = usize::BITS as usize;
    let value = hwnd as usize;
    let first = value.wrapping_mul(0x9E37_79B1);
    let second = value.rotate_right(16).wrapping_mul(0x85EB_CA6B);

    (1usize << (first % bit_count)) | (1usize << (second % bit_count))
}

struct WindowNameChangeEventTargetOwner {
    hwnd: HWND,
}

impl WindowNameChangeEventTargetOwner {
    fn clear(&mut self) {
        if self.hwnd.is_null() {
            return;
        }

        WINDOW_NAME_CHANGE_EVENT_ROUTER.clear_target(self.hwnd);
        self.hwnd = null_mut();
    }
}

impl Drop for WindowNameChangeEventTargetOwner {
    fn drop(&mut self) {
        self.clear();
    }
}

struct WindowNameChangeEventHook {
    hook: HWinEventHook,
    target: WindowNameChangeEventTargetOwner,
}

impl WindowNameChangeEventHook {
    fn install(target_hwnd: HWND) -> Option<Self> {
        let target = WINDOW_NAME_CHANGE_EVENT_ROUTER.publish_target(target_hwnd);
        let hook = unsafe {
            set_win_event_hook(
                EVENT_OBJECT_NAMECHANGE,
                EVENT_OBJECT_NAMECHANGE,
                null_mut(),
                Some(window_name_change_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_null() {
            drop(target);
            return None;
        }

        Some(Self { hook, target })
    }

    fn uninstall(&mut self) {
        self.target.clear();
        if self.hook.is_null() {
            return;
        }

        let hook = self.hook;
        self.hook = null_mut();
        unsafe {
            unhook_win_event(hook);
        }
    }
}

impl Drop for WindowNameChangeEventHook {
    fn drop(&mut self) {
        self.uninstall();
    }
}

struct TabPresetProgramWindowEventRouter {
    target_hwnd: AtomicIsize,
}

impl TabPresetProgramWindowEventRouter {
    const fn new() -> Self {
        Self {
            target_hwnd: AtomicIsize::new(0),
        }
    }

    fn publish_target(&self, hwnd: HWND) -> TabPresetProgramWindowEventTargetOwner {
        self.target_hwnd.store(hwnd as isize, Ordering::Relaxed);
        TabPresetProgramWindowEventTargetOwner { hwnd }
    }

    fn clear_target(&self, hwnd: HWND) {
        let _ = self.target_hwnd.compare_exchange(
            hwnd as isize,
            0,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    fn target_for_window_event(&self, hwnd: HWND) -> Option<HWND> {
        let target = self.target_hwnd.load(Ordering::Relaxed) as HWND;
        if target.is_null() || hwnd.is_null() {
            None
        } else {
            Some(target)
        }
    }

    fn post_window_event(&self, hwnd: HWND) {
        let Some(target) = self.target_for_window_event(hwnd) else {
            return;
        };

        unsafe {
            PostMessageW(
                target,
                WM_TAB_PRESET_PROGRAM_WINDOW_EVENT,
                0,
                hwnd as LPARAM,
            );
        }
    }
}

struct TabPresetProgramWindowEventTargetOwner {
    hwnd: HWND,
}

impl TabPresetProgramWindowEventTargetOwner {
    fn clear(&mut self) {
        if self.hwnd.is_null() {
            return;
        }

        TAB_PRESET_PROGRAM_WINDOW_EVENT_ROUTER.clear_target(self.hwnd);
        self.hwnd = null_mut();
    }
}

impl Drop for TabPresetProgramWindowEventTargetOwner {
    fn drop(&mut self) {
        self.clear();
    }
}

struct TabPresetProgramWindowEventHook {
    hooks: Vec<HWinEventHook>,
    target: TabPresetProgramWindowEventTargetOwner,
    process_ids: HashSet<u32>,
}

impl TabPresetProgramWindowEventHook {
    fn install(target_hwnd: HWND, process_ids: impl IntoIterator<Item = u32>) -> Option<Self> {
        if target_hwnd.is_null() {
            return None;
        }

        let target = TAB_PRESET_PROGRAM_WINDOW_EVENT_ROUTER.publish_target(target_hwnd);
        let mut hook = Self {
            hooks: Vec::new(),
            target,
            process_ids: HashSet::new(),
        };
        hook.add_process_ids(process_ids);
        if hook.hooks.is_empty() {
            return None;
        }

        Some(hook)
    }

    fn add_process_ids(&mut self, process_ids: impl IntoIterator<Item = u32>) {
        for process_id in process_ids {
            self.add_process_id(process_id);
        }
    }

    fn add_process_id(&mut self, process_id: u32) {
        if process_id == 0 || self.process_ids.contains(&process_id) {
            return;
        }

        let hook = unsafe {
            set_win_event_hook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_SHOW,
                null_mut(),
                Some(tab_preset_program_window_event_proc),
                process_id,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if !hook.is_null() {
            self.process_ids.insert(process_id);
            self.hooks.push(hook);
        }
    }

    fn uninstall(&mut self) {
        self.target.clear();
        for hook in self.hooks.drain(..) {
            if hook.is_null() {
                continue;
            }

            unsafe {
                unhook_win_event(hook);
            }
        }
    }
}

impl Drop for TabPresetProgramWindowEventHook {
    fn drop(&mut self) {
        self.uninstall();
    }
}

const CMD_TAB_ADD: u16 = 1001;
const CMD_SPLIT_VERTICAL: u16 = 1003;
const CMD_SPLIT_HORIZONTAL: u16 = 1004;
const CMD_REGION_DELETE: u16 = 1005;
const CMD_UNDOCK: u16 = 1006;
const CMD_WORKSPACE_UI_TOGGLE: u16 = 1007;
const CMD_ABOUT: u16 = 1008;
const CMD_TAB_RENAME_CONTEXT: u16 = 1009;
const CMD_TAB_CLOSE_CONTEXT: u16 = 1010;
const CMD_TAB_CLOSE_OTHER_CONTEXT: u16 = 1011;
const CMD_OPTIONS: u16 = 1015;
const CMD_DOCK_HIDDEN_WORKSPACE_UI_TOGGLE: u16 = 1016;
const CMD_WINDOW_MINIMIZE: u16 = 1018;
const CMD_WINDOW_MAXIMIZE_RESTORE: u16 = 1019;
const CMD_WINDOW_CLOSE: u16 = 1020;
const CMD_LANGUAGE_ENGLISH: u16 = 1022;
const CMD_LANGUAGE_KOREAN: u16 = 1023;
const CMD_TAB_PRESET_SAVE: u16 = 1024;
const CMD_TAB_PRESET_LOAD: u16 = 1025;
const CMD_TAB_PRESET_DELETE: u16 = 1026;
const CMD_TAB_PRESET_EDIT: u16 = 1027;
const CMD_TAB_OVERFLOW_BASE: u16 = 2000;
const CMD_TAB_OVERFLOW_END: u16 = 3000;
const CMD_TAB_PRESET_BASE: u16 = 3000;
const CMD_TAB_PRESET_END: u16 = 4000;
const CMD_TAB_PRESET_DELETE_BASE: u16 = 4000;
const CMD_TAB_PRESET_DELETE_END: u16 = 5000;
const CMD_TAB_PRESET_EDIT_BASE: u16 = 5000;
const CMD_TAB_PRESET_EDIT_END: u16 = 6000;
const TAB_PRESET_WINDOW_WAIT: Duration = Duration::from_secs(10);
const TAB_PRESET_WINDOW_POLL: Duration = Duration::from_millis(100);
const TAB_PRESET_DEADLINE_RESCAN_SUPPRESSION: Duration = Duration::from_millis(500);
const TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL: Duration = Duration::from_secs(1);
const TAB_PRESET_PROCESS_TREE_MAX_SCAN_INTERVAL: Duration = Duration::from_secs(4);
const TAB_PRESET_HOOKED_PROCESS_TREE_SCAN_INTERVAL: Duration = Duration::from_secs(2);
const TAB_PRESET_HOOKED_PROCESS_TREE_MAX_SCAN_INTERVAL: Duration = Duration::from_secs(4);
const TAB_PRESET_WINDOW_SCAN_INTERVAL: Duration = Duration::from_secs(1);
const TAB_PRESET_WINDOW_MAX_SCAN_INTERVAL: Duration = Duration::from_secs(4);
const TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const TAB_PRESET_HOOKED_WINDOW_MAX_SCAN_INTERVAL: Duration = Duration::from_secs(4);
const TAB_PRESET_THREAD_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(2);
const TAB_PRESET_THREAD_SNAPSHOT_MAX_INTERVAL: Duration = Duration::from_secs(4);
const TAB_PRESET_HOOKED_THREAD_SNAPSHOT_DELAY: Duration = Duration::from_secs(6);

const INPUT_DIALOG_OK_ID: u16 = 1;
const INPUT_DIALOG_CANCEL_ID: u16 = 2;
const INPUT_DIALOG_EDIT_ID: u16 = 100;
const TEXT_INPUT_DIALOG_WIDTH: i32 = 380;
const TEXT_INPUT_DIALOG_HEIGHT: i32 = 170;

static TAB_CLOSE_LABEL: [u16; 2] = [b'X' as u16, 0];
static TAB_OVERFLOW_DROPDOWN_LABEL: [u16; 4] = [b'.' as u16, b'.' as u16, b'.' as u16, 0];

const COMMAND_BUTTON_LEFT: i32 = 8;
const COMMAND_BUTTON_GAP: i32 = 6;
const COMMAND_BUTTON_RIGHT_MARGIN: i32 = 8;

const BUTTON_SPECS: [ButtonSpec; 4] = [
    ButtonSpec {
        command: CMD_SPLIT_VERTICAL,
        width: 112,
    },
    ButtonSpec {
        command: CMD_SPLIT_HORIZONTAL,
        width: 128,
    },
    ButtonSpec {
        command: CMD_REGION_DELETE,
        width: 112,
    },
    ButtonSpec {
        command: CMD_UNDOCK,
        width: 88,
    },
];

const COLOR_BG: COLORREF = 0x00F8F8F8;
const COLOR_TOP: COLORREF = 0x00ECE7DF;
const COLOR_TAB_ACTIVE: COLORREF = 0x00FFFFFF;
const COLOR_TAB_INACTIVE: COLORREF = 0x00D8D8D8;
const COLOR_BUTTON: COLORREF = 0x00FFFFFF;
const COLOR_BUTTON_DISABLED: COLORREF = 0x00E8E8E8;
const COLOR_BUTTON_BORDER: COLORREF = 0x00808080;
const COLOR_REGION: COLORREF = 0x00FFFFFF;
const COLOR_REGION_ACTIVE: COLORREF = 0x00E6F2FF;
const COLOR_REGION_OCCUPIED: COLORREF = 0x00ECF7EC;
const COLOR_REGION_BORDER: COLORREF = 0x00606060;
const COLOR_SPLITTER: COLORREF = 0x00909090;
const COLOR_STATUS: COLORREF = 0x00F0F0F0;
const COLOR_TEXT: COLORREF = 0x00202020;
const COLOR_TAB_INSERTION: COLORREF = 0x00C05000;

pub fn run() -> Result<(), EntryError> {
    let hinstance = module_handle()?;
    register_window_class(hinstance)?;
    register_splitter_overlay_class(hinstance)?;

    let mut state = Box::new(MainWindow::new()?);
    let class_name = wide_null(CLASS_NAME);
    let title = wide_null(WINDOW_TITLE);
    let state_ptr = state.as_mut() as *mut MainWindow;

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            DEFAULT_WIDTH,
            DEFAULT_HEIGHT,
            null_mut(),
            null_mut(),
            hinstance,
            state_ptr.cast(),
        )
    };

    if hwnd.is_null() {
        return Err(EntryError::win32(
            "CreateWindowExW",
            "j3GridDocker main window를 생성할 수 없습니다.",
        ));
    }

    state.owned_by_window = true;
    let _owned_by_window = Box::into_raw(state);

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    message_loop()
}

#[derive(Debug)]
pub enum EntryError {
    App(AppError),
    Settings(SettingsFileError),
    Win32 {
        api: &'static str,
        last_error: u32,
        user_message: &'static str,
    },
}

impl EntryError {
    pub fn user_message(&self) -> &str {
        match self {
            Self::App(error) => error.user_message(),
            Self::Settings(error) => error.user_message(),
            Self::Win32 { user_message, .. } => user_message,
        }
    }

    fn win32(api: &'static str, user_message: &'static str) -> Self {
        Self::Win32 {
            api,
            last_error: unsafe { GetLastError() },
            user_message,
        }
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::App(error) => write!(formatter, "{error}"),
            Self::Settings(error) => write!(formatter, "{error}"),
            Self::Win32 {
                api, last_error, ..
            } => write!(formatter, "{api} failed with GetLastError={last_error}"),
        }
    }
}

impl Error for EntryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::App(error) => Some(error),
            Self::Settings(error) => Some(error),
            Self::Win32 { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Win32StatusFailure {
    api: &'static str,
    last_error: u32,
    user_message: &'static str,
}

impl Win32StatusFailure {
    fn new(api: &'static str, last_error: u32, user_message: &'static str) -> Self {
        Self {
            api,
            last_error,
            user_message,
        }
    }
}

impl From<AppError> for EntryError {
    fn from(value: AppError) -> Self {
        Self::App(value)
    }
}

impl From<SettingsFileError> for EntryError {
    fn from(value: SettingsFileError) -> Self {
        Self::Settings(value)
    }
}

fn log_settings_load_error(error: &SettingsFileError) {
    eprintln!("{error}");
    if let Some(source) = error.source() {
        eprintln!("cause: {source}");
    }
}

struct SplitterDragLayoutCache {
    tab_id: TabId,
    bounds: Rect,
    regions: Vec<RegionRect>,
}

struct SplitterOverlayRectCache {
    tab_id: TabId,
    bounds: Rect,
    rects: Vec<Rect>,
}

struct ActiveTabSyncCache {
    tab_id: TabId,
    bounds: Rect,
    regions: Vec<RegionRect>,
    rects_by_region_id: HashMap<RegionId, Rect>,
}

impl ActiveTabSyncCache {
    fn new(tab_id: TabId, bounds: Rect, regions: Vec<RegionRect>) -> Self {
        let mut rects_by_region_id = HashMap::with_capacity(regions.len());
        for region in &regions {
            rects_by_region_id
                .entry(region.region_id())
                .or_insert(region.rect());
        }

        Self {
            tab_id,
            bounds,
            regions,
            rects_by_region_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabTooltipSyncKey {
    layout: TabTooltipSyncLayoutKey,
    tabs: Vec<TabTooltipSyncKeySpec>,
}

impl TabTooltipSyncKey {
    fn new(layout: TabTooltipSyncLayoutKey, tab_capacity: usize) -> Self {
        Self {
            layout,
            tabs: Vec::with_capacity(tab_capacity),
        }
    }

    fn matches_layout(&self, layout: TabTooltipSyncLayoutKey) -> bool {
        self.layout == layout
    }

    fn contains_window(&self, hwnd: isize) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.placement_hwnds.contains(&hwnd))
    }

    #[cfg(test)]
    fn tabs_for_window(&self, hwnd: isize) -> impl Iterator<Item = (TabId, UiRect)> + '_ {
        self.tabs
            .iter()
            .filter(move |tab| tab.placement_hwnds.contains(&hwnd))
            .map(|tab| (tab.tab_id, tab.rect))
    }

    fn window_hwnds(&self) -> impl Iterator<Item = isize> + '_ {
        self.tabs
            .iter()
            .flat_map(|tab| tab.placement_hwnds.iter().copied())
    }

    fn tooltip_sync_specs_for_window_title_change(
        &mut self,
        hwnd: isize,
        title: Option<String>,
    ) -> Vec<(TabId, Option<TabTooltipSpec>)> {
        let mut specs = Vec::new();
        let title = title.as_deref();
        for tab in &mut self.tabs {
            if !tab.update_window_title(hwnd, title) {
                continue;
            }

            specs.push((tab.tab_id, tab.tooltip_spec()));
        }

        specs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabTooltipSyncLayoutKey {
    language: UiLanguage,
    workspace_generation: u64,
    client: UiRect,
    tab_count: usize,
    first_visible_index: usize,
    visible_end_index: usize,
}

impl TabTooltipSyncLayoutKey {
    fn new(
        language: UiLanguage,
        workspace_generation: u64,
        client: UiRect,
        tab_count: usize,
        first_visible_index: usize,
        visible_end_index: usize,
    ) -> Self {
        Self {
            language,
            workspace_generation,
            client,
            tab_count,
            first_visible_index,
            visible_end_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabTooltipSyncKeySpec {
    tab_id: TabId,
    rect: UiRect,
    placement_hwnds: Vec<isize>,
    placement_titles: Vec<Option<String>>,
}

impl TabTooltipSyncKeySpec {
    fn new(
        tab_id: TabId,
        rect: UiRect,
        placement_hwnds: Vec<isize>,
        placement_titles: Vec<Option<String>>,
    ) -> Self {
        debug_assert_eq!(placement_hwnds.len(), placement_titles.len());
        Self {
            tab_id,
            rect,
            placement_hwnds,
            placement_titles,
        }
    }

    fn update_window_title(&mut self, hwnd: isize, title: Option<&str>) -> bool {
        let mut updated = false;
        for (placement_hwnd, placement_title) in self
            .placement_hwnds
            .iter()
            .zip(self.placement_titles.iter_mut())
        {
            if *placement_hwnd != hwnd {
                continue;
            }

            *placement_title = title.map(str::to_owned);
            updated = true;
        }

        updated
    }

    fn tooltip_spec(&self) -> Option<TabTooltipSpec> {
        self.tooltip_text().map(|text| TabTooltipSpec {
            tab_id: self.tab_id,
            rect: self.rect,
            text,
        })
    }

    fn tooltip_text(&self) -> Option<String> {
        tab_tooltip_text_from_titles(self.placement_titles.iter().flatten().cloned())
    }

    #[cfg(test)]
    fn matches_windows(
        &self,
        tab_id: TabId,
        rect: UiRect,
        hwnds: impl IntoIterator<Item = WindowHandle>,
    ) -> bool {
        if self.tab_id != tab_id || self.rect != rect {
            return false;
        }

        let mut expected = self.placement_hwnds.iter();
        for hwnd in hwnds {
            let Some(expected_hwnd) = expected.next() else {
                return false;
            };
            if *expected_hwnd != hwnd.raw() {
                return false;
            }
        }

        expected.next().is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirtyPaintSections {
    top_bar: bool,
    tab_strip: bool,
    command_buttons: bool,
    workspace_regions: bool,
    status_bar: bool,
}

impl DirtyPaintSections {
    fn for_dirty(client: UiRect, dirty: UiRect, workspace_ui_visible: bool) -> Self {
        Self {
            top_bar: optional_rect_overlaps(
                top_bar_rect_for_client(client, workspace_ui_visible),
                dirty,
            ),
            tab_strip: optional_rect_overlaps(
                tab_strip_rect_for_client(client, workspace_ui_visible),
                dirty,
            ),
            command_buttons: workspace_ui_visible
                && optional_rect_overlaps(command_buttons_rect_for_client(client), dirty),
            workspace_regions: workspace_ui_visible
                && optional_rect_overlaps(
                    workspace_body_rect_for_client(client, workspace_ui_visible),
                    dirty,
                ),
            status_bar: workspace_ui_visible
                && optional_rect_overlaps(
                    status_bar_rect_for_client(client, workspace_ui_visible),
                    dirty,
                ),
        }
    }
}

fn rects_overlap(left: UiRect, right: UiRect) -> bool {
    left.intersect(right).is_some()
}

fn optional_rect_overlaps(rect: Option<UiRect>, dirty: UiRect) -> bool {
    rect.map(|rect| rects_overlap(rect, dirty)).unwrap_or(false)
}

#[derive(Default)]
struct SplitterOverlayController {
    windows: Vec<HWND>,
    visible_count: usize,
    visible_rects: Vec<Rect>,
}

impl SplitterOverlayController {
    fn sync(&mut self, owner: HWND, rects: &[Rect]) -> Result<bool, Win32StatusFailure> {
        if owner.is_null() || rects.is_empty() {
            self.hide_all();
            return Ok(false);
        }

        self.ensure_count(owner, rects.len())?;

        for (index, rect) in rects.iter().enumerate() {
            if self.visible_rect_at(index) == Some(*rect) {
                continue;
            }

            let hwnd = self.windows[index];
            let ok = unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left(),
                    rect.top(),
                    rect.width(),
                    rect.height(),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
            if ok == 0 {
                let last_error = unsafe { GetLastError() };
                return Err(Win32StatusFailure::new(
                    "SetWindowPos",
                    last_error,
                    "splitter overlay 위치를 갱신할 수 없습니다.",
                ));
            }
            self.remember_visible_rect(index, *rect);
        }

        for hwnd in self
            .windows
            .iter()
            .skip(rects.len())
            .take(self.visible_count.saturating_sub(rects.len()))
        {
            unsafe {
                ShowWindow(*hwnd, SW_HIDE);
            }
        }
        self.visible_count = rects.len();
        self.visible_rects.truncate(rects.len());

        Ok(true)
    }

    fn hide_all(&mut self) {
        for hwnd in self.windows.iter().take(self.visible_count) {
            unsafe {
                ShowWindow(*hwnd, SW_HIDE);
            }
        }
        self.visible_count = 0;
        self.visible_rects.clear();
    }

    fn destroy_all(&mut self) {
        self.visible_count = 0;
        self.visible_rects.clear();
        for hwnd in self.windows.drain(..) {
            if !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0 {
                unsafe {
                    DestroyWindow(hwnd);
                }
            }
        }
    }

    fn ensure_count(&mut self, owner: HWND, count: usize) -> Result<(), Win32StatusFailure> {
        while self.windows.len() < count {
            self.windows.push(create_splitter_overlay_window(owner)?);
        }

        Ok(())
    }

    fn visible_rect_at(&self, index: usize) -> Option<Rect> {
        if index < self.visible_count {
            self.visible_rects.get(index).copied()
        } else {
            None
        }
    }

    fn remember_visible_rect(&mut self, index: usize, rect: Rect) {
        if self.visible_rects.len() <= index {
            self.visible_rects.resize(index + 1, rect);
        }
        self.visible_rects[index] = rect;
        self.visible_count = self.visible_count.max(index + 1);
    }
}

fn top_bar_rect_for_client(client: UiRect, workspace_ui_visible: bool) -> Option<UiRect> {
    UiRect::new(0, 0, client.width(), top_bar_height(workspace_ui_visible)).intersect(client)
}

fn tab_strip_rect_for_client(client: UiRect, workspace_ui_visible: bool) -> Option<UiRect> {
    UiRect::new(
        0,
        0,
        client.width(),
        TAB_BAR_HEIGHT.min(top_bar_height(workspace_ui_visible)),
    )
    .intersect(client)
}

fn visit_command_button_rects(client: UiRect, mut visit: impl FnMut(usize, ButtonRect) -> bool) {
    let top = TAB_BAR_HEIGHT + 4;
    let bottom = top_bar_height(true) - 4;
    if bottom <= top {
        return;
    }

    let max_right = client.right.saturating_sub(COMMAND_BUTTON_RIGHT_MARGIN);
    let mut left = COMMAND_BUTTON_LEFT.max(client.left);

    for (index, spec) in BUTTON_SPECS.iter().enumerate() {
        let Some(right) = left.checked_add(spec.width) else {
            break;
        };
        if right > max_right || right <= left {
            break;
        }

        let button = ButtonRect {
            command: spec.command,
            rect: UiRect::new(left, top, right, bottom),
        };
        if !visit(index, button) {
            break;
        }

        let Some(next_left) = right.checked_add(COMMAND_BUTTON_GAP) else {
            break;
        };
        left = next_left;
    }
}

fn command_buttons_rect_for_client(client: UiRect) -> Option<UiRect> {
    let mut bounds: Option<UiRect> = None;
    visit_command_button_rects(client, |_, button| {
        bounds = Some(match bounds {
            Some(existing) => existing.union(button.rect),
            None => button.rect,
        });
        true
    });
    bounds
}

fn workspace_body_rect_for_client(client: UiRect, workspace_ui_visible: bool) -> Option<UiRect> {
    let metrics = layout_metrics(client, workspace_ui_visible)?;
    UiRect::new(
        0,
        metrics.content_top,
        metrics.width,
        metrics.content_top + metrics.height,
    )
    .intersect(client)
}

fn status_bar_rect_for_client(client: UiRect, workspace_ui_visible: bool) -> Option<UiRect> {
    if !workspace_ui_visible {
        return None;
    }

    UiRect::new(
        0,
        client.bottom - STATUS_BAR_HEIGHT,
        client.width(),
        client.bottom,
    )
    .intersect(client)
}

#[derive(Default)]
struct MainWindowFrameState {
    is_minimized: bool,
    active_tab_show_pending: bool,
    main_menu_maximized: Option<bool>,
}

impl MainWindowFrameState {
    fn is_minimized(&self) -> bool {
        self.is_minimized
    }

    fn mark_minimized(&mut self) -> bool {
        if self.is_minimized {
            return false;
        }

        self.is_minimized = true;
        self.active_tab_show_pending = true;
        true
    }

    fn mark_restored_or_resized(&mut self) {
        let was_minimized = self.is_minimized;
        self.is_minimized = false;
        if was_minimized {
            self.active_tab_show_pending = true;
        }
    }

    fn active_tab_show_pending(&self) -> bool {
        self.active_tab_show_pending
    }

    fn complete_active_tab_show(&mut self) {
        self.active_tab_show_pending = false;
    }

    fn active_tab_hidden_for_shutdown(&self) -> bool {
        self.is_minimized || self.active_tab_show_pending
    }

    fn main_menu_maximized(&self) -> Option<bool> {
        self.main_menu_maximized
    }

    fn clear_main_menu_size_cache(&mut self) {
        self.main_menu_maximized = None;
    }

    fn cache_main_menu_size(&mut self, is_maximized: bool) {
        self.main_menu_maximized = Some(is_maximized);
    }
}

struct MainWindowPaintState {
    buffer: PaintBuffer,
    toolbar_toggle_label: WideText,
    new_tab_label: WideText,
    button_labels: Vec<WideText>,
    tab_labels: Vec<CachedTabLabel>,
    layout_cache: PaintLayoutCache,
    layout_regions: Vec<RegionRect>,
    layout_splitters: Vec<SplitterRect>,
    occupied_regions: HashSet<RegionId>,
    occupied_tab_id: Option<TabId>,
}

impl MainWindowPaintState {
    fn new() -> Self {
        Self {
            buffer: PaintBuffer::new(),
            toolbar_toggle_label: WideText::new(""),
            new_tab_label: WideText::new(""),
            button_labels: Vec::new(),
            tab_labels: Vec::new(),
            layout_cache: PaintLayoutCache::default(),
            layout_regions: Vec::new(),
            layout_splitters: Vec::new(),
            occupied_regions: HashSet::new(),
            occupied_tab_id: None,
        }
    }
}

struct MainMenuPresentation {
    language: UiLanguage,
    workspace_ui_visible: bool,
    workspace_options: WorkspaceOptions,
    is_maximized: bool,
}

impl MainMenuPresentation {
    fn append_view_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        append_menu(
            menu,
            CMD_WORKSPACE_UI_TOGGLE,
            workspace_ui_toggle_menu_label(self.language, self.workspace_ui_visible),
        );
        if append_submenu(root, menu, ui_text(self.language, "View", "보기")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_options_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        append_checked_menu(
            menu,
            CMD_DOCK_HIDDEN_WORKSPACE_UI_TOGGLE,
            ui_text(
                self.language,
                "Dock While Workspace Controls Are Hidden",
                "작업 영역 컨트롤 숨김 중 Dock",
            ),
            self.workspace_options.dock_hidden_workspace_ui(),
        );

        let Some(language_menu) = create_menu_handle() else {
            unsafe {
                DestroyMenu(menu);
            }
            return false;
        };
        append_checked_menu(
            language_menu,
            CMD_LANGUAGE_ENGLISH,
            "English",
            self.language == UiLanguage::English,
        );
        append_checked_menu(
            language_menu,
            CMD_LANGUAGE_KOREAN,
            "Korean",
            self.language == UiLanguage::Korean,
        );
        if !append_submenu(menu, language_menu, "Language") {
            unsafe {
                DestroyMenu(language_menu);
                DestroyMenu(menu);
            }
            return false;
        }

        if append_submenu(root, menu, ui_text(self.language, "Options", "옵션")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_window_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        append_menu(
            menu,
            CMD_WINDOW_MINIMIZE,
            ui_text(self.language, "Minimize", "최소화"),
        );
        append_menu(
            menu,
            CMD_WINDOW_MAXIMIZE_RESTORE,
            window_maximize_restore_menu_label(self.language, self.is_maximized),
        );
        append_menu(
            menu,
            CMD_WINDOW_CLOSE,
            ui_text(self.language, "Close Window", "창 닫기"),
        );
        if append_submenu(root, menu, ui_text(self.language, "Window", "창")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_help_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        append_menu(
            menu,
            CMD_ABOUT,
            ui_text(self.language, "About j3GridDocker", "j3GridDocker 정보"),
        );
        if append_submenu(root, menu, ui_text(self.language, "Help", "도움말")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }
}

struct TabOverflowPopupMenu {
    menu: HMENU,
    command_tabs: Vec<(u16, TabId)>,
    next_menu_index: usize,
}

impl TabOverflowPopupMenu {
    fn new() -> Option<Self> {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return None;
        }

        Some(Self {
            menu,
            command_tabs: Vec::new(),
            next_menu_index: 0,
        })
    }

    fn append_hidden_tab(&mut self, tab_index: usize, tab_id: TabId, tab_name: &str) -> bool {
        let Some(command) = tab_overflow_command_for_index(self.next_menu_index) else {
            return false;
        };

        append_menu(
            self.menu,
            command,
            &format!("{} {}", tab_index + 1, tab_name),
        );
        self.command_tabs.push((command, tab_id));
        self.next_menu_index = self.next_menu_index.saturating_add(1);
        true
    }

    fn is_empty(&self) -> bool {
        self.command_tabs.is_empty()
    }

    fn select_tab(mut self, hwnd: HWND, screen_point: ScreenPoint) -> Option<TabId> {
        let selected = unsafe {
            TrackPopupMenu(
                self.menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                hwnd,
                null(),
            )
        };
        self.destroy_menu();

        let selected = popup_selected_command(selected, hwnd)?;
        take(&mut self.command_tabs)
            .into_iter()
            .find_map(|(command, tab_id)| {
                if command == selected {
                    Some(tab_id)
                } else {
                    None
                }
            })
    }

    fn destroy_menu(&mut self) {
        if self.menu.is_null() {
            return;
        }

        let menu = self.menu;
        self.menu = null_mut();
        unsafe {
            DestroyMenu(menu);
        }
    }
}

impl Drop for TabOverflowPopupMenu {
    fn drop(&mut self) {
        self.destroy_menu();
    }
}

struct MainWindow {
    app: App<Win32WindowController>,
    settings_store: SettingsFileStore,
    hwnd: HWND,
    active_region: Option<RegionId>,
    dragging_splitter: Option<SplitterPath>,
    last_splitter_drag_screen_point: Option<(i32, i32)>,
    splitter_drag_layout_cache: Option<SplitterDragLayoutCache>,
    drop_tracker: DropTracker,
    drop_poll_timer_active: bool,
    drop_move_event_hook: Option<DropMoveEventHook>,
    window_name_change_event_hook: Option<WindowNameChangeEventHook>,
    splitter_overlay: SplitterOverlayController,
    splitter_overlay_poll_timer_active: bool,
    splitter_overlay_rect_cache: Option<SplitterOverlayRectCache>,
    pending_tab_click: Option<PendingTabClick>,
    tab_reorder_drag: Option<TabReorderDrag>,
    tab_context_target: Option<TabId>,
    tab_overflow_first_visible_index: usize,
    tab_tooltip: TabTooltip,
    tab_tooltip_sync_key: Option<TabTooltipSyncKey>,
    workspace_change_generation: u64,
    status: WideText,
    next_tab_number: u32,
    shutdown_done: bool,
    settings_save_policy: SettingsSavePolicy,
    preserved_startup_session: Option<PreservedStartupSessionSettings>,
    workspace_options: WorkspaceOptions,
    frame_state: MainWindowFrameState,
    workspace_ui_visibility: WorkspaceUiVisibility,
    paint: MainWindowPaintState,
    active_tab_sync_cache: Option<ActiveTabSyncCache>,
    tab_preset_program_restore: Option<TabPresetProgramRestoreState>,
    tab_preset_program_restore_timer_active: bool,
    icons: Vec<HICON>,
    owned_by_window: bool,
    active_message_handlers: u32,
    destroy_pending: bool,
}

impl MainWindow {
    fn new() -> Result<Self, EntryError> {
        let controller = Win32WindowController::new();
        let settings_store = SettingsFileStore::for_current_exe()?;
        let mut status = WideText::new("Ready");
        let mut settings_save_policy = SettingsSavePolicy::Enabled;
        let mut preserved_startup_session = None;
        let mut workspace_options = WorkspaceOptions::default();
        let mut app = match settings_store.load_workspace_for_startup() {
            Ok(Some(settings)) => {
                workspace_options = settings.options();
                let saved_tab_count = settings.saved_tab_count();
                let saved_tab_preset_count = settings.tab_presets().len();
                let (tab_presets, startup_session) =
                    settings.into_tab_presets_and_preserved_session();
                let state = AppState::from_tab_presets_only(
                    tab_presets,
                    crate::domain::DEFAULT_MIN_REGION_SIZE,
                )
                .map_err(AppError::from)?;
                settings_save_policy =
                    SettingsSavePolicy::PreserveStartupSessionUntilWorkspaceChange;
                preserved_startup_session = Some(startup_session);
                status.replace(startup_saved_workspace_skipped_status_text(
                    workspace_options.ui_language(),
                    saved_tab_count,
                    saved_tab_preset_count,
                ));
                App::with_state(controller, state)
            }
            Ok(None) => App::new(controller),
            Err(error) => {
                log_settings_load_error(&error);
                status.replace(settings_load_failure_status_text(
                    workspace_options.ui_language(),
                    &error,
                ));
                settings_save_policy = SettingsSavePolicy::WaitForWorkspaceChange;
                App::new(controller)
            }
        };

        if app.state().workspace().tabs().is_empty() {
            app.create_initial_tab("Tab 0")?;
        }

        let next_tab_number = next_tab_number(app.state().workspace().next_tab_id());

        Ok(Self {
            app,
            settings_store,
            hwnd: null_mut(),
            active_region: None,
            dragging_splitter: None,
            last_splitter_drag_screen_point: None,
            splitter_drag_layout_cache: None,
            drop_tracker: DropTracker::default(),
            drop_poll_timer_active: false,
            drop_move_event_hook: None,
            window_name_change_event_hook: None,
            splitter_overlay: SplitterOverlayController::default(),
            splitter_overlay_poll_timer_active: false,
            splitter_overlay_rect_cache: None,
            pending_tab_click: None,
            tab_reorder_drag: None,
            tab_context_target: None,
            tab_overflow_first_visible_index: 0,
            tab_tooltip: TabTooltip::default(),
            tab_tooltip_sync_key: None,
            workspace_change_generation: 0,
            status,
            next_tab_number,
            shutdown_done: false,
            settings_save_policy,
            preserved_startup_session,
            workspace_options,
            frame_state: MainWindowFrameState::default(),
            workspace_ui_visibility: WorkspaceUiVisibility::new(true),
            paint: MainWindowPaintState::new(),
            active_tab_sync_cache: None,
            tab_preset_program_restore: None,
            tab_preset_program_restore_timer_active: false,
            icons: Vec::new(),
            owned_by_window: false,
            active_message_handlers: 0,
            destroy_pending: false,
        })
    }

    fn initialize(&mut self, hwnd: HWND) {
        self.hwnd = hwnd;
        self.apply_window_title();
        self.refresh_main_menu();

        if let Ok(owner) = WindowHandle::new(hwnd as isize) {
            self.app.controller_mut().exclude_owner_window(owner);
        }

        self.install_icons();
        self.install_drop_move_event_hook();
        self.install_window_name_change_event_hook();
        self.initialize_tab_tooltip();

        self.sync_active_tab();
    }

    fn install_drop_move_event_hook(&mut self) {
        if self.drop_move_event_hook.is_some() {
            return;
        }

        let Some(hook) = DropMoveEventHook::install(self.hwnd) else {
            self.report_win32_status(
                "SetWinEventHook",
                "외부 윈도우 이동 감지 hook을 시작할 수 없습니다.",
            );
            return;
        };

        self.drop_move_event_hook = Some(hook);
    }

    fn uninstall_drop_move_event_hook(&mut self) {
        if let Some(mut hook) = self.drop_move_event_hook.take() {
            hook.uninstall();
        }
    }

    fn install_window_name_change_event_hook(&mut self) {
        if self.window_name_change_event_hook.is_some() {
            return;
        }

        let Some(hook) = WindowNameChangeEventHook::install(self.hwnd) else {
            self.report_win32_status(
                "SetWinEventHook",
                "외부 윈도우 제목 변경 감지 hook을 시작할 수 없습니다.",
            );
            return;
        };

        self.window_name_change_event_hook = Some(hook);
    }

    fn uninstall_window_name_change_event_hook(&mut self) {
        if let Some(mut hook) = self.window_name_change_event_hook.take() {
            hook.uninstall();
        }
    }

    fn initialize_tab_tooltip(&mut self) {
        match self.tab_tooltip.initialize(self.hwnd) {
            Ok(()) => self.sync_tab_overflow(),
            Err(error) => self.report_win32_status_failure(error),
        }
    }

    fn destroy_tab_tooltip(&mut self) {
        self.clear_tab_tooltip_sync_key();
        self.tab_tooltip.destroy(self.hwnd);
    }

    fn start_drop_poll_timer(&mut self) {
        if self.drop_poll_timer_active || self.hwnd.is_null() || self.frame_state.is_minimized() {
            return;
        }

        let timer = unsafe { SetTimer(self.hwnd, TIMER_DROP_POLL, DROP_POLL_INTERVAL_MS, None) };
        if timer == 0 {
            self.set_status("외부 윈도우 drop 감지 timer를 시작할 수 없습니다.");
            return;
        }

        self.drop_poll_timer_active = true;
    }

    fn stop_drop_poll_timer(&mut self) {
        if !self.drop_poll_timer_active {
            return;
        }

        self.drop_poll_timer_active = false;
        if self.hwnd.is_null() {
            return;
        }

        unsafe {
            KillTimer(self.hwnd, TIMER_DROP_POLL);
        }
    }

    fn reset_drop_tracking(&mut self) {
        self.drop_tracker = DropTracker::default();
        self.stop_drop_poll_timer();
    }

    fn teardown_drop_detection(&mut self) {
        self.uninstall_drop_move_event_hook();
        self.stop_drop_poll_timer();
        self.drop_tracker = DropTracker::default();
    }

    fn start_splitter_overlay_poll_timer(&mut self) -> bool {
        if self.splitter_overlay_poll_timer_active || self.hwnd.is_null() {
            return self.splitter_overlay_poll_timer_active;
        }

        let timer = unsafe {
            SetTimer(
                self.hwnd,
                TIMER_SPLITTER_OVERLAY_POLL,
                SPLITTER_OVERLAY_POLL_INTERVAL_MS,
                None,
            )
        };
        if timer == 0 {
            self.set_status("splitter overlay timer를 시작할 수 없습니다.");
            return false;
        }

        self.splitter_overlay_poll_timer_active = true;
        true
    }

    fn stop_splitter_overlay_poll_timer(&mut self) {
        if !self.splitter_overlay_poll_timer_active {
            return;
        }

        self.splitter_overlay_poll_timer_active = false;
        if self.hwnd.is_null() {
            return;
        }

        unsafe {
            KillTimer(self.hwnd, TIMER_SPLITTER_OVERLAY_POLL);
        }
    }

    fn start_tab_preset_program_restore_timer(&mut self) -> bool {
        if self.tab_preset_program_restore_timer_active {
            return true;
        }
        if self.hwnd.is_null() {
            return false;
        }

        let timer = unsafe {
            SetTimer(
                self.hwnd,
                TIMER_TAB_PRESET_PROGRAM_RESTORE,
                TAB_PRESET_WINDOW_POLL.as_millis() as u32,
                None,
            )
        };
        if timer == 0 {
            return false;
        }

        self.tab_preset_program_restore_timer_active = true;
        true
    }

    fn stop_tab_preset_program_restore_timer(&mut self) {
        if !self.tab_preset_program_restore_timer_active {
            return;
        }

        self.tab_preset_program_restore_timer_active = false;
        if self.hwnd.is_null() {
            return;
        }

        unsafe {
            KillTimer(self.hwnd, TIMER_TAB_PRESET_PROGRAM_RESTORE);
        }
    }

    fn cancel_tab_preset_program_restore(&mut self) {
        self.tab_preset_program_restore = None;
        self.stop_tab_preset_program_restore_timer();
    }

    fn teardown_splitter_overlay(&mut self) {
        self.stop_splitter_overlay_poll_timer();
        self.invalidate_splitter_overlay_rect_cache();
        self.splitter_overlay.destroy_all();
    }

    fn hide_splitter_overlay(&mut self) {
        self.stop_splitter_overlay_poll_timer();
        self.splitter_overlay.hide_all();
    }

    fn language(&self) -> UiLanguage {
        self.workspace_options.ui_language()
    }

    fn workspace_ui_visible(&self) -> bool {
        self.workspace_ui_visibility.effective_visible()
    }

    fn apply_window_title(&mut self) {
        let title = wide_null(WINDOW_TITLE);
        let ok = unsafe { SetWindowTextW(self.hwnd, title.as_ptr()) };
        if ok == 0 {
            self.report_win32_status(
                "SetWindowTextW",
                "j3GridDocker window title을 설정할 수 없습니다.",
            );
        }
    }

    fn refresh_main_menu(&mut self) {
        if self.hwnd.is_null() {
            return;
        }

        if let Err(error) = self.apply_main_menu_visibility(self.workspace_ui_visible()) {
            self.report_win32_status_failure(error);
        }
    }

    fn refresh_main_menu_after_size_change(&mut self) {
        if self.hwnd.is_null() {
            return;
        }

        let is_maximized = self.is_maximized();
        if !main_menu_needs_refresh_after_size(
            self.workspace_ui_visible(),
            self.frame_state.main_menu_maximized(),
            is_maximized,
        ) {
            return;
        }

        self.refresh_main_menu();
    }

    fn apply_main_menu_visibility(
        &mut self,
        workspace_ui_visible: bool,
    ) -> Result<(), Win32StatusFailure> {
        if !main_menu_visible_for_workspace_ui(workspace_ui_visible) {
            self.clear_main_menu()?;
            self.frame_state.clear_main_menu_size_cache();
            return Ok(());
        }

        let is_maximized = self.is_maximized();
        let Some(menu) = self.build_main_menu() else {
            return Err(Win32StatusFailure::new(
                "CreateMenu",
                unsafe { GetLastError() },
                "최상위 메뉴를 생성할 수 없습니다.",
            ));
        };
        let previous_menu = unsafe { GetMenu(self.hwnd) };
        let ok = unsafe { SetMenu(self.hwnd, menu) };
        if ok == 0 {
            unsafe {
                DestroyMenu(menu);
            }
            return Err(Win32StatusFailure::new(
                "SetMenu",
                unsafe { GetLastError() },
                "최상위 메뉴를 적용할 수 없습니다.",
            ));
        }
        if !previous_menu.is_null() {
            unsafe {
                DestroyMenu(previous_menu);
            }
        }
        unsafe {
            DrawMenuBar(self.hwnd);
        }
        self.frame_state.cache_main_menu_size(is_maximized);

        Ok(())
    }

    fn clear_main_menu(&mut self) -> Result<(), Win32StatusFailure> {
        let previous_menu = unsafe { GetMenu(self.hwnd) };
        if previous_menu.is_null() {
            return Ok(());
        }

        let ok = unsafe { SetMenu(self.hwnd, null_mut()) };
        if ok == 0 {
            return Err(Win32StatusFailure::new(
                "SetMenu",
                unsafe { GetLastError() },
                "최상위 메뉴를 숨길 수 없습니다.",
            ));
        }
        unsafe {
            DestroyMenu(previous_menu);
            DrawMenuBar(self.hwnd);
        }

        Ok(())
    }

    fn build_main_menu(&self) -> Option<HMENU> {
        let root = create_menu_handle()?;
        if self.populate_main_menu(root) {
            Some(root)
        } else {
            unsafe {
                DestroyMenu(root);
            }
            None
        }
    }

    fn populate_main_menu(&self, root: HMENU) -> bool {
        let presentation = self.main_menu_presentation();
        self.append_workspace_menu(root)
            && self.append_layout_menu(root)
            && self.append_presets_menu(root)
            && presentation.append_view_menu(root)
            && presentation.append_options_menu(root)
            && presentation.append_window_menu(root)
            && presentation.append_help_menu(root)
    }

    fn main_menu_presentation(&self) -> MainMenuPresentation {
        MainMenuPresentation {
            language: self.language(),
            workspace_ui_visible: self.workspace_ui_visible(),
            workspace_options: self.workspace_options,
            is_maximized: self.is_maximized(),
        }
    }

    fn append_workspace_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        let has_active_tab = self.app.active_tab_id().is_some();
        append_menu(
            menu,
            CMD_TAB_ADD,
            ui_text(self.language(), "New Tab", "새 탭"),
        );
        append_menu_enabled(
            menu,
            CMD_TAB_RENAME_CONTEXT,
            ui_text(self.language(), "Rename Tab...", "탭 이름 변경..."),
            has_active_tab,
        );
        append_menu_enabled(
            menu,
            CMD_TAB_CLOSE_CONTEXT,
            ui_text(self.language(), "Close Tab", "탭 닫기"),
            has_active_tab,
        );
        append_menu_enabled(
            menu,
            CMD_TAB_CLOSE_OTHER_CONTEXT,
            ui_text(self.language(), "Close Other Tabs", "다른 탭 닫기"),
            has_active_tab,
        );
        if append_submenu(
            root,
            menu,
            ui_text(self.language(), "Workspace", "작업공간"),
        ) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_layout_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        append_menu(
            menu,
            CMD_SPLIT_VERTICAL,
            ui_text(self.language(), "Split Region Vertically", "영역 세로 분할"),
        );
        append_menu(
            menu,
            CMD_SPLIT_HORIZONTAL,
            ui_text(
                self.language(),
                "Split Region Horizontally",
                "영역 가로 분할",
            ),
        );
        append_menu(
            menu,
            CMD_REGION_DELETE,
            ui_text(self.language(), "Delete Selected Region", "선택 영역 삭제"),
        );
        append_menu(
            menu,
            CMD_UNDOCK,
            ui_text(
                self.language(),
                "Undock Selected Window",
                "선택 창 배치 해제",
            ),
        );

        if append_submenu(root, menu, ui_text(self.language(), "Layout", "레이아웃")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_presets_menu(&self, root: HMENU) -> bool {
        let Some(menu) = create_menu_handle() else {
            return false;
        };
        if !self.append_tab_preset_menu_items(menu) {
            unsafe {
                DestroyMenu(menu);
            }
            return false;
        }

        if append_submenu(root, menu, ui_text(self.language(), "Presets", "프리셋")) {
            true
        } else {
            unsafe {
                DestroyMenu(menu);
            }
            false
        }
    }

    fn append_tab_preset_menu_items(&self, menu: HMENU) -> bool {
        append_menu_enabled(
            menu,
            CMD_TAB_PRESET_SAVE,
            ui_text(self.language(), "Save Tab Preset...", "탭 preset 저장..."),
            self.app.active_tab_id().is_some(),
        );

        let tab_presets = self.app.list_tab_presets();
        let Some(load_menu) = self.build_tab_preset_submenu(
            tab_presets.iter().map(|preset| preset.name()),
            CMD_TAB_PRESET_BASE,
            CMD_TAB_PRESET_END,
        ) else {
            return false;
        };
        if !append_submenu(
            menu,
            load_menu,
            ui_text(self.language(), "Load Tab Preset", "탭 preset 불러오기"),
        ) {
            unsafe {
                DestroyMenu(load_menu);
            }
            return false;
        }

        let Some(edit_menu) = self.build_tab_preset_submenu(
            tab_presets.iter().map(|preset| preset.name()),
            CMD_TAB_PRESET_EDIT_BASE,
            CMD_TAB_PRESET_EDIT_END,
        ) else {
            return false;
        };
        if !append_submenu(
            menu,
            edit_menu,
            ui_text(self.language(), "Edit Tab Preset", "탭 preset 편집"),
        ) {
            unsafe {
                DestroyMenu(edit_menu);
            }
            return false;
        }

        let Some(delete_menu) = self.build_tab_preset_submenu(
            tab_presets.iter().map(|preset| preset.name()),
            CMD_TAB_PRESET_DELETE_BASE,
            CMD_TAB_PRESET_DELETE_END,
        ) else {
            return false;
        };
        if !append_submenu(
            menu,
            delete_menu,
            ui_text(self.language(), "Delete Tab Preset", "탭 preset 삭제"),
        ) {
            unsafe {
                DestroyMenu(delete_menu);
            }
            return false;
        }

        true
    }

    fn build_tab_preset_submenu<'a>(
        &self,
        preset_names: impl IntoIterator<Item = &'a str>,
        base: u16,
        end: u16,
    ) -> Option<HMENU> {
        let menu = create_menu_handle()?;
        let appended = append_preset_menu_items(menu, preset_names, base, end);
        if !appended {
            append_disabled_menu(
                menu,
                ui_text(
                    self.language(),
                    "(No saved tab presets)",
                    "(저장된 탭 preset 없음)",
                ),
            );
        }
        Some(menu)
    }

    fn handle_message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_CREATE => {
                self.initialize(self.hwnd);
                0
            }
            WM_PAINT => {
                self.paint();
                0
            }
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                self.on_size(wparam);
                0
            }
            WM_MOVE => {
                self.on_move();
                0
            }
            WM_ACTIVATE => {
                self.on_activate(low_word(wparam), lparam);
                unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) }
            }
            WM_WINDOWPOSCHANGED => unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) },
            WM_LBUTTONDOWN => {
                self.relay_tab_tooltip_mouse_message(message, wparam, lparam);
                self.on_left_button_down(point_from_lparam(lparam));
                0
            }
            WM_LBUTTONDBLCLK => {
                self.on_left_button_double_click(point_from_lparam(lparam));
                0
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move(point_from_lparam(lparam), wparam, lparam);
                0
            }
            WM_KEYDOWN | WM_KEYUP => {
                self.sync_splitter_overlay();
                unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) }
            }
            WM_LBUTTONUP => {
                self.relay_tab_tooltip_mouse_message(message, wparam, lparam);
                self.on_left_button_up(point_from_lparam(lparam));
                0
            }
            WM_CAPTURECHANGED => {
                self.on_capture_changed();
                0
            }
            WM_CANCELMODE => {
                self.cancel_pointer_drags();
                0
            }
            WM_RBUTTONUP => {
                self.relay_tab_tooltip_mouse_message(message, wparam, lparam);
                self.on_right_button_up(point_from_lparam(lparam));
                0
            }
            WM_COMMAND => {
                let command_id = low_word(wparam);
                self.execute_command(command_id);
                0
            }
            WM_SPLITTER_OVERLAY_LBUTTONDOWN => {
                self.on_splitter_overlay_left_button_down();
                0
            }
            WM_DROP_MOVE_SIZE_EVENT => {
                self.on_drop_move_size_event(wparam as u32, lparam as HWND);
                0
            }
            WM_WINDOW_NAME_CHANGE_EVENT => {
                self.on_window_name_change_event(lparam as HWND);
                0
            }
            WM_TAB_PRESET_PROGRAM_WINDOW_EVENT => {
                self.on_tab_preset_program_window_event(lparam as HWND);
                0
            }
            WM_TIMER => {
                if wparam == TIMER_DROP_POLL {
                    self.poll_external_drop();
                    0
                } else if wparam == TIMER_SPLITTER_OVERLAY_POLL {
                    self.sync_splitter_overlay();
                    0
                } else if wparam == TIMER_TAB_PRESET_PROGRAM_RESTORE {
                    self.poll_tab_preset_program_restore();
                    0
                } else {
                    unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) }
                }
            }
            WM_SETCURSOR => unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) },
            WM_CLOSE => {
                if self.shutdown_once(ShutdownMode::Cancellable) {
                    unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) }
                } else {
                    self.invalidate();
                    0
                }
            }
            WM_DESTROY => {
                self.destroy_tab_tooltip();
                self.teardown_drop_detection();
                self.uninstall_window_name_change_event_hook();
                self.teardown_splitter_overlay();
                self.cancel_tab_preset_program_restore();
                self.shutdown_once(ShutdownMode::Forced);
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(self.hwnd, message, wparam, lparam) },
        }
    }

    fn paint(&mut self) {
        let mut paint = PAINTSTRUCT::default();
        let hdc = unsafe { BeginPaint(self.hwnd, &mut paint) };
        if hdc.is_null() {
            return;
        }

        if let Some(client) = self.client_rect() {
            let dirty = UiRect::from_rect(paint.rcPaint);
            let mut paint_buffer = take(&mut self.paint.buffer);
            paint_buffer.paint(hdc, client, dirty, |target, dirty| {
                self.paint_inner(target, client, dirty);
            });
            self.paint.buffer = paint_buffer;
        }

        unsafe {
            EndPaint(self.hwnd, &paint);
        }
    }

    fn paint_inner(&mut self, hdc: HDC, client: UiRect, dirty: UiRect) {
        let workspace_ui_visible = self.workspace_ui_visible();
        let sections = DirtyPaintSections::for_dirty(client, dirty, workspace_ui_visible);

        fill(hdc, dirty, COLOR_BG);
        if sections.top_bar
            && let Some(rect) = top_bar_rect_for_client(client, workspace_ui_visible)
        {
            fill(hdc, rect, COLOR_TOP);
        }
        if sections.status_bar
            && let Some(rect) = status_bar_rect_for_client(client, workspace_ui_visible)
        {
            fill(hdc, rect, COLOR_STATUS);
        }
        set_text(hdc, COLOR_TEXT);

        if sections.tab_strip {
            self.paint_toolbar_toggle(hdc);
            self.paint_new_tab_button(hdc);
            self.paint_tabs(hdc, client);
        }
        if sections.command_buttons {
            self.paint_buttons(hdc, client);
        }
        if sections.workspace_regions {
            self.paint_regions(hdc, dirty);
        }
        if sections.status_bar {
            self.paint_status(hdc, client);
        }
    }

    fn paint_toolbar_toggle(&mut self, hdc: HDC) {
        let rect = toolbar_toggle_rect();
        let label = workspace_ui_toggle_button_label(self.language(), self.workspace_ui_visible());
        self.paint.toolbar_toggle_label.replace_str(label);
        draw_box(hdc, rect, COLOR_BUTTON, COLOR_BUTTON_BORDER);
        draw_text_wide(hdc, rect, self.paint.toolbar_toggle_label.wide(), DT_CENTER);
    }

    fn paint_new_tab_button(&mut self, hdc: HDC) {
        let rect = new_tab_button_rect();
        let label = ui_text(self.language(), "New", "새 탭");
        self.paint.new_tab_label.replace_str(label);
        draw_box(hdc, rect, COLOR_BUTTON, COLOR_BUTTON_BORDER);
        draw_text_wide(hdc, rect, self.paint.new_tab_label.wide(), DT_CENTER);
    }

    fn paint_tabs(&mut self, hdc: HDC, client: UiRect) {
        let layout = self.sync_tab_overflow_for_client(client);
        self.sync_visible_paint_tab_labels(layout);
        let active = self.app.active_tab_id();
        let tab_labels = &self.paint.tab_labels;
        let first_visible_index = layout.first_visible_index;

        self.visit_tab_rects(layout, |tab| {
            let color = if Some(tab.tab_id) == active {
                COLOR_TAB_ACTIVE
            } else {
                COLOR_TAB_INACTIVE
            };
            draw_box(hdc, tab.rect, color, COLOR_BUTTON_BORDER);
            let label_rect = tab_label_rect(tab.rect);
            if let Some(label) = tab
                .index
                .checked_sub(first_visible_index)
                .and_then(|index| tab_labels.get(index))
                .filter(|label| label.tab_id() == tab.tab_id)
            {
                draw_text_wide(hdc, label_rect, label.wide(), DT_LEFT);
            } else {
                draw_text(hdc, label_rect, tab.label, DT_LEFT);
            }
            if let Some(close_rect) = tab_close_button_rect(tab.rect) {
                draw_box(hdc, close_rect, COLOR_BUTTON, COLOR_BUTTON_BORDER);
                draw_text_wide(hdc, close_rect, &TAB_CLOSE_LABEL, DT_CENTER);
            }
            true
        });
        self.paint_tab_overflow_dropdown(hdc, layout);
        self.paint_tab_reorder_insertion(hdc);
    }

    fn paint_tab_overflow_dropdown(&self, hdc: HDC, layout: TabStripLayout) {
        let Some(dropdown) = layout.dropdown else {
            return;
        };

        let fill_color = if dropdown.hidden_count > 0 {
            COLOR_BUTTON
        } else {
            COLOR_BUTTON_DISABLED
        };
        draw_box(hdc, dropdown.rect, fill_color, COLOR_BUTTON_BORDER);
        draw_text_wide(hdc, dropdown.rect, &TAB_OVERFLOW_DROPDOWN_LABEL, DT_CENTER);
    }

    fn paint_tab_reorder_insertion(&self, hdc: HDC) {
        let Some(drag) = self.tab_reorder_drag else {
            return;
        };
        let Some(insertion) = drag.insertion else {
            return;
        };

        let x = insertion.x;
        let rect = UiRect::new(
            x.saturating_sub(2),
            3,
            x.saturating_add(2),
            TAB_BAR_HEIGHT - 1,
        );
        fill(hdc, rect, COLOR_TAB_INSERTION);
    }

    fn paint_buttons(&mut self, hdc: HDC, client: UiRect) {
        if !self.workspace_ui_visible() {
            return;
        }

        if self.paint.button_labels.len() < BUTTON_SPECS.len() {
            self.paint
                .button_labels
                .resize_with(BUTTON_SPECS.len(), || WideText::new(""));
        }

        let language = self.language();
        visit_command_button_rects(client, |index, button| {
            let label = command_button_label(language, button.command);

            self.paint.button_labels[index].replace_str(label);
            draw_box(hdc, button.rect, COLOR_BUTTON, COLOR_BUTTON_BORDER);
            draw_text_wide(
                hdc,
                button.rect,
                self.paint.button_labels[index].wide(),
                DT_CENTER,
            );
            true
        });
    }

    fn paint_regions(&mut self, hdc: HDC, dirty: UiRect) {
        let Some(tab_id) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_client() else {
            return;
        };

        if !self.ensure_paint_occupied_regions(tab_id) {
            return;
        }

        if !self.ensure_paint_layout_cache(tab_id, bounds) {
            return;
        }

        let active_region = self.active_region;
        let language = self.language();
        let occupied_regions = &self.paint.occupied_regions;

        for region in self.paint.layout_cache.regions_mut() {
            let region_id = region.region_id();
            let rect = region.rect();
            if !rects_overlap(rect, dirty) {
                continue;
            }

            let is_occupied = occupied_regions.contains(&region_id);
            let color = if Some(region_id) == active_region {
                COLOR_REGION_ACTIVE
            } else if is_occupied {
                COLOR_REGION_OCCUPIED
            } else {
                COLOR_REGION
            };

            draw_box(hdc, rect, color, COLOR_REGION_BORDER);
            draw_text_wide(
                hdc,
                rect.inset(8, 6),
                region.title_wide(language, is_occupied),
                DT_LEFT,
            );
        }

        for splitter in self.paint.layout_cache.splitters() {
            if rects_overlap(*splitter, dirty) {
                fill(hdc, *splitter, COLOR_SPLITTER);
            }
        }
    }

    fn paint_status(&self, hdc: HDC, client: UiRect) {
        let rect = UiRect::new(
            8,
            client.bottom - STATUS_BAR_HEIGHT,
            client.right - 8,
            client.bottom,
        );
        draw_text_wide(hdc, rect, self.status.wide(), DT_LEFT);
    }

    fn on_left_button_down(&mut self, point: ClientPoint) {
        if let Some(command) = self.button_at(point) {
            self.execute_command(command);
            return;
        }

        if let Some(target) = self.tab_overflow_hit_at(point) {
            self.handle_tab_overflow_hit(target);
            return;
        }

        if let Some(tab) = self.tab_hit_at(point) {
            match tab_press_action_from_hit(tab, self.workspace_ui_visible()) {
                TabPressAction::Pending(action) => {
                    self.begin_pending_tab_click(tab.tab_id, point, action);
                }
                TabPressAction::Close(tab_id) => self.delete_tab(tab_id),
            }
            return;
        }

        if !self.workspace_ui_visible() {
            if self.is_tab_bar_point(point) {
                self.begin_window_move();
            }
            return;
        }

        let Some(active_tab) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let Some(screen_point) = self.client_point_to_screen(point) else {
            return;
        };

        if self.begin_splitter_drag_for_tab_at(active_tab, bounds, screen_point) {
            return;
        }

        match self
            .app
            .hit_test_region(active_tab, bounds, screen_point.x, screen_point.y)
        {
            Ok(region) => {
                let previous = self.active_region;
                self.active_region = region;
                self.invalidate_active_region_change(active_tab, previous, region);
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn on_splitter_overlay_left_button_down(&mut self) {
        if !ctrl_key_is_down() {
            self.hide_splitter_overlay();
            return;
        }

        let Some(screen_point) = cursor_position() else {
            self.hide_splitter_overlay();
            return;
        };

        if self.begin_splitter_drag_at_screen_point(screen_point) {
            self.hide_splitter_overlay();
        } else {
            self.sync_splitter_overlay();
        }
    }

    fn begin_splitter_drag_at_screen_point(&mut self, screen_point: ScreenPoint) -> bool {
        if !splitter_overlay_workspace_enabled(
            self.workspace_ui_visible(),
            self.workspace_options.dock_hidden_workspace_ui(),
        ) || self.frame_state.is_minimized()
        {
            return false;
        }

        let Some(active_tab) = self.app.active_tab_id() else {
            return false;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return false;
        };

        self.begin_splitter_drag_for_tab_at(active_tab, bounds, screen_point)
    }

    fn begin_splitter_drag_for_tab_at(
        &mut self,
        active_tab: TabId,
        bounds: Rect,
        screen_point: ScreenPoint,
    ) -> bool {
        match self.app.hit_test_splitter(
            active_tab,
            bounds,
            screen_point.x,
            screen_point.y,
            SPLITTER_HIT_TOLERANCE,
        ) {
            Ok(Some(splitter)) => {
                self.dragging_splitter = Some(splitter.path().clone());
                self.last_splitter_drag_screen_point = Some((screen_point.x, screen_point.y));
                self.splitter_drag_layout_cache = None;
                unsafe {
                    SetCapture(self.hwnd);
                }
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.report_app_error(error);
                true
            }
        }
    }

    fn on_left_button_double_click(&mut self, point: ClientPoint) {
        if self.tab_strip_empty_at(point) {
            self.toggle_window_maximize_restore();
        }
    }

    fn on_mouse_move(&mut self, point: ClientPoint, wparam: WPARAM, lparam: LPARAM) {
        self.refresh_tab_tooltips_at(point);
        self.relay_tab_tooltip_mouse_message(WM_MOUSEMOVE, wparam, lparam);

        if self.handle_pending_tab_click_move(point) {
            return;
        }

        if self.handle_tab_reorder_drag_move(point) {
            return;
        }

        let Some(path) = self.dragging_splitter.as_ref() else {
            self.sync_splitter_overlay();
            return;
        };
        let Some(active_tab) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let Some(screen_point) = self.client_point_to_screen(point) else {
            return;
        };
        let drag_point = (screen_point.x, screen_point.y);
        if self.last_splitter_drag_screen_point == Some(drag_point) {
            return;
        }

        let cached_regions = match self.splitter_drag_layout_cache.take() {
            Some(cache) if cache.tab_id == active_tab && cache.bounds == bounds => {
                Some(cache.regions)
            }
            Some(cache) => {
                self.splitter_drag_layout_cache = Some(cache);
                None
            }
            None => None,
        };

        match self.app.resize_splitter_with_owned_cached_regions(
            active_tab,
            path,
            bounds,
            screen_point.x,
            screen_point.y,
            cached_regions,
        ) {
            Ok((SplitterResizeOutcome::Unchanged, retained_cache)) => {
                if let Some(regions) = retained_cache {
                    self.splitter_drag_layout_cache = Some(SplitterDragLayoutCache {
                        tab_id: active_tab,
                        bounds,
                        regions,
                    });
                }
                self.last_splitter_drag_screen_point = Some(drag_point);
            }
            Ok((SplitterResizeOutcome::Changed { target_regions }, retained_cache)) => {
                if let Some(regions) = retained_cache {
                    self.splitter_drag_layout_cache = Some(SplitterDragLayoutCache {
                        tab_id: active_tab,
                        bounds,
                        regions,
                    });
                }
                self.record_workspace_change();
                self.last_splitter_drag_screen_point = Some(drag_point);
                if let Some(regions) = target_regions {
                    if !self
                        .rebuild_paint_layout_cache_from_drag_regions(active_tab, bounds, &regions)
                    {
                        self.invalidate_paint_layout_cache();
                    }
                    self.splitter_drag_layout_cache = Some(SplitterDragLayoutCache {
                        tab_id: active_tab,
                        bounds,
                        regions,
                    });
                } else {
                    self.invalidate_paint_layout_cache();
                }
                self.invalidate_workspace_body();
            }
            Err(error) => {
                self.splitter_drag_layout_cache = None;
                self.report_app_error(error);
            }
        }
    }

    fn refresh_tab_tooltips_at(&mut self, point: ClientPoint) {
        if self.pending_tab_click.is_some()
            || self.tab_reorder_drag.is_some()
            || self.dragging_splitter.is_some()
        {
            return;
        }

        let Some((client, layout)) = self.current_tab_strip_layout_with_client() else {
            return;
        };
        if hit_test_tab_strip(layout, point).is_some() {
            self.sync_tab_tooltips(client, layout);
        }
    }

    fn relay_tab_tooltip_mouse_message(&self, message: u32, wparam: WPARAM, lparam: LPARAM) {
        self.tab_tooltip
            .relay_mouse_message(self.hwnd, message, wparam, lparam);
    }

    fn on_left_button_up(&mut self, point: ClientPoint) {
        if let Some(pending) = self.pending_tab_click.take() {
            unsafe {
                ReleaseCapture();
            }
            if let Some(tab_id) =
                pending_tab_release_switch_target(pending, self.tab_body_at(point))
            {
                self.switch_tab(tab_id);
            }
            return;
        }

        if self.finish_tab_reorder_drag() {
            return;
        }

        if self.dragging_splitter.take().is_some() {
            self.last_splitter_drag_screen_point = None;
            self.splitter_drag_layout_cache = None;
            unsafe {
                ReleaseCapture();
            }
            self.sync_active_tab();
            self.invalidate();
            self.sync_splitter_overlay();
        }
    }

    fn begin_pending_tab_click(
        &mut self,
        tab_id: TabId,
        point: ClientPoint,
        action: PendingTabAction,
    ) {
        self.pending_tab_click = Some(PendingTabClick {
            tab_id,
            point,
            action,
        });
        unsafe {
            SetCapture(self.hwnd);
        }
    }

    fn handle_pending_tab_click_move(&mut self, point: ClientPoint) -> bool {
        let Some(pending) = self.pending_tab_click else {
            return false;
        };

        match pending_tab_move_outcome(pending, point) {
            PendingTabMoveOutcome::ContinueClick => {}
            PendingTabMoveOutcome::StartReorder(tab_id) => {
                self.pending_tab_click = None;
                self.begin_tab_reorder_drag(tab_id, point);
            }
            PendingTabMoveOutcome::StartWindowMove => {
                self.pending_tab_click = None;
                self.begin_window_move();
            }
        }

        true
    }

    fn begin_tab_reorder_drag(&mut self, tab_id: TabId, point: ClientPoint) {
        if self.tab_index(tab_id).is_none() {
            self.set_status("탭 순서 변경 실패: 대상 탭을 찾을 수 없습니다.");
            unsafe {
                ReleaseCapture();
            }
            return;
        }

        self.tab_reorder_drag = Some(TabReorderDrag {
            tab_id,
            insertion: None,
        });
        self.update_tab_reorder_drag(point);
        self.invalidate();
    }

    fn handle_tab_reorder_drag_move(&mut self, point: ClientPoint) -> bool {
        if self.tab_reorder_drag.is_none() {
            return false;
        }

        self.update_tab_reorder_drag(point);
        true
    }

    fn update_tab_reorder_drag(&mut self, point: ClientPoint) {
        let Some(layout) = self.current_tab_strip_layout() else {
            return;
        };

        if let Some(direction) = tab_reorder_auto_scroll(layout, point)
            && self.scroll_tab_reorder_view(direction)
        {
            self.invalidate();
        }

        let Some(layout) = self.current_tab_strip_layout() else {
            return;
        };
        let Some(target) = tab_insertion_target(layout, point) else {
            return;
        };
        let insertion = TabReorderInsertion {
            before_tab_id: self.before_tab_id_for_insertion_index(target.before_index),
            x: target.x,
        };

        let changed = if let Some(drag) = self.tab_reorder_drag.as_mut() {
            if drag.insertion != Some(insertion) {
                drag.insertion = Some(insertion);
                true
            } else {
                false
            }
        } else {
            false
        };

        if changed {
            self.invalidate();
        }
    }

    fn scroll_tab_reorder_view(&mut self, direction: TabReorderAutoScroll) -> bool {
        let Some(layout) = self.current_tab_strip_layout() else {
            return false;
        };

        match direction {
            TabReorderAutoScroll::Backward if layout.first_visible_index > 0 => {
                self.tab_overflow_first_visible_index = layout.first_visible_index - 1;
                true
            }
            TabReorderAutoScroll::Forward if layout.visible_end_index() < layout.tab_count => {
                self.tab_overflow_first_visible_index = layout.first_visible_index + 1;
                true
            }
            _ => false,
        }
    }

    fn finish_tab_reorder_drag(&mut self) -> bool {
        let Some(drag) = self.tab_reorder_drag.take() else {
            return false;
        };

        unsafe {
            ReleaseCapture();
        }

        if let Some(insertion) = drag.insertion {
            match self
                .app
                .reorder_tab_before(drag.tab_id, insertion.before_tab_id)
            {
                Ok(changed) => {
                    if changed {
                        self.record_workspace_change();
                    }
                    self.sync_tab_overflow();
                    log_tab_ux_trace(
                        "reorder-finish",
                        format_args!(
                            "tab_id={}, before_tab_id={}, changed={changed}",
                            drag.tab_id.value(),
                            optional_tab_id_trace_text(insertion.before_tab_id)
                        ),
                    );
                    self.set_status(&tab_reorder_status_text_for(
                        self.language(),
                        drag.tab_id,
                        insertion.before_tab_id,
                        changed,
                    ));
                }
                Err(error) => {
                    log_tab_ux_trace(
                        "reorder-error",
                        format_args!(
                            "tab_id={}, before_tab_id={}",
                            drag.tab_id.value(),
                            optional_tab_id_trace_text(insertion.before_tab_id)
                        ),
                    );
                    self.report_tab_operation_error(drag.tab_id, "순서 변경", error);
                }
            }
        }

        self.invalidate();
        true
    }

    fn before_tab_id_for_insertion_index(&self, before_index: usize) -> Option<TabId> {
        self.app
            .state()
            .workspace()
            .tabs()
            .get(before_index)
            .map(crate::domain::Tab::id)
    }

    fn cancel_pending_tab_click(&mut self) {
        if self.pending_tab_click.take().is_some() {
            unsafe {
                ReleaseCapture();
            }
        }
    }

    fn cancel_tab_reorder_drag(&mut self) {
        if self.tab_reorder_drag.take().is_some() {
            unsafe {
                ReleaseCapture();
            }
            self.sync_tab_overflow();
            self.invalidate();
        }
    }

    fn cancel_pointer_drags(&mut self) {
        self.cancel_pending_tab_click();
        self.cancel_tab_reorder_drag();
        self.cancel_splitter_drag();
    }

    fn on_capture_changed(&mut self) {
        let had_pending_tab = self.pending_tab_click.take().is_some();
        let had_tab_reorder = self.tab_reorder_drag.take().is_some();
        let had_splitter = self.dragging_splitter.take().is_some();
        if had_splitter {
            self.last_splitter_drag_screen_point = None;
            self.splitter_drag_layout_cache = None;
        }
        if had_pending_tab || had_tab_reorder || had_splitter {
            self.sync_tab_overflow();
            self.invalidate();
        }
    }

    fn begin_window_move(&self) {
        unsafe {
            ReleaseCapture();
            SendMessageW(self.hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
        }
    }

    fn on_size(&mut self, wparam: WPARAM) {
        if wparam == SIZE_MINIMIZED as WPARAM {
            self.on_minimized();
        } else {
            self.on_restored_or_resized();
        }

        if !self.frame_state.is_minimized()
            && let Err(error) = self.apply_workspace_ui_window_region(self.workspace_ui_visible())
        {
            self.report_win32_status_failure(error);
        }
        self.sync_tab_overflow();
        self.refresh_main_menu_after_size_change();
        self.invalidate();
        self.sync_splitter_overlay();
    }

    fn on_move(&mut self) {
        if self.frame_state.is_minimized() {
            return;
        }

        self.sync_active_tab();
        self.sync_splitter_overlay();
    }

    fn on_activate(&mut self, activation_state: u16, other_window: LPARAM) {
        if activation_state == 0 {
            return;
        }

        let hwnd = other_window as HWND;
        if hwnd.is_null() {
            return;
        }

        if let Some(external) = self.external_root_from_hwnd(hwnd) {
            self.select_active_region_for_placed_window(external);
        }
    }

    fn on_minimized(&mut self) {
        if !self.frame_state.mark_minimized() {
            return;
        }

        self.reset_drop_tracking();
        self.cancel_pending_tab_click();
        self.cancel_tab_reorder_drag();
        self.cancel_splitter_drag();
        self.hide_splitter_overlay();

        if let Err(error) = self.app.hide_active_tab() {
            self.report_app_error(error);
        }
    }

    fn cancel_splitter_drag(&mut self) {
        if self.dragging_splitter.take().is_some() {
            self.last_splitter_drag_screen_point = None;
            self.splitter_drag_layout_cache = None;
            unsafe {
                ReleaseCapture();
            }
        }
    }

    fn on_restored_or_resized(&mut self) {
        self.frame_state.mark_restored_or_resized();
        self.splitter_drag_layout_cache = None;

        self.sync_active_tab();
        self.sync_splitter_overlay();
    }

    fn on_right_button_up(&mut self, point: ClientPoint) {
        if toolbar_toggle_rect().contains(point) {
            if let Some(screen_point) = self.client_point_to_screen(point) {
                self.show_options_menu_at(screen_point);
            } else {
                self.set_status("옵션 메뉴 위치를 계산할 수 없습니다.");
            }
            return;
        }

        if let Some(tab_id) = self.tab_context_target_at(point) {
            if let Some(screen_point) = self.client_point_to_screen(point) {
                self.show_tab_context_menu(tab_id, screen_point);
            } else {
                self.set_status("탭 메뉴 위치를 계산할 수 없습니다.");
            }
            return;
        }

        if self.tab_strip_empty_at(point) {
            if let Some(screen_point) = self.client_point_to_screen(point) {
                self.show_tab_blank_context_menu(screen_point);
            } else {
                self.set_status("탭바 메뉴 위치를 계산할 수 없습니다.");
            }
            return;
        }

        if !self.workspace_ui_visible() {
            return;
        }

        let Some(active_tab) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let Some(screen_point) = self.client_point_to_screen(point) else {
            return;
        };

        match self
            .app
            .hit_test_region(active_tab, bounds, screen_point.x, screen_point.y)
        {
            Ok(Some(region)) => {
                self.active_region = Some(region);
                self.show_region_menu(screen_point);
                if self.hwnd.is_null() {
                    return;
                }
                self.invalidate();
            }
            Ok(None) => {}
            Err(error) => self.report_app_error(error),
        }
    }

    fn show_region_menu(&mut self, screen_point: ScreenPoint) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status("영역 메뉴를 열 수 없습니다.");
            return;
        }

        append_menu(
            menu,
            CMD_SPLIT_VERTICAL,
            ui_text(self.language(), "Split vertical", "세로 분할"),
        );
        append_menu(
            menu,
            CMD_SPLIT_HORIZONTAL,
            ui_text(self.language(), "Split horizontal", "가로 분할"),
        );
        append_menu(
            menu,
            CMD_REGION_DELETE,
            ui_text(self.language(), "Delete region", "영역 삭제"),
        );
        append_menu(menu, CMD_UNDOCK, ui_text(self.language(), "Undock", "해제"));

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        let Some(command_id) = popup_selected_command(selected, self.hwnd) else {
            return;
        };
        self.execute_command(command_id);
    }

    fn show_tab_blank_context_menu(&mut self, screen_point: ScreenPoint) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status("탭바 메뉴를 열 수 없습니다.");
            return;
        }

        append_menu(
            menu,
            CMD_TAB_ADD,
            ui_text(self.language(), "New tab", "새 탭"),
        );

        self.tab_context_target = self.app.active_tab_id();
        if self.tab_context_target.is_some() {
            append_menu(
                menu,
                CMD_TAB_RENAME_CONTEXT,
                ui_text(self.language(), "Rename active tab", "활성 탭 이름 변경"),
            );
            append_menu(
                menu,
                CMD_TAB_CLOSE_CONTEXT,
                ui_text(self.language(), "Close active tab", "활성 탭 닫기"),
            );
            append_menu(
                menu,
                CMD_TAB_CLOSE_OTHER_CONTEXT,
                ui_text(self.language(), "Close other tabs", "다른 탭 닫기"),
            );
            append_separator(menu);
            append_menu(
                menu,
                CMD_TAB_PRESET_SAVE,
                ui_text(
                    self.language(),
                    "Save active tab preset...",
                    "활성 탭 preset 저장...",
                ),
            );
            append_menu(
                menu,
                CMD_TAB_PRESET_LOAD,
                ui_text(
                    self.language(),
                    "Load active tab preset...",
                    "활성 탭 preset 불러오기...",
                ),
            );
            append_menu(
                menu,
                CMD_TAB_PRESET_EDIT,
                ui_text(self.language(), "Edit tab preset...", "탭 preset 편집..."),
            );
            append_menu(
                menu,
                CMD_TAB_PRESET_DELETE,
                ui_text(self.language(), "Delete tab preset...", "탭 preset 삭제..."),
            );
        }
        append_separator(menu);
        append_menu(
            menu,
            CMD_WINDOW_MINIMIZE,
            ui_text(self.language(), "Minimize window", "창 최소화"),
        );
        append_menu(
            menu,
            CMD_WINDOW_MAXIMIZE_RESTORE,
            window_maximize_restore_menu_label(self.language(), self.is_maximized()),
        );
        append_menu(
            menu,
            CMD_WINDOW_CLOSE,
            ui_text(self.language(), "Close window", "창 닫기"),
        );

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        if let Some(command_id) = popup_selected_command(selected, self.hwnd) {
            self.execute_command(command_id);
        }
        if self.hwnd.is_null() {
            return;
        }
        self.tab_context_target = None;
    }

    fn show_tab_context_menu(&mut self, tab_id: TabId, screen_point: ScreenPoint) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status("탭 메뉴를 열 수 없습니다.");
            return;
        }

        append_menu(
            menu,
            CMD_TAB_RENAME_CONTEXT,
            ui_text(self.language(), "Rename tab", "탭 이름 변경"),
        );
        append_menu(
            menu,
            CMD_TAB_CLOSE_CONTEXT,
            ui_text(self.language(), "Close tab", "탭 닫기"),
        );
        append_menu(
            menu,
            CMD_TAB_CLOSE_OTHER_CONTEXT,
            ui_text(self.language(), "Close other tabs", "다른 탭 닫기"),
        );
        append_separator(menu);
        append_menu(
            menu,
            CMD_TAB_PRESET_SAVE,
            ui_text(self.language(), "Save tab preset...", "탭 preset 저장..."),
        );
        append_menu(
            menu,
            CMD_TAB_PRESET_LOAD,
            ui_text(
                self.language(),
                "Load tab preset...",
                "탭 preset 불러오기...",
            ),
        );

        self.tab_context_target = Some(tab_id);
        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        if self.hwnd.is_null() {
            return;
        }
        if let Some(command_id) = popup_selected_command(selected, self.hwnd) {
            self.execute_command(command_id);
        }
        if self.hwnd.is_null() {
            return;
        }
        self.tab_context_target = None;
    }

    fn execute_command(&mut self, command_id: u16) {
        if self.execute_preset_menu_command(command_id) {
            return;
        }

        match command_id {
            CMD_TAB_ADD => self.add_tab(),
            CMD_SPLIT_VERTICAL => self.split_active_region(SplitDirection::Vertical),
            CMD_SPLIT_HORIZONTAL => self.split_active_region(SplitDirection::Horizontal),
            CMD_REGION_DELETE => self.delete_active_region(),
            CMD_UNDOCK => self.undock_active_region(),
            CMD_WORKSPACE_UI_TOGGLE => self.toggle_workspace_ui(),
            CMD_TAB_PRESET_SAVE => self.save_active_tab_preset(),
            CMD_TAB_PRESET_LOAD => self.show_tab_preset_menu(),
            CMD_TAB_PRESET_EDIT => self.show_edit_tab_preset_menu(),
            CMD_TAB_PRESET_DELETE => self.show_delete_tab_preset_menu(),
            CMD_OPTIONS => self.show_options_menu(),
            CMD_DOCK_HIDDEN_WORKSPACE_UI_TOGGLE => self.toggle_dock_hidden_workspace_ui(),
            CMD_LANGUAGE_ENGLISH => self.set_ui_language(UiLanguage::English),
            CMD_LANGUAGE_KOREAN => self.set_ui_language(UiLanguage::Korean),
            CMD_WINDOW_MINIMIZE => self.minimize_window(),
            CMD_WINDOW_MAXIMIZE_RESTORE => self.toggle_window_maximize_restore(),
            CMD_WINDOW_CLOSE => self.close_window(),
            CMD_ABOUT => self.show_about_dialog(),
            _ => {
                self.execute_tab_context_command(command_id);
            }
        }
    }

    fn execute_preset_menu_command(&mut self, command_id: u16) -> bool {
        if let Some(preset_name) =
            self.tab_preset_name_for_command(command_id, CMD_TAB_PRESET_BASE, CMD_TAB_PRESET_END)
        {
            self.apply_tab_preset_to_active_tab(&preset_name);
            return true;
        }

        if let Some(preset_name) = self.tab_preset_name_for_command(
            command_id,
            CMD_TAB_PRESET_DELETE_BASE,
            CMD_TAB_PRESET_DELETE_END,
        ) {
            self.delete_tab_preset(&preset_name);
            return true;
        }

        if let Some(preset_name) = self.tab_preset_name_for_command(
            command_id,
            CMD_TAB_PRESET_EDIT_BASE,
            CMD_TAB_PRESET_EDIT_END,
        ) {
            self.edit_tab_preset(&preset_name);
            return true;
        }

        false
    }

    fn tab_preset_name_for_command(&self, command_id: u16, base: u16, end: u16) -> Option<String> {
        let index = command_index_from_range(command_id, base, end)?;
        self.app
            .list_tab_presets()
            .get(index)
            .map(|preset| preset.name().to_owned())
    }

    fn execute_tab_context_command(&mut self, command_id: u16) -> bool {
        let Some(action) = tab_context_action_from_command(command_id) else {
            return false;
        };

        let should_clear_fallback_target = self.tab_context_target.is_none();
        if should_clear_fallback_target {
            self.tab_context_target = self.app.active_tab_id();
        }
        self.execute_tab_context_action(action);
        if should_clear_fallback_target && !self.hwnd.is_null() {
            self.tab_context_target = None;
        }
        true
    }

    fn execute_tab_context_action(&mut self, action: TabContextAction) {
        if let Some(tab_id) = self.tab_context_target {
            log_tab_ux_trace(
                "context-action",
                format_args!("tab_id={}, action={}", tab_id.value(), action.trace_name()),
            );
        }

        match action {
            TabContextAction::Rename => self.rename_context_tab(),
            TabContextAction::Close => self.close_context_tab(),
            TabContextAction::CloseOther => self.close_other_tabs_from_context(),
        }
    }

    fn minimize_window(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_MINIMIZE);
        }
    }

    fn toggle_window_maximize_restore(&self) {
        let command = if self.is_maximized() {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        unsafe {
            ShowWindow(self.hwnd, command);
        }
    }

    fn close_window(&self) {
        unsafe {
            SendMessageW(self.hwnd, WM_CLOSE, 0, 0);
        }
    }

    fn save_active_tab_preset(&mut self) {
        let Some(tab_id) = self.tab_context_target.or_else(|| self.app.active_tab_id()) else {
            self.set_status("저장할 탭이 없습니다.");
            return;
        };

        let initial_name = next_tab_preset_name(self.app.list_tab_presets().len());
        let programs = match self.program_specs_for_tab(tab_id) {
            Ok(programs) => programs,
            Err(error) => {
                self.report_app_error(error);
                return;
            }
        };
        let mut preset = match self.app.tab_preset_for_tab(tab_id, initial_name, programs) {
            Ok(preset) => preset,
            Err(error) => {
                self.report_app_error(error);
                return;
            }
        };
        let program_count = match self.prompt_tab_preset_edit_dialog(&mut preset) {
            Ok(Some(program_count)) => program_count,
            Ok(None) => {
                self.set_status_i18n(
                    "Tab preset save was canceled.",
                    "탭 preset 저장을 취소했습니다.",
                );
                return;
            }
            Err(error) => {
                self.report_entry_error(
                    ui_text(
                        self.language(),
                        "Tab preset edit input",
                        "탭 preset 편집 입력",
                    ),
                    error,
                );
                return;
            }
        };
        match self.app.save_tab_preset_value(preset) {
            Ok(preset) => {
                self.record_workspace_change();
                self.refresh_main_menu();
                self.set_status(&tab_preset_save_success_status_text(
                    self.language(),
                    preset.name(),
                    program_count,
                ));
            }
            Err(error) => self.report_app_error(error),
        };
    }

    fn program_specs_for_tab(
        &mut self,
        tab_id: TabId,
    ) -> Result<HashMap<RegionId, ExternalProgramSpec>, AppError> {
        let placements = self
            .app
            .state()
            .workspace()
            .placements_for_tab(tab_id)?
            .to_vec();
        let mut programs = HashMap::with_capacity(placements.len());

        for placement in placements {
            let title = window_title_for_program_spec(placement.hwnd());
            let program = self
                .app
                .controller_mut()
                .program_spec_for_snapshot(placement.snapshot(), title)?;
            programs.insert(placement.region_id(), program);
        }

        Ok(programs)
    }

    fn show_tab_preset_menu(&mut self) {
        let Some(target_tab) = self.tab_context_target.or_else(|| self.app.active_tab_id()) else {
            self.set_status("탭 preset을 적용할 탭이 없습니다.");
            return;
        };

        if self.app.list_tab_presets().is_empty() {
            self.set_status("저장된 탭 preset이 없습니다.");
            return;
        }

        let Some(screen_point) = self.tab_preset_menu_screen_point_for_tab(target_tab) else {
            self.set_status("탭 preset 목록 위치를 계산할 수 없습니다.");
            return;
        };

        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status("탭 preset 목록을 열 수 없습니다.");
            return;
        }

        if !append_preset_menu_items(
            menu,
            self.app
                .list_tab_presets()
                .iter()
                .map(|preset| preset.name()),
            CMD_TAB_PRESET_BASE,
            CMD_TAB_PRESET_END,
        ) {
            unsafe {
                DestroyMenu(menu);
            }
            self.set_status("탭 preset 목록 command를 만들 수 없습니다.");
            return;
        }

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        if selected == 0 || self.hwnd.is_null() {
            return;
        }

        let Some(selected_command) = u16::try_from(selected).ok() else {
            self.set_status("탭 preset 선택을 처리할 수 없습니다.");
            return;
        };
        let Some(preset_name) = self.tab_preset_name_for_command(
            selected_command,
            CMD_TAB_PRESET_BASE,
            CMD_TAB_PRESET_END,
        ) else {
            self.set_status("탭 preset 선택을 처리할 수 없습니다.");
            return;
        };
        self.apply_tab_preset_to_tab(&preset_name, target_tab);
    }

    fn show_delete_tab_preset_menu(&mut self) {
        if self.app.list_tab_presets().is_empty() {
            self.set_status("삭제할 저장된 탭 preset이 없습니다.");
            return;
        }

        let Some(screen_point) = self.tab_preset_menu_screen_point() else {
            self.set_status("탭 preset 삭제 목록 위치를 계산할 수 없습니다.");
            return;
        };

        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status("탭 preset 삭제 목록을 열 수 없습니다.");
            return;
        }

        if !append_preset_menu_items(
            menu,
            self.app
                .list_tab_presets()
                .iter()
                .map(|preset| preset.name()),
            CMD_TAB_PRESET_DELETE_BASE,
            CMD_TAB_PRESET_DELETE_END,
        ) {
            unsafe {
                DestroyMenu(menu);
            }
            self.set_status("탭 preset 삭제 목록 command를 만들 수 없습니다.");
            return;
        }

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        if selected == 0 || self.hwnd.is_null() {
            return;
        }

        let Some(selected_command) = u16::try_from(selected).ok() else {
            self.set_status("탭 preset 삭제 선택을 처리할 수 없습니다.");
            return;
        };
        let Some(preset_name) = self.tab_preset_name_for_command(
            selected_command,
            CMD_TAB_PRESET_DELETE_BASE,
            CMD_TAB_PRESET_DELETE_END,
        ) else {
            self.set_status("탭 preset 삭제 선택을 처리할 수 없습니다.");
            return;
        };
        self.delete_tab_preset(&preset_name);
    }

    fn show_edit_tab_preset_menu(&mut self) {
        if self.app.list_tab_presets().is_empty() {
            self.set_status_i18n(
                "There are no saved tab presets to edit.",
                "편집할 저장된 탭 preset이 없습니다.",
            );
            return;
        }

        let Some(screen_point) = self.tab_preset_menu_screen_point() else {
            self.set_status_i18n(
                "Tab preset edit list position could not be calculated.",
                "탭 preset 편집 목록 위치를 계산할 수 없습니다.",
            );
            return;
        };

        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status_i18n(
                "Tab preset edit list could not be opened.",
                "탭 preset 편집 목록을 열 수 없습니다.",
            );
            return;
        }

        if !append_preset_menu_items(
            menu,
            self.app
                .list_tab_presets()
                .iter()
                .map(|preset| preset.name()),
            CMD_TAB_PRESET_EDIT_BASE,
            CMD_TAB_PRESET_EDIT_END,
        ) {
            unsafe {
                DestroyMenu(menu);
            }
            self.set_status_i18n(
                "Tab preset edit list command could not be created.",
                "탭 preset 편집 목록 command를 만들 수 없습니다.",
            );
            return;
        }

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        if selected == 0 || self.hwnd.is_null() {
            return;
        }

        let Some(selected_command) = u16::try_from(selected).ok() else {
            self.set_status_i18n(
                "Tab preset edit selection could not be handled.",
                "탭 preset 편집 선택을 처리할 수 없습니다.",
            );
            return;
        };
        let Some(preset_name) = self.tab_preset_name_for_command(
            selected_command,
            CMD_TAB_PRESET_EDIT_BASE,
            CMD_TAB_PRESET_EDIT_END,
        ) else {
            self.set_status_i18n(
                "Tab preset edit selection could not be handled.",
                "탭 preset 편집 선택을 처리할 수 없습니다.",
            );
            return;
        };
        self.edit_tab_preset(&preset_name);
    }

    fn tab_preset_menu_screen_point(&self) -> Option<ScreenPoint> {
        self.client_point_to_screen(ClientPoint {
            x: TAB_BAR_LEFT,
            y: TAB_BAR_HEIGHT,
        })
    }

    fn tab_preset_menu_screen_point_for_tab(&self, tab_id: TabId) -> Option<ScreenPoint> {
        self.tab_menu_client_point_for_tab(tab_id)
            .and_then(|point| self.client_point_to_screen(point))
            .or_else(|| self.tab_preset_menu_screen_point())
    }

    fn tab_menu_client_point_for_tab(&self, tab_id: TabId) -> Option<ClientPoint> {
        let layout = self.current_tab_strip_layout()?;
        let index = self
            .app
            .state()
            .workspace()
            .tabs()
            .iter()
            .position(|tab| tab.id() == tab_id)?;
        let rect = tab_rect_for_index(layout, index)?;
        Some(ClientPoint {
            x: rect.left,
            y: rect.bottom,
        })
    }

    fn edit_tab_preset(&mut self, preset_name: &str) {
        let Some(mut preset) = self
            .app
            .list_tab_presets()
            .iter()
            .find(|preset| preset.name() == preset_name)
            .cloned()
        else {
            let error = AppError::from(crate::domain::DomainError::TabPresetNotFound(
                preset_name.to_owned(),
            ));
            self.status.replace(tab_preset_edit_failure_status_text(
                self.language(),
                preset_name,
                &error,
            ));
            log_app_error(&error);
            self.invalidate();
            return;
        };

        let preset_name = preset.name().to_owned();
        let edited_program_count = match self.prompt_tab_preset_edit_dialog(&mut preset) {
            Ok(Some(program_count)) => program_count,
            Ok(None) => {
                self.set_status_i18n(
                    "Tab preset edit was canceled.",
                    "탭 preset 편집을 취소했습니다.",
                );
                return;
            }
            Err(error) => {
                self.report_entry_error(
                    ui_text(
                        self.language(),
                        "Tab preset edit input",
                        "탭 preset 편집 입력",
                    ),
                    error,
                );
                return;
            }
        };
        if self.hwnd.is_null() {
            return;
        }

        match self.app.replace_tab_preset(&preset_name, preset) {
            Ok(preset) => {
                self.record_workspace_change();
                self.refresh_main_menu();
                self.set_status(&tab_preset_edit_success_status_text(
                    self.language(),
                    preset.name(),
                    edited_program_count,
                ));
            }
            Err(error) => {
                self.status.replace(tab_preset_edit_failure_status_text(
                    self.language(),
                    &preset_name,
                    &error,
                ));
                log_app_error(&error);
                self.invalidate();
            }
        }
    }

    fn prompt_tab_preset_edit_dialog(
        &mut self,
        preset: &mut TabPreset,
    ) -> Result<Option<usize>, EntryError> {
        let program_specs = preset.program_specs();

        let edit_result = match prompt_tab_preset_edit(
            self.hwnd,
            ui_text(self.language(), "Edit tab preset", "탭 프리셋 편집"),
            self.language(),
            preset.name(),
            &program_specs,
            ui_text(self.language(), "OK", "확인"),
            ui_text(self.language(), "Cancel", "취소"),
        ) {
            Ok(Some(result)) => result,
            Ok(None) => return Ok(None),
            Err(error) => return Err(error),
        };
        if self.hwnd.is_null() {
            return Ok(None);
        }
        let (name, edited_programs) = edit_result.into_parts();
        let edited_program_count = edited_programs.len();
        preset
            .rename(name)
            .map_err(|error| EntryError::App(error.into()))?;
        preset.replace_program_specs(edited_programs);

        Ok(Some(edited_program_count))
    }

    fn delete_tab_preset(&mut self, preset_name: &str) {
        match self.app.delete_tab_preset(preset_name) {
            Ok(preset) => {
                self.record_workspace_change();
                self.refresh_main_menu();
                self.set_status(&tab_preset_delete_success_status_text(
                    self.language(),
                    preset.name(),
                ));
            }
            Err(error) => {
                self.status.replace(tab_preset_delete_failure_status_text(
                    self.language(),
                    preset_name,
                    &error,
                ));
                log_app_error(&error);
                self.invalidate();
            }
        }
    }

    fn apply_tab_preset_to_active_tab(&mut self, preset_name: &str) {
        let Some(active_tab) = self.app.active_tab_id() else {
            self.set_status("탭 preset을 적용할 활성 탭이 없습니다.");
            return;
        };
        self.apply_tab_preset_to_tab(preset_name, active_tab);
    }

    fn apply_tab_preset_to_tab(&mut self, preset_name: &str, target_tab: TabId) {
        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_status("탭 preset 적용 실패: 작업 영역 좌표를 계산할 수 없습니다.");
            return;
        };

        let target_label = self.tab_status_label(target_tab);
        match self
            .app
            .apply_tab_preset_to_tab_replacing_existing_placements(preset_name, target_tab, bounds)
        {
            Ok((report, undocked)) => {
                self.record_workspace_change();
                self.apply_tab_preset_ui_effects(&report);
                let restore = self.restore_tab_preset_programs(&report, undocked);
                if restore.docked > 0 {
                    self.record_workspace_change();
                }
                self.refresh_main_menu();
                let target_label = self.tab_status_label(report.target_tab_id());
                self.set_status(&tab_preset_apply_success_status_text(
                    self.language(),
                    &report,
                    &target_label,
                    undocked,
                    &restore,
                ));
            }
            Err(error) => {
                self.status.replace(tab_preset_apply_failure_status_text(
                    self.language(),
                    preset_name,
                    &target_label,
                    &error,
                ));
                log_app_error(&error);
                self.invalidate();
            }
        }
    }

    fn apply_tab_preset_ui_effects(&mut self, report: &TabPresetApplication) {
        self.active_region = report
            .active_regions()
            .and_then(|regions| regions.first().map(|region| region.region_id()));
        self.cancel_pointer_state_after_preset_apply();
        self.invalidate_paint_layout_cache();
        self.invalidate_paint_occupied_regions();
        self.sync_tab_overflow();
        self.invalidate();
    }

    fn restore_tab_preset_programs(
        &mut self,
        report: &TabPresetApplication,
        undocked: usize,
    ) -> TabPresetProgramRestoreReport {
        self.cancel_tab_preset_program_restore();

        let mut restore = TabPresetProgramRestoreReport::new(report.program_placements().len());
        let mut pending = Vec::new();
        for placement in report.program_placements() {
            let program = placement.program();
            let label = tab_preset_program_label(program);
            match start_tab_preset_program(program) {
                Ok(LaunchedTabPresetProgram { path, child }) => {
                    pending.push(PendingTabPresetProgramRestore::new(
                        label,
                        placement.region_id(),
                        path,
                        child,
                    ));
                }
                Err(error) => {
                    restore.failures.push(TabPresetProgramFailure {
                        label,
                        message: error.user_message(self.language()),
                    });
                }
            }
        }

        if !pending.is_empty() {
            let now = Instant::now();
            let state = TabPresetProgramRestoreState::new(
                TabPresetProgramRestoreRequest {
                    preset_name: report.preset_name().to_owned(),
                    target: self.tab_status_label(report.target_tab_id()),
                    target_tab_id: report.target_tab_id(),
                    undocked,
                    report: restore.clone(),
                    pending,
                    deadline: now + TAB_PRESET_WINDOW_WAIT,
                },
                now,
                self.hwnd,
            );

            if self.start_tab_preset_program_restore_timer() {
                self.tab_preset_program_restore = Some(state);
            } else {
                restore = tab_preset_program_restore_timer_failure_report(state, self.language());
            }
        }

        self.invalidate_paint_occupied_regions();
        self.invalidate();
        restore
    }

    fn poll_tab_preset_program_restore(&mut self) {
        let Some(mut state) = self.tab_preset_program_restore.take() else {
            self.stop_tab_preset_program_restore_timer();
            return;
        };

        let mut docked = 0usize;
        let now = Instant::now();
        state.observe_child_statuses();
        docked += self.dock_tab_preset_program_matches(&mut state);

        if !state.pending.is_empty() && state.has_due_fallback_scan(now) {
            state.refresh_tracked_processes(now);
            if state.scan_windows(now) {
                docked += self.dock_tab_preset_program_matches(&mut state);
            }
        }

        if docked > 0 {
            self.record_workspace_change();
        }

        if now >= state.deadline {
            state.window_search.clear();
            for pending in state.pending.drain(..) {
                state.report.failures.push(TabPresetProgramFailure {
                    label: pending.label,
                    message: TabPresetProgramLaunchError::WindowNotFound {
                        path: pending.path,
                        process_id: pending.process_id,
                    }
                    .user_message(self.language()),
                });
            }
        }

        if state.pending.is_empty() {
            self.finish_tab_preset_program_restore(state);
        } else {
            self.tab_preset_program_restore = Some(state);
        }
    }

    fn dock_tab_preset_program_matches(
        &mut self,
        state: &mut TabPresetProgramRestoreState,
    ) -> usize {
        if !state.has_window_matches() {
            return 0;
        }

        let mut docked = 0usize;
        let mut index = 0usize;
        while index < state.pending.len() {
            let Some(hwnd) = state.pending[index].matching_hwnd(&state.window_search) else {
                index += 1;
                continue;
            };

            let pending = state.remove_pending(index);
            match WindowHandle::new(hwnd as isize) {
                Ok(hwnd) => {
                    let Some(bounds) = self.layout_bounds_screen() else {
                        state.report.failures.push(TabPresetProgramFailure {
                            label: pending.label,
                            message: if self.language() == UiLanguage::English {
                                "workspace bounds could not be calculated".to_owned()
                            } else {
                                "작업 영역 좌표를 계산할 수 없습니다".to_owned()
                            },
                        });
                        continue;
                    };

                    match self.app.place_window(
                        state.target_tab_id,
                        pending.region_id,
                        hwnd,
                        bounds,
                    ) {
                        Ok(()) => {
                            state.report.docked += 1;
                            docked += 1;
                        }
                        Err(error) => {
                            log_app_error(&error);
                            state.report.failures.push(TabPresetProgramFailure {
                                label: pending.label,
                                message: app_error_message(self.language(), &error),
                            });
                        }
                    }
                }
                Err(source) => {
                    state.report.failures.push(TabPresetProgramFailure {
                        label: pending.label,
                        message: TabPresetProgramLaunchError::InvalidWindowHandle {
                            path: pending.path,
                            source,
                        }
                        .user_message(self.language()),
                    });
                }
            }
        }

        docked
    }

    fn finish_tab_preset_program_restore(&mut self, state: TabPresetProgramRestoreState) {
        self.stop_tab_preset_program_restore_timer();
        self.invalidate_paint_occupied_regions();
        self.invalidate();
        self.refresh_main_menu();
        self.set_status(&tab_preset_apply_success_status_text_for_preset(
            self.language(),
            &state.preset_name,
            &state.target,
            state.undocked,
            &state.report,
        ));
    }

    fn cancel_pointer_state_after_preset_apply(&mut self) {
        self.cancel_pending_tab_click();
        self.cancel_tab_reorder_drag();
        self.cancel_splitter_drag();
        self.last_splitter_drag_screen_point = None;
        self.splitter_drag_layout_cache = None;
        self.reset_drop_tracking();
    }

    fn show_options_menu(&mut self) {
        let Some(button_rect) = self.button_rect_for_command(CMD_OPTIONS) else {
            self.set_status_i18n(
                "Options menu position could not be calculated.",
                "옵션 메뉴 위치를 계산할 수 없습니다.",
            );
            return;
        };
        let Some(screen_point) = self.client_point_to_screen(ClientPoint {
            x: button_rect.left,
            y: button_rect.bottom,
        }) else {
            self.set_status_i18n(
                "Options menu position could not be calculated.",
                "옵션 메뉴 위치를 계산할 수 없습니다.",
            );
            return;
        };

        self.show_options_menu_at(screen_point);
    }

    fn show_options_menu_at(&mut self, screen_point: ScreenPoint) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            self.set_status_i18n(
                "Options menu could not be opened.",
                "옵션 메뉴를 열 수 없습니다.",
            );
            return;
        }

        append_checked_menu(
            menu,
            CMD_DOCK_HIDDEN_WORKSPACE_UI_TOGGLE,
            ui_text(self.language(), "Dock while hidden", "숨김 상태에서도 Dock"),
            self.workspace_options.dock_hidden_workspace_ui(),
        );
        append_separator(menu);
        append_checked_menu(
            menu,
            CMD_LANGUAGE_ENGLISH,
            ui_text(self.language(), "Language: English", "언어: 영어"),
            self.language() == UiLanguage::English,
        );
        append_checked_menu(
            menu,
            CMD_LANGUAGE_KOREAN,
            ui_text(self.language(), "Language: Korean", "언어: 한국어"),
            self.language() == UiLanguage::Korean,
        );

        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                screen_point.x,
                screen_point.y,
                0,
                self.hwnd,
                null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }

        let Some(command_id) = popup_selected_command(selected, self.hwnd) else {
            return;
        };
        self.execute_command(command_id);
    }

    fn toggle_dock_hidden_workspace_ui(&mut self) {
        let enabled = !self.workspace_options.dock_hidden_workspace_ui();
        self.workspace_options = self
            .workspace_options
            .with_dock_hidden_workspace_ui(enabled);
        self.record_workspace_options_change();
        self.refresh_main_menu();
        if enabled {
            self.set_status_i18n(
                "Docking while the workspace UI is hidden is enabled.",
                "숨김 상태에서도 Dock을 허용합니다.",
            );
        } else {
            self.set_status_i18n(
                "Docking while the workspace UI is hidden is disabled.",
                "숨김 상태 Dock을 비활성화했습니다.",
            );
        }
    }

    fn set_ui_language(&mut self, language: UiLanguage) {
        if self.language() == language {
            let status = match language {
                UiLanguage::English => "UI language is already English.",
                UiLanguage::Korean => "UI 언어가 이미 한국어입니다.",
            };
            self.set_status(status);
            return;
        }

        self.workspace_options = self.workspace_options.with_ui_language(language);
        self.record_workspace_options_change();
        self.refresh_main_menu();
        self.invalidate();
        let status = match language {
            UiLanguage::English => "UI language changed to English.",
            UiLanguage::Korean => "UI 언어를 한국어로 변경했습니다.",
        };
        self.set_status(status);
    }

    fn show_about_dialog(&mut self) {
        if let Err(error) = show_about_dialog(self.hwnd, self.language()) {
            self.report_win32_status_failure(Win32StatusFailure::new(
                error.api,
                error.last_error,
                error.user_message,
            ));
        }
    }

    fn toggle_workspace_ui(&mut self) {
        let transition = self.workspace_ui_visibility.begin_toggle();
        match self.apply_workspace_ui_window_chrome(transition.desired_visible) {
            Ok(()) => self.workspace_ui_visibility.commit(transition),
            Err(error) => {
                self.workspace_ui_visibility.rollback(transition);
                if let Err(rollback_error) =
                    self.apply_workspace_ui_window_chrome(transition.previous_visible)
                {
                    log_workspace_ui_chrome_rollback_error(rollback_error);
                }
                self.report_win32_status_failure(error);
                return;
            }
        }

        self.splitter_drag_layout_cache = None;
        self.invalidate_paint_layout_cache();

        if self.workspace_ui_visible() {
            self.set_status("작업 영역 UI를 표시했습니다.");
        } else {
            self.cancel_tab_reorder_drag();
            self.cancel_splitter_drag();
            self.reset_drop_tracking();
            self.set_status("작업 영역 UI를 숨겼습니다.");
        }

        self.sync_active_tab();
        self.invalidate();
        self.sync_splitter_overlay();
    }

    fn add_tab(&mut self) {
        let name = format!("Tab {}", self.next_tab_number);
        self.next_tab_number = self.next_tab_number.saturating_add(1);

        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };

        match self.app.add_tab(name) {
            Ok(tab_id) => {
                self.record_workspace_change();
                self.switch_tab_with_bounds(tab_id, bounds);
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn delete_tab(&mut self, tab_id: TabId) {
        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_status(
                "탭 삭제 실패: 작업 영역 좌표를 계산할 수 없습니다. Undock: 시도하지 않음",
            );
            return;
        };

        let context = self.tab_deletion_status_context(tab_id);
        match self.delete_tab_with_bounds(tab_id, bounds) {
            Ok(report) => {
                self.report_tab_deletion_summary(&report);
                self.invalidate();
            }
            Err(error) => self.report_tab_deletion_error(&context, error),
        }
    }

    fn delete_tab_with_bounds(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
    ) -> Result<TabDeletionReport, AppError> {
        let report = self.app.delete_tab(tab_id, bounds)?;
        self.record_workspace_change();
        self.after_tab_deleted(&report);
        Ok(report)
    }

    fn after_tab_deleted(&mut self, report: &TabDeletionReport) {
        if report.previous_active_tab() == Some(report.deleted_tab_id()) {
            self.active_region = None;
            self.invalidate_paint_layout_cache();
            self.invalidate_paint_occupied_regions();
        }
        if self.app.state().workspace().tabs().is_empty() {
            self.next_tab_number = next_tab_number(self.app.state().workspace().next_tab_id());
        }
        self.sync_tab_overflow();
    }

    fn rename_context_tab(&mut self) {
        let Some(tab_id) = self.tab_context_target else {
            self.set_status("이름을 변경할 탭 메뉴 대상이 없습니다.");
            return;
        };

        let current_name = match self.app.state().workspace().tab(tab_id) {
            Ok(tab) => tab.name().to_owned(),
            Err(error) => {
                self.report_app_error(AppError::from(error));
                return;
            }
        };

        let input = prompt_text_input(
            self.hwnd,
            ui_text(self.language(), "Rename tab", "탭 이름 변경"),
            ui_text(self.language(), "Tab name:", "탭 이름:"),
            &current_name,
            ui_text(self.language(), "OK", "확인"),
            ui_text(self.language(), "Cancel", "취소"),
        );
        if self.hwnd.is_null() {
            return;
        }
        match input {
            Ok(Some(name)) => match self.app.rename_tab(tab_id, name) {
                Ok(()) => {
                    self.record_workspace_change();
                    self.sync_tab_overflow();
                    log_tab_ux_trace("rename-finish", format_args!("tab_id={}", tab_id.value()));
                    self.set_status(&tab_rename_success_status_text(self.language(), tab_id));
                    self.invalidate();
                }
                Err(error) => self.report_tab_operation_error(tab_id, "이름 변경", error),
            },
            Ok(None) => {
                self.set_status(&tab_rename_cancel_status_text(self.language(), tab_id));
            }
            Err(error) => self.report_entry_error(
                ui_text(self.language(), "Tab name input", "탭 이름 입력"),
                error,
            ),
        }
    }

    fn close_context_tab(&mut self) {
        let Some(tab_id) = self.tab_context_target else {
            self.set_status("닫을 탭 메뉴 대상이 없습니다.");
            return;
        };

        self.delete_tab(tab_id);
    }

    fn close_other_tabs_from_context(&mut self) {
        let Some(target_tab_id) = self.tab_context_target else {
            self.set_status("다른 탭을 닫을 탭 메뉴 대상이 없습니다.");
            return;
        };
        let tab_ids = self.current_tab_ids();
        let Some(targets) = close_other_tab_targets(&tab_ids, target_tab_id) else {
            self.set_status(&close_other_target_missing_status_text(
                self.language(),
                target_tab_id,
            ));
            return;
        };

        if targets.is_empty() {
            self.set_status("닫을 다른 탭이 없습니다.");
            return;
        }

        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_status(close_other_bounds_failure_status_text(self.language()));
            return;
        };

        let total = targets.len();
        let mut closed_count = 0usize;
        let mut attempted = 0usize;
        let mut restored = 0usize;
        let mut missing = 0usize;
        let mut undock_failures = 0usize;
        let mut failures = Vec::new();

        // Best-effort policy: each tab deletion keeps App::delete_tab rollback semantics;
        // a failed tab is left in place and the remaining target tabs are still attempted.
        for tab_id in targets {
            match self.delete_tab_with_bounds(tab_id, bounds) {
                Ok(report) => {
                    closed_count += 1;
                    attempted += report.undock().attempted();
                    restored += report.undock().restored();
                    missing += report.undock().missing();
                    undock_failures += report.undock().failures().len();
                    log_undock_failures(report.undock());
                }
                Err(error) => {
                    log_app_error(&error);
                    failures.push(TabOperationFailure {
                        tab_id,
                        operation: "삭제",
                        message: app_error_message(self.language(), &error),
                    });
                }
            }
        }

        self.invalidate_paint_layout_cache();
        self.invalidate_paint_occupied_regions();
        self.sync_tab_overflow();
        self.status.replace(close_other_tabs_status_text_for(
            self.language(),
            target_tab_id,
            total,
            closed_count,
            self.app.active_tab_id(),
            UndockCounts {
                attempted,
                restored,
                missing,
                failures: undock_failures,
            },
            &failures,
        ));
        log_tab_ux_trace(
            "close-other-finish",
            format_args!(
                "target_tab_id={}, closed={}, total={}, failures={}, active_tab={}",
                target_tab_id.value(),
                closed_count,
                total,
                failures.len(),
                optional_tab_id_trace_text(self.app.active_tab_id())
            ),
        );
        self.invalidate();
    }

    fn switch_tab(&mut self, tab_id: TabId) {
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        self.switch_tab_with_bounds(tab_id, bounds);
    }

    fn switch_tab_with_bounds(&mut self, tab_id: TabId, bounds: Rect) {
        let context = self.tab_switch_status_context(tab_id);
        let result = self.app.switch_tab(tab_id, bounds);
        self.invalidate_paint_occupied_regions();

        match result {
            Ok(report) => {
                if report.removed_stale_placements() > 0 {
                    self.record_workspace_change();
                }
                self.active_region = None;
                self.sync_tab_overflow();
                log_tab_ux_trace(
                    "switch-finish",
                    format_args!(
                        "target_tab_id={}, previous_tab_id={}, removed_stale_target={}, removed_stale_previous={}",
                        tab_id.value(),
                        optional_tab_id_trace_text(report.previous()),
                        report.removed_stale_target_placements(),
                        report.removed_stale_previous_placements()
                    ),
                );
                self.set_status(&switch_tab_success_status_text_for(
                    self.language(),
                    &context,
                    report,
                ));
                self.invalidate();
            }
            Err(error) => self.report_switch_tab_error(&context, error),
        }
    }

    fn split_active_region(&mut self, direction: SplitDirection) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_status("활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_status("분할할 영역을 먼저 선택하세요.");
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };

        match self.app.split_region(tab_id, region_id, direction, bounds) {
            Ok(_) => {
                self.record_workspace_change();
                self.invalidate_paint_layout_cache();
                self.invalidate_workspace_body();
                self.set_status("영역을 분할했습니다.");
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn delete_active_region(&mut self) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_status("활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_status("삭제할 영역을 먼저 선택하세요.");
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };

        match self.app.delete_region(tab_id, region_id, bounds) {
            Ok(_) => {
                self.record_workspace_change();
                self.active_region = None;
                self.invalidate_paint_layout_cache();
                self.invalidate_paint_occupied_regions();
                self.sync_tab_tooltips_for_current_layout();
                self.invalidate_workspace_body();
                self.set_status("영역을 삭제했습니다.");
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn undock_active_region(&mut self) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_status("활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_status("해제할 영역을 먼저 선택하세요.");
            return;
        };

        match self.app.unregister_placement(tab_id, region_id) {
            Ok(_) => {
                self.record_workspace_change();
                self.invalidate_paint_occupied_regions();
                self.sync_tab_tooltips_for_current_layout();
                self.invalidate_workspace_body();
                self.set_status("외부 윈도우 배치를 해제했습니다.");
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn on_drop_move_size_event(&mut self, event: u32, hwnd: HWND) {
        match event {
            EVENT_SYSTEM_MOVESIZESTART
                if self.external_root_candidate_from_hwnd(hwnd).is_some() =>
            {
                self.start_drop_poll_timer();
                self.poll_external_drop();
            }
            EVENT_SYSTEM_MOVESIZEEND
                if self.drop_poll_timer_active || self.drop_tracker.is_tracking() =>
            {
                self.poll_external_drop();
                if self.drop_tracker.is_tracking() {
                    self.start_drop_poll_timer();
                } else {
                    self.stop_drop_poll_timer();
                }
            }
            _ => {}
        }
    }

    fn on_window_name_change_event(&mut self, hwnd: HWND) {
        let raw = hwnd as isize;
        let Some(key) = self.tab_tooltip_sync_key.as_ref() else {
            return;
        };
        if !key.contains_window(raw) {
            return;
        }

        let title = self.docked_window_title_for_tooltip_raw(hwnd);
        let specs = {
            let Some(key) = self.tab_tooltip_sync_key.as_mut() else {
                return;
            };
            key.tooltip_sync_specs_for_window_title_change(raw, title)
        };

        for (tab_id, spec) in specs {
            if self.tab_tooltip.sync_tab(self.hwnd, tab_id, spec) {
                continue;
            }

            self.clear_tab_tooltip_sync_key();
            self.sync_tab_tooltips_for_current_layout();
            return;
        }
    }

    fn on_tab_preset_program_window_event(&mut self, hwnd: HWND) {
        let Some(state) = self.tab_preset_program_restore.as_mut() else {
            return;
        };

        state.record_window_event(hwnd);
    }

    fn poll_external_drop(&mut self) {
        if self.frame_state.is_minimized() {
            self.reset_drop_tracking();
            return;
        }

        let is_down = unsafe { (GetAsyncKeyState(VK_LBUTTON as i32) & 0x8000u16 as i16) != 0 };
        let Some(cursor) = cursor_position() else {
            if !is_down {
                self.reset_drop_tracking();
            }
            return;
        };

        if is_down {
            self.track_external_press(cursor);
            return;
        }

        if let Some(hwnd) = self.finish_external_press() {
            self.try_place_drop(hwnd, cursor);
        }
        self.stop_drop_poll_timer();
    }

    fn track_external_press(&mut self, cursor: ScreenPoint) {
        self.drop_tracker.begin_press();
        if self.observe_drop_candidate_motion() {
            return;
        }

        if !self.drop_tracker.needs_candidate() {
            return;
        }

        let Some(hwnd) = self.external_window_at(cursor) else {
            return;
        };

        self.select_active_region_for_placed_window(hwnd);

        let Some(rect) = self.external_window_rect(hwnd) else {
            return;
        };

        self.drop_tracker.set_candidate(hwnd, rect);
    }

    fn observe_drop_candidate_motion(&mut self) -> bool {
        let Some(hwnd) = self.drop_tracker.candidate_hwnd() else {
            return false;
        };

        let Some(rect) = self.external_window_rect(hwnd) else {
            self.drop_tracker.suppress_drop();
            return true;
        };

        self.drop_tracker
            .observe_candidate_rect(rect, DROP_WINDOW_MOVE_THRESHOLD);
        true
    }

    fn finish_external_press(&mut self) -> Option<WindowHandle> {
        self.observe_drop_candidate_motion();
        self.drop_tracker.finish_press()
    }

    fn try_place_drop(&mut self, hwnd: WindowHandle, cursor: ScreenPoint) {
        let Some(tab_id) = self.app.active_tab_id() else {
            return;
        };

        if !drop_uses_workspace_hit_test(self.workspace_ui_visible(), self.workspace_options) {
            self.try_detach_drop_outside_docker(hwnd, cursor);
            return;
        }

        let Some(bounds) = self.layout_bounds_screen() else {
            self.try_detach_drop_outside_docker(hwnd, cursor);
            return;
        };

        match self.app.hit_test_region(tab_id, bounds, cursor.x, cursor.y) {
            Ok(Some(region_id)) => self.try_place_drop_in_region(tab_id, region_id, hwnd, bounds),
            Ok(None) => self.try_detach_drop_outside_docker(hwnd, cursor),
            Err(error) => self.report_app_error(error),
        }
    }

    fn try_place_drop_in_region(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        bounds: Rect,
    ) {
        let source_region_id = match self.app.active_tab_region_for_window(hwnd) {
            Ok(region_id) => region_id,
            Err(error) => {
                self.report_app_error(error);
                return;
            }
        };

        match self.app.register_placement(tab_id, region_id, hwnd, bounds) {
            Ok(registration) => {
                self.record_workspace_change();
                let target_region_id = registration.target_region_id();
                self.active_region = Some(target_region_id);
                self.invalidate_paint_occupied_regions();
                self.sync_tab_tooltips_for_current_layout();
                self.invalidate_workspace_body();
                match registration {
                    PlacementRegistration::Placed { .. } => {
                        self.set_status("외부 윈도우를 영역에 배치했습니다.");
                    }
                    PlacementRegistration::Moved { .. } => {
                        self.set_status("외부 윈도우를 다른 영역으로 이동했습니다.");
                    }
                    PlacementRegistration::Resynced { .. } => {
                        self.set_status("외부 윈도우를 현재 영역에 다시 맞췄습니다.");
                    }
                }
            }
            Err(error) => self.report_drop_registration_error(source_region_id, region_id, error),
        }
    }

    fn try_detach_drop_outside_docker(&mut self, hwnd: WindowHandle, cursor: ScreenPoint) {
        if self.main_window_contains(cursor) {
            return;
        }

        match self.app.active_tab_region_for_window(hwnd) {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(error) => {
                self.report_app_error(error);
                return;
            }
        }

        let Some(rect) = self
            .external_window_rect(hwnd)
            .and_then(TrackedWindowRect::to_domain_rect)
        else {
            self.set_status("외부 윈도우 detach 실패: 현재 위치를 조회할 수 없습니다.");
            return;
        };

        match self.app.detach_active_placement_at(hwnd, rect) {
            Ok(Some(UndockStatus::Restored)) => {
                self.record_workspace_change();
                self.active_region = None;
                self.invalidate_paint_occupied_regions();
                self.sync_tab_tooltips_for_current_layout();
                self.invalidate_workspace_body();
                self.set_status("외부 윈도우를 현재 위치에서 배치 해제했습니다.");
            }
            Ok(Some(UndockStatus::WindowMissing)) => {
                self.record_workspace_change();
                self.active_region = None;
                self.invalidate_paint_occupied_regions();
                self.sync_tab_tooltips_for_current_layout();
                self.invalidate_workspace_body();
                self.set_status("외부 윈도우가 유효하지 않아 배치 정보를 제거했습니다.");
            }
            Ok(None) => {}
            Err(error) => self.report_app_error(error),
        }
    }

    fn main_window_contains(&self, cursor: ScreenPoint) -> bool {
        tracked_window_rect(self.hwnd).is_some_and(|rect| rect.contains_point(cursor))
    }

    fn external_window_at(&mut self, cursor: ScreenPoint) -> Option<WindowHandle> {
        let point = POINT {
            x: cursor.x,
            y: cursor.y,
        };
        let hwnd = unsafe { WindowFromPoint(point) };
        if hwnd.is_null() {
            return None;
        }

        let candidate = self.external_root_candidate_from_hwnd(hwnd)?;
        if candidate.docker_owned && !self.is_active_tab_placement(candidate.hwnd) {
            return None;
        }

        Some(candidate.hwnd)
    }

    fn external_window_rect(&self, hwnd: WindowHandle) -> Option<TrackedWindowRect> {
        let raw = hwnd.raw() as HWND;
        if raw.is_null() || unsafe { IsWindow(raw) } == 0 {
            return None;
        }

        tracked_window_rect(raw)
    }

    fn external_root_from_hwnd(&self, hwnd: HWND) -> Option<WindowHandle> {
        let candidate = self.external_root_candidate_from_hwnd(hwnd)?;
        (!candidate.docker_owned).then_some(candidate.hwnd)
    }

    fn external_root_candidate_from_hwnd(&self, hwnd: HWND) -> Option<ExternalRootCandidate> {
        external_root_candidate_from_hwnd_with(
            hwnd,
            self.hwnd,
            |candidate| unsafe { GetAncestor(candidate, GA_ROOT) },
            |candidate| unsafe { GetWindow(candidate, GW_OWNER) },
        )
    }

    fn is_active_tab_placement(&mut self, hwnd: WindowHandle) -> bool {
        match self.app.active_tab_region_for_window(hwnd) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(error) => {
                self.report_app_error(error);
                false
            }
        }
    }

    fn select_active_region_for_placed_window(&mut self, hwnd: WindowHandle) -> bool {
        match self.app.active_tab_region_for_window(hwnd) {
            Ok(Some(region_id)) => {
                if self.active_region != Some(region_id) {
                    let previous = self.active_region;
                    self.active_region = Some(region_id);
                    if let Some(tab_id) = self.app.active_tab_id() {
                        self.invalidate_active_region_change(tab_id, previous, Some(region_id));
                    }
                    self.set_status(docked_window_selection_status_text_for(self.language()));
                }
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.report_app_error(error);
                false
            }
        }
    }

    fn sync_active_tab(&mut self) {
        if self.frame_state.is_minimized() {
            return;
        }

        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };

        if self.frame_state.active_tab_show_pending() {
            self.show_active_tab(bounds);
            return;
        }

        if self.active_tab_sync_cache_matches(bounds) {
            return;
        }

        let cache_offset = self.active_tab_sync_cache_offset(bounds);
        let cached_layout = cache_offset.and_then(|_| {
            self.active_tab_sync_cache.as_ref().map(|cache| {
                CachedActiveTabLayout::with_region_rects(
                    cache.bounds,
                    &cache.regions,
                    &cache.rects_by_region_id,
                )
            })
        });

        let result = self
            .app
            .sync_active_tab_with_cached_layout(bounds, cached_layout);

        match result {
            Ok(report) => {
                let removed_stale_placements = report.removed_stale_placements();
                if removed_stale_placements > 0 {
                    self.record_workspace_change();
                    self.invalidate_paint_occupied_regions();
                }
                if let Some(regions) = report.into_computed_regions() {
                    self.remember_active_tab_sync_regions(bounds, regions);
                } else if let Some((dx, dy)) = cache_offset {
                    if !self.shift_active_tab_sync_cache(bounds, dx, dy) {
                        self.remember_active_tab_sync(bounds);
                    }
                } else {
                    self.remember_active_tab_sync(bounds);
                }
            }
            Err(error) => {
                self.invalidate_paint_occupied_regions();
                self.report_app_error(error);
            }
        }
    }

    fn show_active_tab(&mut self, bounds: Rect) {
        let result = self.app.show_active_tab(bounds);

        match result {
            Ok(removed_stale_placements) => {
                if removed_stale_placements > 0 {
                    self.record_workspace_change();
                    self.invalidate_paint_occupied_regions();
                }
                self.remember_active_tab_sync(bounds);
                self.frame_state.complete_active_tab_show();
            }
            Err(error) => {
                self.invalidate_paint_occupied_regions();
                self.report_app_error(error);
            }
        }
    }

    fn sync_splitter_overlay(&mut self) {
        if !splitter_overlay_should_show(
            self.workspace_ui_visible(),
            self.workspace_options.dock_hidden_workspace_ui(),
            self.frame_state.is_minimized(),
            self.dragging_splitter.is_some()
                || self.pending_tab_click.is_some()
                || self.tab_reorder_drag.is_some(),
            ctrl_key_is_down(),
        ) {
            self.hide_splitter_overlay();
            return;
        }

        if let Err(error) = self.refresh_splitter_overlay_rect_cache() {
            self.hide_splitter_overlay();
            self.report_app_error(error);
            return;
        }

        let rects = match self.splitter_overlay_rect_cache.as_ref() {
            Some(cache) => cache.rects.as_slice(),
            None => &[],
        };

        match self.splitter_overlay.sync(self.hwnd, rects) {
            Ok(true) => {
                if !self.start_splitter_overlay_poll_timer() {
                    self.hide_splitter_overlay();
                }
            }
            Ok(false) => self.stop_splitter_overlay_poll_timer(),
            Err(error) => {
                self.hide_splitter_overlay();
                self.report_win32_status_failure(error);
            }
        }
    }

    fn refresh_splitter_overlay_rect_cache(&mut self) -> Result<(), AppError> {
        let Some(active_tab) = self.app.active_tab_id() else {
            self.invalidate_splitter_overlay_rect_cache();
            return Ok(());
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            self.invalidate_splitter_overlay_rect_cache();
            return Ok(());
        };

        if matches!(
            self.splitter_overlay_rect_cache.as_ref(),
            Some(cache) if cache.tab_id == active_tab && cache.bounds == bounds
        ) {
            return Ok(());
        }

        let splitters = self
            .app
            .splitter_rects(active_tab, bounds, SPLITTER_HIT_TOLERANCE)?;

        let mut rects = self
            .splitter_overlay_rect_cache
            .take()
            .map(|mut cache| {
                cache.rects.clear();
                cache.rects
            })
            .unwrap_or_default();
        rects.reserve(splitters.len());
        rects.extend(splitters.into_iter().map(|splitter| splitter.rect()));
        self.splitter_overlay_rect_cache = Some(SplitterOverlayRectCache {
            tab_id: active_tab,
            bounds,
            rects,
        });

        Ok(())
    }

    fn invalidate_splitter_overlay_rect_cache(&mut self) {
        self.splitter_overlay_rect_cache = None;
    }

    fn active_tab_sync_cache_matches(&mut self, bounds: Rect) -> bool {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.active_tab_sync_cache = None;
            return false;
        };

        matches!(
            self.active_tab_sync_cache.as_ref(),
            Some(cache) if cache.tab_id == tab_id && cache.bounds == bounds
        )
    }

    fn remember_active_tab_sync(&mut self, bounds: Rect) {
        self.remember_active_tab_sync_regions(bounds, Vec::new());
    }

    fn remember_active_tab_sync_regions(&mut self, bounds: Rect, regions: Vec<RegionRect>) {
        self.active_tab_sync_cache = self
            .app
            .active_tab_id()
            .map(|tab_id| ActiveTabSyncCache::new(tab_id, bounds, regions));
    }

    fn active_tab_sync_cache_offset(&mut self, bounds: Rect) -> Option<(i32, i32)> {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.active_tab_sync_cache = None;
            return None;
        };
        let cache = self.active_tab_sync_cache.as_ref()?;
        if cache.tab_id != tab_id
            || cache.regions.is_empty()
            || cache.bounds.width() != bounds.width()
            || cache.bounds.height() != bounds.height()
        {
            return None;
        }

        Some((
            bounds.left().checked_sub(cache.bounds.left())?,
            bounds.top().checked_sub(cache.bounds.top())?,
        ))
    }

    fn shift_active_tab_sync_cache(&mut self, bounds: Rect, dx: i32, dy: i32) -> bool {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.active_tab_sync_cache = None;
            return false;
        };
        let Some(cache) = self.active_tab_sync_cache.as_mut() else {
            return false;
        };
        if cache.tab_id != tab_id {
            return false;
        }

        cache.rects_by_region_id.clear();
        for region in &mut cache.regions {
            let region_id = region.region_id();
            let Ok(rect) = region.rect().translated(dx, dy) else {
                return false;
            };
            *region = RegionRect::new(region_id, rect);
            cache.rects_by_region_id.entry(region_id).or_insert(rect);
        }
        cache.bounds = bounds;
        true
    }

    fn layout_bounds_screen(&self) -> Option<Rect> {
        let client = self.client_rect()?;
        let metrics = layout_metrics(client, self.workspace_ui_visible())?;

        let top_left = self.client_point_to_screen(ClientPoint {
            x: 0,
            y: metrics.content_top,
        })?;
        Rect::new(top_left.x, top_left.y, metrics.width, metrics.height).ok()
    }

    fn layout_bounds_client(&self) -> Option<Rect> {
        layout_bounds_for_client_rect(self.client_rect()?, self.workspace_ui_visible())
    }

    fn workspace_body_rect(&self) -> Option<UiRect> {
        workspace_body_rect_for_client(self.client_rect()?, self.workspace_ui_visible())
    }

    fn status_bar_rect(&self) -> Option<UiRect> {
        status_bar_rect_for_client(self.client_rect()?, self.workspace_ui_visible())
    }

    fn region_client_rect(&self, tab_id: TabId, region_id: RegionId) -> Option<UiRect> {
        let client = self.client_rect()?;
        let metrics = layout_metrics(client, self.workspace_ui_visible())?;
        let bounds = Rect::new(0, 0, metrics.width, metrics.height).ok()?;
        let cache_key = PaintLayoutCacheKey {
            tab_id,
            bounds,
            content_top: metrics.content_top,
        };
        if let Some(rect) = self.paint.layout_cache.region_rect(cache_key, region_id) {
            return rect.intersect(client);
        }

        let rect = self
            .app
            .state()
            .workspace()
            .find_region_rect(tab_id, region_id, bounds, DEFAULT_MIN_REGION_SIZE)
            .ok()?;
        layout_rect_to_client_rect(metrics.content_top, bounds, rect)?.intersect(client)
    }

    fn top_bar_height(&self) -> i32 {
        top_bar_height(self.workspace_ui_visible())
    }

    fn client_rect(&self) -> Option<UiRect> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetClientRect(self.hwnd, &mut rect) };
        if ok == 0 {
            None
        } else {
            Some(UiRect::from_rect(rect))
        }
    }

    fn client_point_to_screen(&self, point: ClientPoint) -> Option<ScreenPoint> {
        let mut raw = POINT {
            x: point.x,
            y: point.y,
        };
        let ok = unsafe { ClientToScreen(self.hwnd, &mut raw) };
        if ok == 0 {
            None
        } else {
            Some(ScreenPoint { x: raw.x, y: raw.y })
        }
    }

    fn sync_tab_overflow(&mut self) {
        let Some(client) = self.client_rect() else {
            self.tab_overflow_first_visible_index = 0;
            return;
        };

        let layout = self.sync_tab_overflow_for_client(client);
        self.sync_tab_tooltips(client, layout);
    }

    fn sync_tab_overflow_for_client(&mut self, client: UiRect) -> TabStripLayout {
        let layout = self.tab_strip_layout_for_client(client);
        self.tab_overflow_first_visible_index = layout.first_visible_index;
        layout
    }

    fn sync_tab_tooltips(&mut self, client: UiRect, layout: TabStripLayout) {
        if self.hwnd.is_null() {
            return;
        }

        let layout_key = self.tab_tooltip_sync_layout_key(client, layout);
        if self.tab_tooltip_sync_key_matches_layout(layout_key) {
            return;
        }

        let (key, specs) = self.visible_tab_tooltip_specs_with_key(layout, layout_key);
        if self.tab_tooltip.sync(self.hwnd, specs) {
            self.set_tab_tooltip_sync_key(key);
        } else {
            self.clear_tab_tooltip_sync_key();
        }
    }

    fn set_tab_tooltip_sync_key(&mut self, key: TabTooltipSyncKey) {
        WINDOW_NAME_CHANGE_EVENT_ROUTER.replace_interested_hwnds(key.window_hwnds());
        self.tab_tooltip_sync_key = Some(key);
    }

    fn clear_tab_tooltip_sync_key(&mut self) {
        WINDOW_NAME_CHANGE_EVENT_ROUTER.clear_interested_hwnds();
        self.tab_tooltip_sync_key = None;
    }

    fn sync_tab_tooltips_for_current_layout(&mut self) {
        let Some((client, layout)) = self.current_tab_strip_layout_with_client() else {
            return;
        };

        self.sync_tab_tooltips(client, layout);
    }

    fn tab_tooltip_sync_layout_key(
        &self,
        client: UiRect,
        layout: TabStripLayout,
    ) -> TabTooltipSyncLayoutKey {
        TabTooltipSyncLayoutKey::new(
            self.language(),
            self.workspace_change_generation,
            client,
            layout.tab_count,
            layout.first_visible_index,
            layout.visible_end_index(),
        )
    }

    fn tab_tooltip_sync_key_matches_layout(&self, layout: TabTooltipSyncLayoutKey) -> bool {
        self.tab_tooltip_sync_key
            .as_ref()
            .is_some_and(|key| key.matches_layout(layout))
    }

    fn visible_tab_tooltip_specs_with_key(
        &self,
        layout: TabStripLayout,
        layout_key: TabTooltipSyncLayoutKey,
    ) -> (TabTooltipSyncKey, Vec<TabTooltipSpec>) {
        let tabs = self.app.state().workspace().tabs();
        let visible_tab_count = layout
            .visible_end_index()
            .saturating_sub(layout.first_visible_index);
        let mut key = TabTooltipSyncKey::new(layout_key, visible_tab_count);
        let mut specs = Vec::with_capacity(visible_tab_count);

        for index in layout.first_visible_index..layout.visible_end_index() {
            let Some(tab) = tabs.get(index) else {
                break;
            };
            let Some(rect) = tab_rect_for_index(layout, index) else {
                continue;
            };
            let placements = tab.placements();
            let mut placement_hwnds = Vec::with_capacity(placements.len());
            let mut placement_titles = Vec::with_capacity(placements.len());
            for placement in placements {
                let hwnd = placement.hwnd();
                placement_hwnds.push(hwnd.raw());
                placement_titles.push(self.docked_window_title_for_tooltip(hwnd));
            }

            let spec =
                TabTooltipSyncKeySpec::new(tab.id(), rect, placement_hwnds, placement_titles);
            if let Some(tooltip_spec) = spec.tooltip_spec() {
                specs.push(tooltip_spec);
            }
            key.tabs.push(spec);
        }

        (key, specs)
    }

    fn docked_window_title_for_tooltip(&self, hwnd: WindowHandle) -> Option<String> {
        self.docked_window_title_for_tooltip_raw(hwnd.raw() as HWND)
    }

    fn docked_window_title_for_tooltip_raw(&self, raw: HWND) -> Option<String> {
        if raw.is_null() || unsafe { IsWindow(raw) } == 0 {
            return None;
        }

        match read_window_text(raw) {
            Ok(title) if title.trim().is_empty() => {
                Some(ui_text(self.language(), "(untitled window)", "(제목 없는 창)").to_owned())
            }
            Ok(title) => Some(title),
            Err(_) => None,
        }
    }

    fn current_tab_strip_layout(&self) -> Option<TabStripLayout> {
        self.current_tab_strip_layout_with_client()
            .map(|(_, layout)| layout)
    }

    fn current_tab_strip_layout_with_client(&self) -> Option<(UiRect, TabStripLayout)> {
        let client = self.client_rect()?;
        Some((client, self.tab_strip_layout_for_client(client)))
    }

    fn tab_strip_layout_for_client(&self, client: UiRect) -> TabStripLayout {
        tab_strip_layout(
            client,
            self.app.state().workspace().tabs().len(),
            self.tab_strip_active_anchor_index(),
            self.tab_overflow_first_visible_index,
        )
    }

    fn tab_strip_active_anchor_index(&self) -> Option<usize> {
        if self.tab_reorder_drag.is_some() {
            None
        } else {
            self.active_tab_index()
        }
    }

    fn active_tab_index(&self) -> Option<usize> {
        let active_tab = self.app.active_tab_id()?;
        self.tab_index(active_tab)
    }

    fn tab_index(&self, tab_id: TabId) -> Option<usize> {
        self.app
            .state()
            .workspace()
            .tabs()
            .iter()
            .position(|tab| tab.id() == tab_id)
    }

    fn is_tab_bar_point(&self, point: ClientPoint) -> bool {
        let Some(client) = self.client_rect() else {
            return false;
        };

        point.x >= 0 && point.x < client.width() && point.y >= 0 && point.y < TAB_BAR_HEIGHT
    }

    fn apply_workspace_ui_window_chrome(
        &mut self,
        workspace_ui_visible: bool,
    ) -> Result<(), Win32StatusFailure> {
        let hidden_maximized_previous_bounds =
            self.apply_workspace_ui_title_bar_visibility(workspace_ui_visible)?;
        let chrome_result = self
            .apply_main_menu_visibility(workspace_ui_visible)
            .and_then(|()| self.apply_workspace_ui_window_region(workspace_ui_visible));

        restore_hidden_maximized_bounds_after_chrome_failure(
            chrome_result,
            hidden_maximized_previous_bounds,
            |bounds| self.restore_workspace_ui_window_bounds(bounds),
        )
    }

    fn apply_workspace_ui_title_bar_visibility(
        &mut self,
        workspace_ui_visible: bool,
    ) -> Result<Option<RECT>, Win32StatusFailure> {
        let current_style = self.current_window_style()?;
        let is_maximized = self.is_maximized();
        let desired_style = window_style_for_workspace_ui_visibility(
            current_style,
            workspace_ui_visible,
            is_maximized,
        );

        if desired_style != current_style {
            unsafe {
                SetLastError(0);
            }
            let previous_style = unsafe { SetWindowLongPtrW(self.hwnd, GWL_STYLE, desired_style) };
            if previous_style == 0 {
                let last_error = unsafe { GetLastError() };
                if last_error != 0 {
                    return Err(Win32StatusFailure::new(
                        "SetWindowLongPtrW",
                        last_error,
                        "j3GridDocker title bar 표시 상태를 바꿀 수 없습니다.",
                    ));
                }
            }

            self.apply_workspace_ui_frame_changed()?;
        }

        if !workspace_ui_visible && is_maximized {
            return self.apply_hidden_maximized_window_bounds();
        }

        Ok(None)
    }

    fn apply_workspace_ui_frame_changed(&self) -> Result<(), Win32StatusFailure> {
        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "SetWindowPos",
                last_error,
                "j3GridDocker title bar frame을 갱신할 수 없습니다.",
            ));
        }

        Ok(())
    }

    fn apply_hidden_maximized_window_bounds(&self) -> Result<Option<RECT>, Win32StatusFailure> {
        let work_area = self.monitor_work_area()?;
        let current = self.current_window_rect()?;

        if current.left == work_area.left
            && current.top == work_area.top
            && current.right == work_area.right
            && current.bottom == work_area.bottom
        {
            return Ok(None);
        }

        let width = work_area.right.checked_sub(work_area.left).ok_or_else(|| {
            Win32StatusFailure::new(
                "GetMonitorInfoW",
                0,
                "monitor 작업 영역 너비를 계산할 수 없습니다.",
            )
        })?;
        let height = work_area.bottom.checked_sub(work_area.top).ok_or_else(|| {
            Win32StatusFailure::new(
                "GetMonitorInfoW",
                0,
                "monitor 작업 영역 높이를 계산할 수 없습니다.",
            )
        })?;
        if width <= 0 || height <= 0 {
            return Err(Win32StatusFailure::new(
                "GetMonitorInfoW",
                0,
                "monitor 작업 영역 크기가 올바르지 않습니다.",
            ));
        }

        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                null_mut(),
                work_area.left,
                work_area.top,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "SetWindowPos",
                last_error,
                "숨김 상태의 maximized j3GridDocker window bounds를 보정할 수 없습니다.",
            ));
        }

        Ok(Some(current))
    }

    fn restore_workspace_ui_window_bounds(&self, bounds: RECT) -> Result<(), Win32StatusFailure> {
        let width = bounds.right.checked_sub(bounds.left).ok_or_else(|| {
            Win32StatusFailure::new(
                "GetWindowRect",
                0,
                "j3GridDocker window bounds 너비를 복구할 수 없습니다.",
            )
        })?;
        let height = bounds.bottom.checked_sub(bounds.top).ok_or_else(|| {
            Win32StatusFailure::new(
                "GetWindowRect",
                0,
                "j3GridDocker window bounds 높이를 복구할 수 없습니다.",
            )
        })?;
        if width <= 0 || height <= 0 {
            return Err(Win32StatusFailure::new(
                "GetWindowRect",
                0,
                "j3GridDocker window bounds 크기를 복구할 수 없습니다.",
            ));
        }

        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                null_mut(),
                bounds.left,
                bounds.top,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "SetWindowPos",
                last_error,
                "숨김 전환 롤백 중 j3GridDocker window bounds를 복구할 수 없습니다.",
            ));
        }

        Ok(())
    }

    fn monitor_work_area(&self) -> Result<RECT, Win32StatusFailure> {
        let monitor = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "MonitorFromWindow",
                last_error,
                "j3GridDocker monitor를 찾을 수 없습니다.",
            ));
        }

        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        let ok = unsafe { GetMonitorInfoW(monitor, &mut info) };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "GetMonitorInfoW",
                last_error,
                "j3GridDocker monitor 작업 영역을 읽을 수 없습니다.",
            ));
        }

        Ok(info.rcWork)
    }

    fn current_window_rect(&self) -> Result<RECT, Win32StatusFailure> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetWindowRect(self.hwnd, &mut rect) };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "GetWindowRect",
                last_error,
                "j3GridDocker window bounds를 읽을 수 없습니다.",
            ));
        }

        Ok(rect)
    }

    fn current_window_style(&self) -> Result<isize, Win32StatusFailure> {
        unsafe {
            SetLastError(0);
        }
        let style = unsafe { GetWindowLongPtrW(self.hwnd, GWL_STYLE) };
        if style == 0 {
            let last_error = unsafe { GetLastError() };
            if last_error != 0 {
                return Err(Win32StatusFailure::new(
                    "GetWindowLongPtrW",
                    last_error,
                    "j3GridDocker window style을 읽을 수 없습니다.",
                ));
            }
        }

        Ok(style)
    }

    fn is_maximized(&self) -> bool {
        unsafe { IsZoomed(self.hwnd) != 0 }
    }

    fn apply_workspace_ui_window_region(
        &mut self,
        workspace_ui_visible: bool,
    ) -> Result<(), Win32StatusFailure> {
        if workspace_ui_visible {
            let ok = unsafe { SetWindowRgn(self.hwnd, null_mut(), 1) };
            if ok == 0 {
                let last_error = unsafe { GetLastError() };
                return Err(Win32StatusFailure::new(
                    "SetWindowRgn",
                    last_error,
                    "작업 영역 UI window region을 복원할 수 없습니다.",
                ));
            }
            return Ok(());
        }

        let region = self.hidden_workspace_ui_window_region()?;
        let ok = unsafe { SetWindowRgn(self.hwnd, region, 1) };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            unsafe {
                DeleteObject(region as HGDIOBJ);
            }
            return Err(Win32StatusFailure::new(
                "SetWindowRgn",
                last_error,
                "작업 영역 UI window region을 적용할 수 없습니다.",
            ));
        }

        Ok(())
    }

    fn hidden_workspace_ui_window_region(
        &self,
    ) -> Result<windows_sys::Win32::Graphics::Gdi::HRGN, Win32StatusFailure> {
        let window = self.current_window_rect()?;
        let mut raw_client_origin = POINT { x: 0, y: 0 };
        let ok = unsafe { ClientToScreen(self.hwnd, &mut raw_client_origin) };
        if ok == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "ClientToScreen",
                last_error,
                "작업 영역 UI window region 기준 좌표를 screen 좌표로 변환할 수 없습니다.",
            ));
        }

        let client_origin = ScreenPoint {
            x: raw_client_origin.x,
            y: raw_client_origin.y,
        };
        let (width, height) = hidden_workspace_ui_region_size(window, client_origin)?;

        let region = unsafe { CreateRectRgn(0, 0, width, height) };
        if region.is_null() {
            let last_error = unsafe { GetLastError() };
            Err(Win32StatusFailure::new(
                "CreateRectRgn",
                last_error,
                "작업 영역 UI window region을 만들 수 없습니다.",
            ))
        } else {
            Ok(region)
        }
    }

    fn sync_visible_paint_tab_labels(&mut self, layout: TabStripLayout) {
        let tabs = self.app.state().workspace().tabs();
        let (first_visible_index, visible_end_index) = visible_tab_label_bounds(layout, tabs.len());

        sync_paint_tab_labels(
            &mut self.paint.tab_labels,
            tabs[first_visible_index..visible_end_index]
                .iter()
                .map(|tab| (tab.id(), tab.name())),
        );
    }

    fn visit_tab_rects<'a>(
        &'a self,
        layout: TabStripLayout,
        mut visit: impl FnMut(TabRect<'a>) -> bool,
    ) {
        let tabs = self.app.state().workspace().tabs();
        for index in layout.first_visible_index..layout.visible_end_index() {
            let Some(tab) = tabs.get(index) else {
                break;
            };
            let Some(rect) = tab_rect_for_index(layout, index) else {
                continue;
            };
            let keep_going = visit(TabRect {
                index,
                tab_id: tab.id(),
                rect,
                label: tab.name(),
            });
            if !keep_going {
                break;
            }
        }
    }

    fn visit_buttons(&self, mut visit: impl FnMut(ButtonRect) -> bool) {
        if !self.workspace_ui_visible() {
            return;
        }

        let Some(client) = self.client_rect() else {
            return;
        };

        visit_command_button_rects(client, |_, button| visit(button));
    }

    fn button_rect_for_command(&self, command: u16) -> Option<UiRect> {
        let mut rect = None;
        self.visit_buttons(|button| {
            if button.command == command {
                rect = Some(button.rect);
                false
            } else {
                true
            }
        });
        rect
    }

    fn tab_hit_at(&self, point: ClientPoint) -> Option<TabHit> {
        let layout = self.current_tab_strip_layout()?;
        let hit = hit_test_tab_strip(layout, point)?;
        let tab = self.app.state().workspace().tabs().get(hit.index)?;
        Some(TabHit {
            tab_id: tab.id(),
            target: hit.target,
        })
    }

    fn tab_body_at(&self, point: ClientPoint) -> Option<TabId> {
        tab_body_target_from_hit(self.tab_hit_at(point))
    }

    fn tab_context_target_at(&self, point: ClientPoint) -> Option<TabId> {
        tab_context_target_from_hit(self.tab_hit_at(point))
    }

    fn tab_strip_empty_at(&self, point: ClientPoint) -> bool {
        self.current_tab_strip_layout()
            .is_some_and(|layout| hit_test_tab_strip_empty(layout, point))
    }

    fn current_tab_ids(&self) -> Vec<TabId> {
        self.app
            .state()
            .workspace()
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .collect()
    }

    fn tab_overflow_hit_at(&self, point: ClientPoint) -> Option<TabOverflowHitTarget> {
        let layout = self.current_tab_strip_layout()?;
        hit_test_tab_overflow(layout, point)
    }

    fn handle_tab_overflow_hit(&mut self, target: TabOverflowHitTarget) {
        match target {
            TabOverflowHitTarget::Dropdown => self.show_tab_overflow_menu(),
        }
    }

    fn show_tab_overflow_menu(&mut self) {
        let Some(layout) = self.current_tab_strip_layout() else {
            self.set_status("탭 overflow 목록을 열 수 없습니다.");
            return;
        };
        let Some(dropdown) = layout.dropdown else {
            self.set_status("숨겨진 탭이 없습니다.");
            return;
        };
        if dropdown.hidden_count == 0 {
            self.set_status("숨겨진 탭이 없습니다.");
            return;
        }

        let Some(screen_point) = self.client_point_to_screen(ClientPoint {
            x: dropdown.rect.left,
            y: dropdown.rect.bottom,
        }) else {
            self.set_status("탭 overflow 목록 위치를 계산할 수 없습니다.");
            return;
        };

        let Some(mut menu) = TabOverflowPopupMenu::new() else {
            self.set_status("탭 overflow 목록을 열 수 없습니다.");
            return;
        };

        for (index, tab) in self.app.state().workspace().tabs().iter().enumerate() {
            if layout.is_index_visible(index) {
                continue;
            }

            if !menu.append_hidden_tab(index, tab.id(), tab.name()) {
                break;
            }
        }

        if menu.is_empty() {
            self.set_status("숨겨진 탭이 없습니다.");
            return;
        }

        if let Some(tab_id) = menu.select_tab(self.hwnd, screen_point) {
            log_tab_ux_trace(
                "overflow-select",
                format_args!(
                    "tab_id={}, first_visible_index={}, visible_count={}",
                    tab_id.value(),
                    layout.first_visible_index,
                    layout.visible_count
                ),
            );
            self.switch_tab(tab_id);
        }
    }

    fn button_at(&self, point: ClientPoint) -> Option<u16> {
        if toolbar_toggle_rect().contains(point) {
            return Some(CMD_WORKSPACE_UI_TOGGLE);
        }
        if new_tab_button_rect().contains(point) {
            return Some(CMD_TAB_ADD);
        }

        let mut hit = None;
        self.visit_buttons(|button| {
            if button.rect.contains(point) {
                hit = Some(button.command);
                false
            } else {
                true
            }
        });
        hit
    }

    fn ensure_paint_layout_cache(&mut self, tab_id: TabId, bounds: Rect) -> bool {
        let key = PaintLayoutCacheKey {
            tab_id,
            bounds,
            content_top: self.top_bar_height(),
        };
        if self.paint.layout_cache.matches(key) {
            return true;
        }

        let result = {
            let workspace = self.app.state().workspace();
            workspace.region_and_splitter_rects_for_tab_into(
                tab_id,
                bounds,
                SPLITTER_HIT_TOLERANCE,
                DEFAULT_MIN_REGION_SIZE,
                &mut self.paint.layout_regions,
                &mut self.paint.layout_splitters,
            )
        };

        match result {
            Ok(()) => {
                self.paint.layout_cache.rebuild(
                    key,
                    &self.paint.layout_regions,
                    &self.paint.layout_splitters,
                );
                true
            }
            Err(error) => {
                self.paint.layout_cache.clear();
                self.report_app_error(AppError::from(error));
                false
            }
        }
    }

    fn rebuild_paint_layout_cache_from_drag_regions(
        &mut self,
        tab_id: TabId,
        drag_bounds: Rect,
        regions: &[RegionRect],
    ) -> bool {
        let Some(bounds) = self.layout_bounds_client() else {
            return false;
        };
        if drag_bounds.width() != bounds.width() || drag_bounds.height() != bounds.height() {
            return false;
        }

        let Some(dx) = bounds.left().checked_sub(drag_bounds.left()) else {
            return false;
        };
        let Some(dy) = bounds.top().checked_sub(drag_bounds.top()) else {
            return false;
        };
        let key = PaintLayoutCacheKey {
            tab_id,
            bounds,
            content_top: self.top_bar_height(),
        };

        self.paint.layout_regions.clear();
        for region in regions {
            let Ok(rect) = region.rect().translated(dx, dy) else {
                self.paint.layout_regions.clear();
                return false;
            };
            self.paint
                .layout_regions
                .push(RegionRect::new(region.region_id(), rect));
        }

        let result = {
            let workspace = self.app.state().workspace();
            workspace.splitter_rects_for_tab_into(
                tab_id,
                bounds,
                SPLITTER_HIT_TOLERANCE,
                DEFAULT_MIN_REGION_SIZE,
                &mut self.paint.layout_splitters,
            )
        };
        if result.is_err() {
            self.paint.layout_regions.clear();
            self.paint.layout_splitters.clear();
            return false;
        }

        self.paint.layout_cache.rebuild(
            key,
            &self.paint.layout_regions,
            &self.paint.layout_splitters,
        );
        self.invalidate_active_tab_sync_cache();
        self.invalidate_splitter_overlay_rect_cache();
        true
    }

    fn invalidate_paint_layout_cache(&mut self) {
        self.paint.layout_cache.invalidate();
        self.invalidate_active_tab_sync_cache();
        self.invalidate_splitter_overlay_rect_cache();
    }

    fn ensure_paint_occupied_regions(&mut self, tab_id: TabId) -> bool {
        if self.paint.occupied_tab_id == Some(tab_id) {
            return true;
        }

        self.paint.occupied_regions.clear();

        match self.app.state().workspace().placements_for_tab(tab_id) {
            Ok(placements) => {
                self.paint.occupied_regions.reserve(placements.len());
                for placement in placements {
                    self.paint.occupied_regions.insert(placement.region_id());
                }
                self.paint.occupied_tab_id = Some(tab_id);
                true
            }
            Err(error) => {
                self.paint.occupied_tab_id = None;
                self.report_app_error(AppError::from(error));
                false
            }
        }
    }

    fn invalidate_paint_occupied_regions(&mut self) {
        self.paint.occupied_tab_id = None;
        self.invalidate_active_tab_sync_cache();
    }

    fn invalidate_active_tab_sync_cache(&mut self) {
        self.active_tab_sync_cache = None;
    }

    fn install_icons(&mut self) {
        for (size, icon_type) in [(32, ICON_BIG), (16, ICON_SMALL)] {
            let Some(icon) = load_icon(size) else {
                continue;
            };
            unsafe {
                SendMessageW(self.hwnd, WM_SETICON, icon_type as WPARAM, icon as LPARAM);
            }
            self.icons.push(icon);
        }
    }

    fn shutdown_once(&mut self, mode: ShutdownMode) -> bool {
        if self.shutdown_done {
            return true;
        }

        let settings_save_result = self.save_settings_before_shutdown();
        let active_tab_hidden = self.active_tab_hidden_for_shutdown();
        let attempt = match shutdown_report_after_settings_save(settings_save_result, mode, || {
            self.app.shutdown_with_active_tab_hidden(active_tab_hidden)
        }) {
            Ok(attempt) => attempt,
            Err(error) => {
                self.report_shutdown_settings_save_error(error);
                return false;
            }
        };
        self.shutdown_done = shutdown_report_is_complete(&attempt.report);
        self.report_shutdown_attempt(attempt);
        self.shutdown_done
    }

    fn active_tab_hidden_for_shutdown(&self) -> bool {
        self.frame_state.active_tab_hidden_for_shutdown()
    }

    fn save_settings_before_shutdown(&mut self) -> Result<(), ShutdownSettingsSaveError> {
        ShutdownSettingsSaver::new(
            &self.app,
            &self.settings_store,
            self.settings_save_policy,
            &mut self.preserved_startup_session,
            self.workspace_options,
        )
        .save()
    }

    fn record_workspace_change(&mut self) {
        self.workspace_change_generation = self.workspace_change_generation.wrapping_add(1);
        self.settings_save_policy.allow_after_workspace_change();
        self.preserved_startup_session = None;
        self.invalidate_splitter_overlay_rect_cache();
    }

    fn record_workspace_options_change(&mut self) {
        self.settings_save_policy
            .allow_after_workspace_options_change();
    }

    fn destroy_icons(&mut self) {
        for icon in self.icons.drain(..) {
            unsafe {
                DestroyIcon(icon);
            }
        }
    }

    fn report_undock_summary(&mut self, report: &ShutdownReport) {
        self.status
            .replace(undock_summary_text_for(self.language(), report));
        log_undock_failures(report);
    }

    fn report_shutdown_attempt(&mut self, attempt: ShutdownAttemptReport) {
        self.report_undock_summary(&attempt.report);
        if let Some(error) = attempt.settings_save_error {
            let settings_save_error_text =
                shutdown_settings_save_error_message(self.language(), &error);
            self.report_shutdown_settings_save_error(error);
            self.status.replace(format!(
                "{settings_save_error_text} {}",
                undock_summary_text_for(self.language(), &attempt.report)
            ));
        }
    }

    fn report_tab_deletion_summary(&mut self, report: &TabDeletionReport) {
        self.status
            .replace(tab_deletion_status_text_for(self.language(), report));
        log_tab_ux_trace(
            "delete-finish",
            format_args!(
                "tab_id={}, current_active={}, undock_attempted={}, undock_restored={}, undock_missing={}, undock_failures={}",
                report.deleted_tab_id().value(),
                optional_tab_id_trace_text(report.current_active_tab()),
                report.undock().attempted(),
                report.undock().restored(),
                report.undock().missing(),
                report.undock().failures().len()
            ),
        );
        log_undock_failures(report.undock());
    }

    fn tab_status_label(&self, tab_id: TabId) -> TabStatusLabel {
        let name = self
            .app
            .state()
            .workspace()
            .tab(tab_id)
            .ok()
            .map(|tab| tab.name().to_owned());
        TabStatusLabel::new(tab_id, name)
    }

    fn tab_switch_status_context(&self, target_tab: TabId) -> TabSwitchStatusContext {
        TabSwitchStatusContext {
            target: self.tab_status_label(target_tab),
            previous_active: self
                .app
                .active_tab_id()
                .map(|tab_id| self.tab_status_label(tab_id)),
        }
    }

    fn tab_deletion_status_context(&self, deleted_tab: TabId) -> TabDeletionStatusContext {
        TabDeletionStatusContext {
            deleted: self.tab_status_label(deleted_tab),
            previous_active: self
                .app
                .active_tab_id()
                .map(|tab_id| self.tab_status_label(tab_id)),
            automatic_target: self.automatic_active_after_deletion_status_label(deleted_tab),
        }
    }

    fn automatic_active_after_deletion_status_label(
        &self,
        deleted_tab: TabId,
    ) -> Option<TabStatusLabel> {
        let workspace = self.app.state().workspace();
        if workspace.active_tab_id() != Some(deleted_tab) {
            return None;
        }

        let tabs = workspace.tabs();
        let index = tabs.iter().position(|tab| tab.id() == deleted_tab)?;
        if tabs.len() <= 1 {
            return None;
        }

        let target = if index + 1 < tabs.len() {
            tabs.get(index + 1)
        } else {
            index.checked_sub(1).and_then(|index| tabs.get(index))
        }?;

        Some(TabStatusLabel::new(
            target.id(),
            Some(target.name().to_owned()),
        ))
    }

    fn report_switch_tab_error(&mut self, context: &TabSwitchStatusContext, error: AppError) {
        log_tab_ux_trace(
            "switch-error",
            format_args!(
                "target_tab_id={}, current_active={}",
                context.target.tab_id.value(),
                optional_tab_id_trace_text(self.app.active_tab_id())
            ),
        );
        self.status.replace(switch_tab_failure_status_text_for(
            self.language(),
            context,
            self.app.active_tab_id(),
            &error,
        ));
        log_app_error(&error);
        self.invalidate_status();
    }

    fn report_tab_deletion_error(&mut self, context: &TabDeletionStatusContext, error: AppError) {
        self.status.replace(tab_deletion_error_status_text_for(
            self.language(),
            context,
            self.app.active_tab_id(),
            &error,
        ));
        log_app_error(&error);
        self.invalidate_status();
    }

    fn report_tab_operation_error(&mut self, tab_id: TabId, operation: &str, error: AppError) {
        self.status.replace(tab_operation_error_status_text(
            self.language(),
            tab_id,
            operation,
            &error,
        ));
        log_app_error(&error);
        self.invalidate_status();
    }

    fn report_app_error(&mut self, error: AppError) {
        self.status
            .replace(app_error_message(self.language(), &error));
        log_app_error(&error);
        self.invalidate_status();
    }

    fn report_drop_registration_error(
        &mut self,
        source_region_id: Option<RegionId>,
        target_region_id: RegionId,
        error: AppError,
    ) {
        self.status.replace(drop_registration_error_status_text_for(
            self.language(),
            source_region_id,
            target_region_id,
            &error,
        ));
        log_app_error(&error);
        self.invalidate_status();
    }

    fn report_entry_error(&mut self, operation: &str, error: EntryError) {
        self.status
            .replace(entry_error_status_text(self.language(), operation, &error));
        eprintln!("{error}");
        if let Some(source) = error.source() {
            eprintln!("cause: {source}");
        }
        self.invalidate_status();
    }

    fn report_settings_error(&mut self, error: SettingsFileError) {
        self.status
            .replace_str(settings_error_message(self.language(), &error));
        eprintln!("{error}");
        if let Some(source) = error.source() {
            eprintln!("cause: {source}");
        }
        self.invalidate_status();
    }

    fn report_shutdown_settings_save_error(&mut self, error: ShutdownSettingsSaveError) {
        match error {
            ShutdownSettingsSaveError::App(error) => self.report_app_error(error),
            ShutdownSettingsSaveError::Settings(error) => self.report_settings_error(error),
        }
    }

    fn report_win32_status(&mut self, api: &'static str, user_message: &'static str) {
        let last_error = unsafe { GetLastError() };
        self.report_win32_status_with_code(api, last_error, user_message);
    }

    fn report_win32_status_with_code(
        &mut self,
        api: &'static str,
        last_error: u32,
        user_message: &'static str,
    ) {
        let message = localized_message(self.language(), user_message);
        self.status.replace_str(message.as_ref());
        eprintln!("{api} failed with GetLastError={last_error}");
        self.invalidate_status();
    }

    fn report_win32_status_failure(&mut self, failure: Win32StatusFailure) {
        self.report_win32_status_with_code(failure.api, failure.last_error, failure.user_message);
    }

    fn set_status(&mut self, status: &str) {
        let status = localized_message(self.language(), status);
        self.status.replace_str(status.as_ref());
        self.invalidate_status();
    }

    fn set_status_i18n(&mut self, english: &'static str, korean: &'static str) {
        self.set_status(ui_text(self.language(), english, korean));
    }

    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.hwnd, null(), 0);
        }
    }

    fn invalidate_rect(&self, rect: UiRect) {
        if rect.is_empty() {
            return;
        }

        let rect = rect.to_rect();
        unsafe {
            InvalidateRect(self.hwnd, &rect, 0);
        }
    }

    fn invalidate_status(&self) {
        if let Some(rect) = self.status_bar_rect() {
            self.invalidate_rect(rect);
        }
    }

    fn invalidate_workspace_body(&self) {
        if let Some(rect) = self.workspace_body_rect() {
            self.invalidate_rect(rect);
        }
    }

    fn invalidate_active_region_change(
        &self,
        tab_id: TabId,
        previous: Option<RegionId>,
        current: Option<RegionId>,
    ) {
        if previous == current {
            return;
        }

        let mut dirty: Option<UiRect> = None;
        for region_id in [previous, current].into_iter().flatten() {
            let Some(rect) = self.region_client_rect(tab_id, region_id) else {
                continue;
            };
            dirty = Some(match dirty {
                Some(existing) => existing.union(rect),
                None => rect,
            });
        }

        if let Some(rect) = dirty {
            self.invalidate_rect(rect);
        } else {
            self.invalidate_workspace_body();
        }
    }
}

fn next_tab_preset_name(existing_count: usize) -> String {
    let next = existing_count.saturating_add(1);
    format!("Tab Preset {next}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabPresetProgramFailure {
    label: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabPresetProgramRestoreReport {
    expected: usize,
    docked: usize,
    failures: Vec<TabPresetProgramFailure>,
}

impl TabPresetProgramRestoreReport {
    fn new(expected: usize) -> Self {
        Self {
            expected,
            docked: 0,
            failures: Vec::new(),
        }
    }
}

struct TabPresetProgramRestoreState {
    preset_name: String,
    target: TabStatusLabel,
    target_tab_id: TabId,
    undocked: usize,
    report: TabPresetProgramRestoreReport,
    pending: Vec<PendingTabPresetProgramRestore>,
    process_tree_scan: ProcessTreeScanSchedule,
    window_search: PendingProcessWindowSearch,
    window_event_hook: Option<TabPresetProgramWindowEventHook>,
    deadline: Instant,
}

struct TabPresetProgramRestoreRequest {
    preset_name: String,
    target: TabStatusLabel,
    target_tab_id: TabId,
    undocked: usize,
    report: TabPresetProgramRestoreReport,
    pending: Vec<PendingTabPresetProgramRestore>,
    deadline: Instant,
}

impl TabPresetProgramRestoreState {
    fn new(request: TabPresetProgramRestoreRequest, now: Instant, target_hwnd: HWND) -> Self {
        let window_event_hook = TabPresetProgramWindowEventHook::install(
            target_hwnd,
            request
                .pending
                .iter()
                .flat_map(|pending| pending.process_ids()),
        );
        let process_tree_scan = if window_event_hook.is_some() {
            ProcessTreeScanSchedule::hooked_fallback(now)
        } else {
            ProcessTreeScanSchedule::new(now)
        };
        let window_search = if window_event_hook.is_some() {
            PendingProcessWindowSearch::from_pending_with_hooked_fallback(&request.pending, now)
        } else {
            PendingProcessWindowSearch::from_pending(
                &request.pending,
                now,
                Duration::from_millis(0),
            )
        };
        Self {
            preset_name: request.preset_name,
            target: request.target,
            target_tab_id: request.target_tab_id,
            undocked: request.undocked,
            report: request.report,
            pending: request.pending,
            process_tree_scan,
            window_search,
            window_event_hook,
            deadline: request.deadline,
        }
    }

    fn remove_pending(&mut self, index: usize) -> PendingTabPresetProgramRestore {
        let pending = self.pending.remove(index);
        for process_id in pending.process_ids() {
            self.window_search.remove_process(process_id);
        }
        pending
    }

    fn observe_child_statuses(&mut self) {
        for pending in &mut self.pending {
            pending.observe_child_status();
        }
    }

    fn refresh_tracked_processes(&mut self, now: Instant) {
        if self.pending.is_empty()
            || !self.process_tree_scan.should_scan(now, self.deadline)
            || !self.has_unmatched_tracked_processes()
        {
            return;
        }

        let Some(process_tree) = process_tree_child_index() else {
            self.process_tree_scan.mark_completed(now, false);
            return;
        };

        let mut discovered = Vec::new();
        let mut stale_process_ids = Vec::new();
        let mut descendant_stack = Vec::new();
        let window_search = &self.window_search;
        for pending in &mut self.pending {
            process_tree.append_new_descendants(
                &mut pending.tracked_process_ids,
                &mut discovered,
                &mut descendant_stack,
            );
            process_tree.remove_missing_processes(
                &mut pending.tracked_process_ids,
                |process_id| window_search.has_match_for_process(process_id),
                &mut stale_process_ids,
            );
        }
        for process_id in stale_process_ids {
            self.window_search.remove_process(process_id);
        }

        let found_process = !discovered.is_empty();
        self.track_process_ids(discovered);
        self.process_tree_scan.mark_completed(now, found_process);
    }

    fn track_process_ids(&mut self, process_ids: Vec<u32>) {
        if process_ids.is_empty() {
            return;
        }

        self.window_search
            .add_process_ids(process_ids.iter().copied());
        if let Some(window_event_hook) = self.window_event_hook.as_mut() {
            window_event_hook.add_process_ids(process_ids);
        }
    }

    fn scan_windows(&mut self, now: Instant) -> bool {
        self.window_search.refresh(now, self.deadline)
    }

    fn has_due_fallback_scan(&self, now: Instant) -> bool {
        self.window_search.should_scan(now, self.deadline)
            || (self.process_tree_scan.should_scan(now, self.deadline)
                && self.has_unmatched_tracked_processes())
    }

    fn has_unmatched_tracked_processes(&self) -> bool {
        self.pending.iter().any(|pending| {
            pending
                .process_ids()
                .any(|process_id| self.window_search.needs_window_for_process(process_id))
        })
    }

    fn has_window_matches(&self) -> bool {
        self.window_search.has_matches()
    }

    fn record_window_event(&mut self, hwnd: HWND) {
        self.window_search.record_window_event(hwnd);
    }
}

struct PendingTabPresetProgramRestore {
    label: String,
    region_id: RegionId,
    path: String,
    process_id: u32,
    tracked_process_ids: HashSet<u32>,
    child: Child,
}

impl PendingTabPresetProgramRestore {
    fn new(label: String, region_id: RegionId, path: String, child: Child) -> Self {
        let process_id = child.id();
        Self {
            label,
            region_id,
            path,
            process_id,
            tracked_process_ids: HashSet::from([process_id]),
            child,
        }
    }

    fn process_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.tracked_process_ids.iter().copied()
    }

    fn matching_hwnd(&self, window_search: &PendingProcessWindowSearch) -> Option<HWND> {
        window_search.hwnd_for_any(self.process_ids())
    }

    fn observe_child_status(&mut self) {
        let _ = self.child.try_wait();
    }
}

struct LaunchedTabPresetProgram {
    path: String,
    child: Child,
}

#[derive(Debug, Clone, Copy)]
struct ProcessTreeScanSchedule {
    next_scan_at: Instant,
    min_scan_interval: Duration,
    scan_interval: Duration,
    max_scan_interval: Duration,
    last_scan_at: Option<Instant>,
}

impl ProcessTreeScanSchedule {
    fn new(now: Instant) -> Self {
        Self::with_policy(
            now,
            TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL,
            TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL,
            TAB_PRESET_PROCESS_TREE_MAX_SCAN_INTERVAL,
        )
    }

    fn hooked_fallback(now: Instant) -> Self {
        Self::with_policy(
            now,
            TAB_PRESET_HOOKED_PROCESS_TREE_SCAN_INTERVAL,
            TAB_PRESET_HOOKED_PROCESS_TREE_SCAN_INTERVAL,
            TAB_PRESET_HOOKED_PROCESS_TREE_MAX_SCAN_INTERVAL,
        )
    }

    fn with_policy(
        now: Instant,
        initial_scan_delay: Duration,
        min_scan_interval: Duration,
        max_scan_interval: Duration,
    ) -> Self {
        Self {
            next_scan_at: now + initial_scan_delay,
            min_scan_interval,
            scan_interval: min_scan_interval,
            max_scan_interval,
            last_scan_at: None,
        }
    }

    fn should_scan(&self, now: Instant, deadline: Instant) -> bool {
        now >= self.next_scan_at || (now >= deadline && self.should_scan_at_deadline(now))
    }

    fn should_scan_at_deadline(&self, now: Instant) -> bool {
        let Some(last_scan_at) = self.last_scan_at else {
            return true;
        };

        match now.checked_duration_since(last_scan_at) {
            Some(elapsed) => elapsed >= TAB_PRESET_DEADLINE_RESCAN_SUPPRESSION,
            None => false,
        }
    }

    fn mark_completed(&mut self, now: Instant, found_process: bool) {
        self.last_scan_at = Some(now);
        if found_process {
            self.scan_interval = self.min_scan_interval;
        }

        self.next_scan_at = now + self.scan_interval;

        if !found_process {
            self.scan_interval =
                (self.scan_interval + self.scan_interval).min(self.max_scan_interval);
        }
    }
}

#[derive(Debug)]
struct PendingProcessWindowSearch {
    process_ids: HashSet<u32>,
    hwnds: HashMap<u32, HWND>,
    remaining_process_ids: HashSet<u32>,
    next_scan_at: Instant,
    min_scan_interval: Duration,
    scan_interval: Duration,
    max_scan_interval: Duration,
    last_scan_at: Option<Instant>,
    next_thread_snapshot_at: Instant,
    thread_snapshot_min_interval: Duration,
    thread_snapshot_interval: Duration,
    thread_snapshot_max_interval: Duration,
    last_thread_snapshot_at: Option<Instant>,
}

#[derive(Clone, Copy)]
struct PendingProcessWindowScanPolicy {
    initial_scan_delay: Duration,
    min_scan_interval: Duration,
    max_scan_interval: Duration,
    initial_thread_snapshot_delay: Duration,
    thread_snapshot_min_interval: Duration,
    thread_snapshot_max_interval: Duration,
}

impl PendingProcessWindowScanPolicy {
    const fn standard(initial_scan_delay: Duration) -> Self {
        Self {
            initial_scan_delay,
            min_scan_interval: TAB_PRESET_WINDOW_SCAN_INTERVAL,
            max_scan_interval: TAB_PRESET_WINDOW_MAX_SCAN_INTERVAL,
            initial_thread_snapshot_delay: TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            thread_snapshot_min_interval: TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            thread_snapshot_max_interval: TAB_PRESET_THREAD_SNAPSHOT_MAX_INTERVAL,
        }
    }

    const fn hooked_fallback() -> Self {
        Self {
            initial_scan_delay: TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL,
            min_scan_interval: TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL,
            max_scan_interval: TAB_PRESET_HOOKED_WINDOW_MAX_SCAN_INTERVAL,
            initial_thread_snapshot_delay: TAB_PRESET_HOOKED_THREAD_SNAPSHOT_DELAY,
            thread_snapshot_min_interval: TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            thread_snapshot_max_interval: TAB_PRESET_THREAD_SNAPSHOT_MAX_INTERVAL,
        }
    }
}

impl PendingProcessWindowSearch {
    fn from_pending(
        pending: &[PendingTabPresetProgramRestore],
        now: Instant,
        initial_scan_delay: Duration,
    ) -> Self {
        Self::from_process_ids_with_scan_delay(
            pending.iter().flat_map(|pending| pending.process_ids()),
            now,
            initial_scan_delay,
        )
    }

    fn from_pending_with_hooked_fallback(
        pending: &[PendingTabPresetProgramRestore],
        now: Instant,
    ) -> Self {
        Self::from_process_ids_with_scan_policy(
            pending.iter().flat_map(|pending| pending.process_ids()),
            now,
            PendingProcessWindowScanPolicy::hooked_fallback(),
        )
    }

    #[cfg(test)]
    fn from_process_ids(process_ids: impl IntoIterator<Item = u32>, now: Instant) -> Self {
        Self::from_process_ids_with_scan_delay(process_ids, now, Duration::from_millis(0))
    }

    fn from_process_ids_with_scan_delay(
        process_ids: impl IntoIterator<Item = u32>,
        now: Instant,
        initial_scan_delay: Duration,
    ) -> Self {
        Self::from_process_ids_with_scan_policy(
            process_ids,
            now,
            PendingProcessWindowScanPolicy::standard(initial_scan_delay),
        )
    }

    fn from_process_ids_with_scan_policy(
        process_ids: impl IntoIterator<Item = u32>,
        now: Instant,
        policy: PendingProcessWindowScanPolicy,
    ) -> Self {
        let process_ids = process_ids
            .into_iter()
            .filter(|process_id| *process_id != 0)
            .collect::<HashSet<_>>();
        let hwnds = HashMap::with_capacity(process_ids.len());
        let remaining_process_ids = process_ids.clone();
        Self {
            process_ids,
            hwnds,
            remaining_process_ids,
            next_scan_at: now + policy.initial_scan_delay,
            min_scan_interval: policy.min_scan_interval,
            scan_interval: policy.min_scan_interval,
            max_scan_interval: policy.max_scan_interval,
            last_scan_at: None,
            next_thread_snapshot_at: now + policy.initial_thread_snapshot_delay,
            thread_snapshot_min_interval: policy.thread_snapshot_min_interval,
            thread_snapshot_interval: policy.thread_snapshot_min_interval,
            thread_snapshot_max_interval: policy.thread_snapshot_max_interval,
            last_thread_snapshot_at: None,
        }
    }

    fn add_process_ids(&mut self, process_ids: impl IntoIterator<Item = u32>) {
        for process_id in process_ids {
            if process_id == 0 || self.hwnds.contains_key(&process_id) {
                continue;
            }
            if self.process_ids.insert(process_id) {
                self.remaining_process_ids.insert(process_id);
            }
        }
    }

    fn remove_process(&mut self, process_id: u32) {
        self.process_ids.remove(&process_id);
        self.hwnds.remove(&process_id);
        self.remaining_process_ids.remove(&process_id);
    }

    fn clear(&mut self) {
        self.process_ids.clear();
        self.hwnds.clear();
        self.remaining_process_ids.clear();
    }

    fn hwnd_for(&self, process_id: u32) -> Option<HWND> {
        self.hwnds.get(&process_id).copied()
    }

    fn hwnd_for_any(&self, process_ids: impl IntoIterator<Item = u32>) -> Option<HWND> {
        let mut selected = None;
        for process_id in process_ids {
            let Some(hwnd) = self.hwnd_for(process_id) else {
                continue;
            };

            let should_replace = match selected {
                Some(existing) => window_is_above(hwnd, existing),
                None => true,
            };
            if should_replace {
                selected = Some(hwnd);
            }
        }

        selected
    }

    fn has_matches(&self) -> bool {
        !self.hwnds.is_empty()
    }

    fn has_match_for_process(&self, process_id: u32) -> bool {
        self.hwnds.contains_key(&process_id)
    }

    fn needs_window_for_process(&self, process_id: u32) -> bool {
        self.remaining_process_ids.contains(&process_id)
    }

    fn record_window_event(&mut self, hwnd: HWND) -> bool {
        if !top_level_window_is_visible_root_unowned(hwnd) {
            return false;
        }

        let process_id = window_process_id(hwnd);
        if !self.process_ids.contains(&process_id) {
            return false;
        }

        self.record_process_window(process_id, hwnd);
        true
    }

    fn refresh(&mut self, now: Instant, deadline: Instant) -> bool {
        if !self.should_scan(now, deadline) {
            return false;
        }

        let scan_threads = self.should_scan_threads(now, deadline);
        let found_before = self.hwnds.len();
        if self.remaining_process_ids.is_empty() {
            return false;
        }

        let found_thread_window = top_level_windows_for_processes(
            &mut self.remaining_process_ids,
            &mut self.hwnds,
            scan_threads,
        );
        if scan_threads {
            self.mark_thread_scan_completed(now, found_thread_window);
        }
        let found_window = self.hwnds.len() > found_before;
        self.mark_scan_completed(now, found_window);
        found_window
    }

    fn should_scan(&self, now: Instant, deadline: Instant) -> bool {
        !self.remaining_process_ids.is_empty()
            && (now >= self.next_scan_at || (now >= deadline && self.should_scan_at_deadline(now)))
    }

    fn should_scan_threads(&self, now: Instant, deadline: Instant) -> bool {
        !self.remaining_process_ids.is_empty()
            && (now >= self.next_thread_snapshot_at
                || (now >= deadline && self.should_scan_threads_at_deadline(now)))
    }

    fn should_scan_at_deadline(&self, now: Instant) -> bool {
        let Some(last_scan_at) = self.last_scan_at else {
            return true;
        };

        match now.checked_duration_since(last_scan_at) {
            Some(elapsed) => elapsed >= TAB_PRESET_DEADLINE_RESCAN_SUPPRESSION,
            None => false,
        }
    }

    fn should_scan_threads_at_deadline(&self, now: Instant) -> bool {
        let Some(last_thread_snapshot_at) = self.last_thread_snapshot_at else {
            return true;
        };

        match now.checked_duration_since(last_thread_snapshot_at) {
            Some(elapsed) => elapsed >= TAB_PRESET_DEADLINE_RESCAN_SUPPRESSION,
            None => false,
        }
    }

    fn mark_scan_completed(&mut self, now: Instant, found_window: bool) {
        self.last_scan_at = Some(now);
        if found_window {
            self.scan_interval = self.min_scan_interval;
        }

        self.next_scan_at = now + self.scan_interval;

        if !found_window {
            self.scan_interval =
                (self.scan_interval + self.scan_interval).min(self.max_scan_interval);
        }
    }

    fn mark_thread_scan_completed(&mut self, now: Instant, found_window: bool) {
        self.last_thread_snapshot_at = Some(now);
        if found_window {
            self.thread_snapshot_interval = self.thread_snapshot_min_interval;
        }

        self.next_thread_snapshot_at = now + self.thread_snapshot_interval;

        if !found_window {
            self.thread_snapshot_interval = (self.thread_snapshot_interval
                + self.thread_snapshot_interval)
                .min(self.thread_snapshot_max_interval);
        }
    }

    fn record_process_window(&mut self, process_id: u32, hwnd: HWND) {
        insert_topmost_process_window(&mut self.hwnds, process_id, hwnd);
        self.remaining_process_ids.remove(&process_id);
    }
}

fn tab_preset_save_success_status_text(
    language: UiLanguage,
    preset_name: &str,
    program_count: usize,
) -> String {
    if language == UiLanguage::English {
        return format!("Tab preset saved: {preset_name}. Program(s): {program_count}");
    }

    format!("탭 preset 저장 완료: {preset_name}. 프로그램 {program_count}개")
}

fn tab_preset_delete_success_status_text(language: UiLanguage, preset_name: &str) -> String {
    if language == UiLanguage::English {
        return format!("Tab preset deleted: {preset_name}");
    }

    format!("탭 preset 삭제 완료: {preset_name}")
}

fn tab_preset_edit_success_status_text(
    language: UiLanguage,
    preset_name: &str,
    program_count: usize,
) -> String {
    if language == UiLanguage::English {
        return format!("Tab preset edited: {preset_name}. Program(s): {program_count}");
    }

    format!("탭 preset 편집 완료: {preset_name}. 프로그램 {program_count}개")
}

fn tab_preset_edit_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    error: &AppError,
) -> String {
    if language == UiLanguage::English {
        return format!(
            "Tab preset edit failed: {preset_name}. Cause: {}",
            app_error_message(language, error)
        );
    }

    format!(
        "탭 preset 편집 실패: {preset_name}. 원인: {}",
        error.user_message()
    )
}

fn tab_preset_delete_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    error: &AppError,
) -> String {
    if language == UiLanguage::English {
        return format!(
            "Tab preset delete failed: {preset_name}. Cause: {}",
            app_error_message(language, error)
        );
    }

    format!(
        "탭 preset 삭제 실패: {preset_name}. 원인: {}",
        error.user_message()
    )
}

fn tab_preset_apply_success_status_text(
    language: UiLanguage,
    report: &TabPresetApplication,
    target: &TabStatusLabel,
    undocked: usize,
    restore: &TabPresetProgramRestoreReport,
) -> String {
    tab_preset_apply_success_status_text_for_preset(
        language,
        report.preset_name(),
        target,
        undocked,
        restore,
    )
}

fn tab_preset_apply_success_status_text_for_preset(
    language: UiLanguage,
    preset_name: &str,
    target: &TabStatusLabel,
    undocked: usize,
    restore: &TabPresetProgramRestoreReport,
) -> String {
    let target = tab_status_label_for_entry(language, target);
    let failures = restore.failures.len();
    if language == UiLanguage::English {
        return format!(
            "Tab preset loaded: {} -> {target}. Existing undocked: {undocked}. Programs docked {}/{}; failures {}{}",
            preset_name,
            restore.docked,
            restore.expected,
            failures,
            tab_preset_program_failures_suffix(language, &restore.failures)
        );
    }

    format!(
        "탭 preset 불러오기 완료: {} -> {target}. 기존 Undock {undocked}개. 프로그램 dock {}/{}개, 실패 {}개{}",
        preset_name,
        restore.docked,
        restore.expected,
        failures,
        tab_preset_program_failures_suffix(language, &restore.failures)
    )
}

fn tab_preset_apply_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    target: &TabStatusLabel,
    error: &AppError,
) -> String {
    let target = tab_status_label_for_entry(language, target);
    if language == UiLanguage::English {
        return format!(
            "Tab preset load failed: {preset_name} -> {target}. Cause: {}",
            app_error_message(language, error)
        );
    }

    format!(
        "탭 preset 불러오기 실패: {preset_name} -> {target}. 원인: {}",
        error.user_message()
    )
}

fn tab_preset_program_failures_suffix(
    language: UiLanguage,
    failures: &[TabPresetProgramFailure],
) -> String {
    if failures.is_empty() {
        return String::new();
    }

    let mut text = if language == UiLanguage::English {
        String::from(". Failed: ")
    } else {
        String::from(". 실패: ")
    };
    for (index, failure) in failures.iter().take(3).enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        text.push_str(&failure.label);
        text.push_str(" (");
        text.push_str(&failure.message);
        text.push(')');
    }
    if failures.len() > 3 {
        text.push_str(", ...");
    }
    text
}

fn tab_preset_program_restore_timer_failure_report(
    state: TabPresetProgramRestoreState,
    language: UiLanguage,
) -> TabPresetProgramRestoreReport {
    let mut report = state.report;
    let message = if language == UiLanguage::English {
        "program restore timer could not be started"
    } else {
        "프로그램 복원 timer를 시작할 수 없습니다"
    };
    for pending in state.pending {
        report.failures.push(TabPresetProgramFailure {
            label: pending.label,
            message: message.to_owned(),
        });
    }
    report
}

fn tab_status_label_for_entry(language: UiLanguage, label: &TabStatusLabel) -> String {
    let tab_word = ui_text(language, "tab", "탭");
    match label.name.as_deref() {
        Some(name) => format!("{name} ({tab_word} {})", label.tab_id.value()),
        None => format!("{tab_word} {}", label.tab_id.value()),
    }
}

fn tab_preset_program_label(program: &ExternalProgramSpec) -> String {
    program
        .title()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| program.executable_path())
        .to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramArgumentsParseError {
    UnterminatedQuote,
}

impl ProgramArgumentsParseError {
    fn user_message(self, language: UiLanguage) -> &'static str {
        match self {
            Self::UnterminatedQuote => {
                if language == UiLanguage::English {
                    "program arguments contain an unterminated quote"
                } else {
                    "프로그램 arguments에 닫히지 않은 큰따옴표가 있습니다"
                }
            }
        }
    }
}

fn parse_program_arguments(input: &str) -> Result<Vec<String>, ProgramArgumentsParseError> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut started = false;

    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            in_quotes = !in_quotes;
            started = true;
        } else if ch == '\\' && chars.peek() == Some(&'"') {
            chars.next();
            current.push('"');
            started = true;
        } else if ch.is_whitespace() && !in_quotes {
            if started {
                arguments.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
            started = true;
        }
    }

    if in_quotes {
        return Err(ProgramArgumentsParseError::UnterminatedQuote);
    }

    if started {
        arguments.push(current);
    }

    Ok(arguments)
}

fn format_program_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| format_program_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_program_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.chars().any(char::is_whitespace) && !argument.contains('"')
    {
        return argument.to_owned();
    }

    let trailing_backslashes = argument.chars().rev().take_while(|ch| *ch == '\\').count();
    let quoted_end = argument.len() - trailing_backslashes;
    let (quoted, trailing) = argument.split_at(quoted_end);
    let escaped = quoted.replace('"', "\\\"");
    format!("\"{escaped}\"{trailing}")
}

fn window_title_for_program_spec(hwnd: WindowHandle) -> Option<String> {
    let raw = hwnd.raw() as HWND;
    if raw.is_null() || unsafe { IsWindow(raw) } == 0 {
        return None;
    }

    read_window_text(raw)
        .ok()
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
}

#[derive(Debug)]
enum TabPresetProgramLaunchError {
    Spawn {
        path: String,
        source: std::io::Error,
    },
    WindowNotFound {
        path: String,
        process_id: u32,
    },
    InvalidWindowHandle {
        path: String,
        source: crate::domain::DomainError,
    },
}

impl TabPresetProgramLaunchError {
    fn user_message(&self, language: UiLanguage) -> String {
        match self {
            Self::Spawn { path, source } => {
                if language == UiLanguage::English {
                    format!("{path} could not be started: {source}")
                } else {
                    format!("{path} 실행 실패: {source}")
                }
            }
            Self::WindowNotFound { path, process_id } => {
                if language == UiLanguage::English {
                    format!(
                        "{path} started as process {process_id}, but no top-level window was found"
                    )
                } else {
                    format!(
                        "{path} 프로세스 {process_id}를 실행했지만 top-level window를 찾을 수 없습니다"
                    )
                }
            }
            Self::InvalidWindowHandle { path, source } => {
                if language == UiLanguage::English {
                    format!("{path}: {source}")
                } else {
                    format!("{path}: {}", source.user_message())
                }
            }
        }
    }
}

fn start_tab_preset_program(
    program: &ExternalProgramSpec,
) -> Result<LaunchedTabPresetProgram, TabPresetProgramLaunchError> {
    let path = program.executable_path().to_owned();
    let child = Command::new(program.executable_path())
        .args(program.arguments())
        .spawn()
        .map_err(|source| TabPresetProgramLaunchError::Spawn {
            path: path.clone(),
            source,
        })?;
    Ok(LaunchedTabPresetProgram { path, child })
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct ThreadEntry32 {
    size: u32,
    usage_count: u32,
    thread_id: u32,
    owner_process_id: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

impl ThreadEntry32 {
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            usage_count: 0,
            thread_id: 0,
            owner_process_id: 0,
            base_priority: 0,
            delta_priority: 0,
            flags: 0,
        }
    }
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct ProcessEntry32 {
    size: u32,
    usage_count: u32,
    process_id: u32,
    default_heap_id: usize,
    module_id: u32,
    thread_count: u32,
    parent_process_id: u32,
    priority_class_base: i32,
    flags: u32,
    exe_file: [u16; MAX_PATH_CHARS],
}

impl ProcessEntry32 {
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            usage_count: 0,
            process_id: 0,
            default_heap_id: 0,
            module_id: 0,
            thread_count: 0,
            parent_process_id: 0,
            priority_class_base: 0,
            flags: 0,
            exe_file: [0; MAX_PATH_CHARS],
        }
    }

    fn tree_entry(self) -> ProcessTreeEntry {
        ProcessTreeEntry {
            process_id: self.process_id,
            parent_process_id: self.parent_process_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessTreeEntry {
    process_id: u32,
    parent_process_id: u32,
}

#[derive(Debug, Default)]
struct ProcessTreeChildIndex {
    process_ids: HashSet<u32>,
    children_by_parent: HashMap<u32, Vec<u32>>,
}

impl ProcessTreeChildIndex {
    fn new() -> Self {
        Self {
            process_ids: HashSet::new(),
            children_by_parent: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn from_entries(entries: impl IntoIterator<Item = ProcessTreeEntry>) -> Self {
        let mut index = Self::new();
        for entry in entries {
            index.insert(entry);
        }
        index
    }

    fn insert(&mut self, entry: ProcessTreeEntry) {
        if entry.process_id == 0 {
            return;
        }

        self.process_ids.insert(entry.process_id);
        self.children_by_parent
            .entry(entry.parent_process_id)
            .or_default()
            .push(entry.process_id);
    }

    fn append_new_descendants(
        &self,
        tracked_process_ids: &mut HashSet<u32>,
        discovered: &mut Vec<u32>,
        stack: &mut Vec<u32>,
    ) {
        stack.clear();
        if tracked_process_ids.is_empty() {
            return;
        }

        stack.extend(tracked_process_ids.iter().copied());
        while let Some(parent_process_id) = stack.pop() {
            let Some(children) = self.children_by_parent.get(&parent_process_id) else {
                continue;
            };

            for process_id in children.iter().copied() {
                if tracked_process_ids.insert(process_id) {
                    discovered.push(process_id);
                    stack.push(process_id);
                }
            }
        }
    }

    fn remove_missing_processes(
        &self,
        tracked_process_ids: &mut HashSet<u32>,
        mut keep_process_id: impl FnMut(u32) -> bool,
        removed: &mut Vec<u32>,
    ) {
        tracked_process_ids.retain(|process_id| {
            let is_running = self.process_ids.contains(process_id) || keep_process_id(*process_id);
            if !is_running {
                removed.push(*process_id);
            }
            is_running
        });
    }
}

#[derive(Debug)]
struct ThreadSnapshot {
    handle: Win32Handle,
}

impl ThreadSnapshot {
    fn new() -> Option<Self> {
        let handle = unsafe { create_toolhelp32_snapshot(TH32CS_SNAPTHREAD, 0) };
        if handle == invalid_win32_handle_value() {
            None
        } else {
            Some(Self { handle })
        }
    }

    fn handle(&self) -> Win32Handle {
        self.handle
    }
}

impl Drop for ThreadSnapshot {
    fn drop(&mut self) {
        unsafe {
            close_handle(self.handle);
        }
    }
}

#[derive(Debug)]
struct ProcessSnapshot {
    handle: Win32Handle,
}

impl ProcessSnapshot {
    fn new() -> Option<Self> {
        let handle = unsafe { create_toolhelp32_snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == invalid_win32_handle_value() {
            None
        } else {
            Some(Self { handle })
        }
    }

    fn handle(&self) -> Win32Handle {
        self.handle
    }
}

impl Drop for ProcessSnapshot {
    fn drop(&mut self) {
        unsafe {
            close_handle(self.handle);
        }
    }
}

fn invalid_win32_handle_value() -> Win32Handle {
    -1isize as Win32Handle
}

fn process_tree_child_index() -> Option<ProcessTreeChildIndex> {
    let snapshot = ProcessSnapshot::new()?;
    let mut entry = ProcessEntry32::new();
    if unsafe { process32_first(snapshot.handle(), &mut entry) } == 0 {
        return None;
    }

    let mut index = ProcessTreeChildIndex::new();
    loop {
        index.insert(entry.tree_entry());
        if unsafe { process32_next(snapshot.handle(), &mut entry) } == 0 {
            break;
        }
    }

    Some(index)
}

#[cfg(test)]
fn descendant_process_ids_from_entries(
    known_process_ids: &HashSet<u32>,
    entries: &[ProcessTreeEntry],
) -> Vec<u32> {
    let index = ProcessTreeChildIndex::from_entries(entries.iter().copied());
    let mut expanded_process_ids = known_process_ids.clone();
    let mut descendants = Vec::new();
    let mut stack = Vec::new();
    index.append_new_descendants(&mut expanded_process_ids, &mut descendants, &mut stack);
    descendants
}

#[derive(Debug)]
struct ProcessWindowSearch<'a, 'b> {
    remaining_process_ids: &'a mut HashSet<u32>,
    hwnds: &'b mut HashMap<u32, HWND>,
}

#[derive(Debug)]
struct ProcessThreadWindowSearch<'a> {
    process_id: u32,
    hwnds: &'a mut HashMap<u32, HWND>,
}

fn top_level_windows_for_processes(
    remaining_process_ids: &mut HashSet<u32>,
    hwnds: &mut HashMap<u32, HWND>,
    scan_threads: bool,
) -> bool {
    if remaining_process_ids.is_empty() {
        return false;
    }

    enum_top_level_windows_for_processes(remaining_process_ids, hwnds);
    if remaining_process_ids.is_empty() || !scan_threads {
        if scan_threads {
            remaining_process_ids.clear();
        }
        return false;
    }

    let window_count_before_thread_scan = hwnds.len();
    top_level_thread_windows_for_processes(remaining_process_ids, hwnds);
    let found_thread_window = hwnds.len() > window_count_before_thread_scan;
    if found_thread_window {
        remaining_process_ids.retain(|process_id| !hwnds.contains_key(process_id));
    }
    found_thread_window
}

fn top_level_thread_windows_for_processes(
    process_ids: &HashSet<u32>,
    hwnds: &mut HashMap<u32, HWND>,
) -> bool {
    let mut missing_process_count = process_ids
        .iter()
        .filter(|process_id| !hwnds.contains_key(process_id))
        .count();
    if missing_process_count == 0 {
        return false;
    }

    let Some(snapshot) = ThreadSnapshot::new() else {
        return false;
    };

    let mut entry = ThreadEntry32::new();
    if unsafe { thread32_first(snapshot.handle(), &mut entry) } == 0 {
        return false;
    }

    loop {
        if process_ids.contains(&entry.owner_process_id)
            && !hwnds.contains_key(&entry.owner_process_id)
        {
            top_level_windows_for_process_thread(entry.owner_process_id, entry.thread_id, hwnds);
            if hwnds.contains_key(&entry.owner_process_id) {
                missing_process_count = missing_process_count.saturating_sub(1);
                if missing_process_count == 0 {
                    break;
                }
            }
        }

        if unsafe { thread32_next(snapshot.handle(), &mut entry) } == 0 {
            break;
        }
    }

    true
}

fn top_level_windows_for_process_thread(
    process_id: u32,
    thread_id: u32,
    hwnds: &mut HashMap<u32, HWND>,
) {
    let mut search = ProcessThreadWindowSearch { process_id, hwnds };

    unsafe {
        enum_thread_windows(
            thread_id,
            Some(enum_thread_windows_for_process_window),
            (&mut search as *mut ProcessThreadWindowSearch<'_>) as LPARAM,
        );
    }
}

fn enum_top_level_windows_for_processes(
    remaining_process_ids: &mut HashSet<u32>,
    hwnds: &mut HashMap<u32, HWND>,
) {
    let mut search = ProcessWindowSearch {
        remaining_process_ids,
        hwnds,
    };

    unsafe {
        EnumWindows(
            Some(enum_windows_for_process_window),
            (&mut search as *mut ProcessWindowSearch<'_, '_>) as LPARAM,
        );
    }
}

unsafe extern "system" fn enum_thread_windows_for_process_window(
    hwnd: HWND,
    lparam: LPARAM,
) -> i32 {
    let search = unsafe { &mut *(lparam as *mut ProcessThreadWindowSearch<'_>) };
    if !top_level_window_matches_process(hwnd, search.process_id) {
        return 1;
    }

    insert_topmost_process_window(search.hwnds, search.process_id, hwnd);
    1
}

unsafe extern "system" fn enum_windows_for_process_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let search = unsafe { &mut *(lparam as *mut ProcessWindowSearch<'_, '_>) };
    let process_id = window_process_id(hwnd);
    if !search.remaining_process_ids.contains(&process_id) {
        return 1;
    }

    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return 1;
    }
    if !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null() {
        return 1;
    }

    insert_topmost_process_window(search.hwnds, process_id, hwnd);
    search.remaining_process_ids.remove(&process_id);
    if search.remaining_process_ids.is_empty() {
        0
    } else {
        1
    }
}

fn top_level_window_matches_process(hwnd: HWND, process_id: u32) -> bool {
    top_level_window_is_visible_root_unowned(hwnd) && window_process_id(hwnd) == process_id
}

fn top_level_window_is_visible_root_unowned(hwnd: HWND) -> bool {
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return false;
    }
    if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return false;
    }
    if !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null() {
        return false;
    }

    true
}

fn window_process_id(hwnd: HWND) -> u32 {
    let mut process_id = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut process_id);
    }
    process_id
}

fn insert_topmost_process_window(hwnds: &mut HashMap<u32, HWND>, process_id: u32, hwnd: HWND) {
    if let Some(existing) = hwnds.get(&process_id).copied()
        && !window_is_above(hwnd, existing)
    {
        return;
    }

    hwnds.insert(process_id, hwnd);
}

fn window_is_above(hwnd: HWND, other: HWND) -> bool {
    if hwnd == other {
        return false;
    }

    let mut current = other;
    loop {
        current = unsafe { GetWindow(current, GW_HWNDPREV) };
        if current.is_null() {
            return false;
        }
        if current == hwnd {
            return true;
        }
    }
}

fn log_app_error(error: &AppError) {
    eprintln!("{error}");
    if let Some(source) = error.source() {
        eprintln!("cause: {source}");
    }
}

fn log_tab_ux_trace(event: &'static str, detail: fmt::Arguments<'_>) {
    eprintln!("tab-ux event={event} {detail}");
}

fn optional_tab_id_trace_text(tab_id: Option<TabId>) -> String {
    match tab_id {
        Some(tab_id) => tab_id.value().to_string(),
        None => "end".to_owned(),
    }
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        self.teardown_drop_detection();
        self.teardown_splitter_overlay();
        self.shutdown_once(ShutdownMode::Forced);
        self.destroy_icons();
    }
}

fn next_tab_number(next_tab_id: u64) -> u32 {
    u32::try_from(next_tab_id).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceUiVisibilityTransition {
    previous_visible: bool,
    desired_visible: bool,
}

impl WorkspaceUiVisibilityTransition {
    const fn from_current(current_visible: bool) -> Self {
        Self {
            previous_visible: current_visible,
            desired_visible: !current_visible,
        }
    }
}

// Win32 can synchronously reenter message handlers while menu/frame chrome is
// being changed, so layout and menu refresh paths must see the pending target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceUiVisibility {
    committed_visible: bool,
    transition: Option<WorkspaceUiVisibilityTransition>,
}

impl WorkspaceUiVisibility {
    const fn new(visible: bool) -> Self {
        Self {
            committed_visible: visible,
            transition: None,
        }
    }

    const fn effective_visible(self) -> bool {
        match self.transition {
            Some(transition) => transition.desired_visible,
            None => self.committed_visible,
        }
    }

    #[cfg(test)]
    const fn committed_visible(self) -> bool {
        self.committed_visible
    }

    fn begin_toggle(&mut self) -> WorkspaceUiVisibilityTransition {
        let transition = WorkspaceUiVisibilityTransition::from_current(self.committed_visible);
        self.transition = Some(transition);
        transition
    }

    fn commit(&mut self, transition: WorkspaceUiVisibilityTransition) {
        self.committed_visible = transition.desired_visible;
        self.transition = None;
    }

    fn rollback(&mut self, transition: WorkspaceUiVisibilityTransition) {
        self.committed_visible = transition.previous_visible;
        self.transition = None;
    }
}

fn window_style_for_workspace_ui_visibility(
    current_style: isize,
    workspace_ui_visible: bool,
    window_maximized: bool,
) -> isize {
    let title_bar_style = WS_CAPTION as isize;
    let resize_frame_style = WS_THICKFRAME as isize;
    if workspace_ui_visible {
        current_style | title_bar_style | resize_frame_style
    } else if window_maximized {
        current_style & !title_bar_style & !resize_frame_style
    } else {
        current_style & !title_bar_style
    }
}

fn main_menu_visible_for_workspace_ui(workspace_ui_visible: bool) -> bool {
    workspace_ui_visible
}

fn main_menu_needs_refresh_after_size(
    workspace_ui_visible: bool,
    cached_maximized: Option<bool>,
    current_maximized: bool,
) -> bool {
    main_menu_visible_for_workspace_ui(workspace_ui_visible)
        && cached_maximized != Some(current_maximized)
}

fn hidden_workspace_ui_region_size(
    window: RECT,
    client_origin: ScreenPoint,
) -> Result<(i32, i32), Win32StatusFailure> {
    let width = window.right.checked_sub(window.left).ok_or_else(|| {
        Win32StatusFailure::new(
            "GetWindowRect",
            0,
            "작업 영역 UI window region 너비를 계산할 수 없습니다.",
        )
    })?;
    let client_top = client_origin.y.checked_sub(window.top).ok_or_else(|| {
        Win32StatusFailure::new(
            "ClientToScreen",
            0,
            "작업 영역 UI window region 상단 좌표를 계산할 수 없습니다.",
        )
    })?;
    let height = client_top.checked_add(TAB_BAR_HEIGHT).ok_or_else(|| {
        Win32StatusFailure::new(
            "ClientToScreen",
            0,
            "작업 영역 UI window region 높이를 계산할 수 없습니다.",
        )
    })?;

    if width <= 0 {
        return Err(Win32StatusFailure::new(
            "GetWindowRect",
            0,
            "작업 영역 UI window region 너비가 올바르지 않습니다.",
        ));
    }
    if height <= 0 {
        return Err(Win32StatusFailure::new(
            "ClientToScreen",
            0,
            "작업 영역 UI window region 높이가 올바르지 않습니다.",
        ));
    }

    Ok((width, height))
}

fn drop_uses_workspace_hit_test(
    workspace_ui_visible: bool,
    workspace_options: WorkspaceOptions,
) -> bool {
    workspace_ui_visible || workspace_options.dock_hidden_workspace_ui()
}

fn ctrl_key_is_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_CONTROL as i32) & 0x8000u16 as i16) != 0 }
}

fn splitter_overlay_should_show(
    workspace_ui_visible: bool,
    dock_hidden_workspace_ui: bool,
    is_minimized: bool,
    pointer_drag_active: bool,
    ctrl_down: bool,
) -> bool {
    splitter_overlay_workspace_enabled(workspace_ui_visible, dock_hidden_workspace_ui)
        && !is_minimized
        && !pointer_drag_active
        && ctrl_down
}

fn splitter_overlay_workspace_enabled(
    workspace_ui_visible: bool,
    dock_hidden_workspace_ui: bool,
) -> bool {
    workspace_ui_visible || dock_hidden_workspace_ui
}

fn log_workspace_ui_chrome_rollback_error(error: Win32StatusFailure) {
    eprintln!(
        "workspace UI chrome rollback failed: {} failed with GetLastError={}",
        error.api, error.last_error
    );
}

fn restore_hidden_maximized_bounds_after_chrome_failure<T>(
    result: Result<T, Win32StatusFailure>,
    previous_bounds: Option<RECT>,
    mut restore_bounds: impl FnMut(RECT) -> Result<(), Win32StatusFailure>,
) -> Result<T, Win32StatusFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Some(bounds) = previous_bounds
                && let Err(rollback_error) = restore_bounds(bounds)
            {
                log_workspace_ui_chrome_rollback_error(rollback_error);
            }
            Err(error)
        }
    }
}

fn tab_drag_exceeds_move_threshold(start: ClientPoint, current: ClientPoint) -> bool {
    let dx = current.x.abs_diff(start.x);
    let dy = current.y.abs_diff(start.y);
    dx >= TAB_DRAG_MOVE_THRESHOLD as u32 || dy >= TAB_DRAG_MOVE_THRESHOLD as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabPressAction {
    Pending(PendingTabAction),
    Close(TabId),
}

fn tab_press_action_from_hit(hit: TabHit, workspace_ui_visible: bool) -> TabPressAction {
    match hit.target {
        TabHitTarget::Body => {
            TabPressAction::Pending(pending_tab_action_for_workspace_ui(workspace_ui_visible))
        }
        TabHitTarget::CloseButton => TabPressAction::Close(hit.tab_id),
    }
}

fn pending_tab_action_for_workspace_ui(workspace_ui_visible: bool) -> PendingTabAction {
    if workspace_ui_visible {
        PendingTabAction::ClickOrReorder
    } else {
        PendingTabAction::ClickOrWindowMove
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingTabMoveOutcome {
    ContinueClick,
    StartReorder(TabId),
    StartWindowMove,
}

fn pending_tab_move_outcome(
    pending: PendingTabClick,
    current: ClientPoint,
) -> PendingTabMoveOutcome {
    if !tab_drag_exceeds_move_threshold(pending.point, current) {
        return PendingTabMoveOutcome::ContinueClick;
    }

    match pending.action {
        PendingTabAction::ClickOrReorder => PendingTabMoveOutcome::StartReorder(pending.tab_id),
        PendingTabAction::ClickOrWindowMove => PendingTabMoveOutcome::StartWindowMove,
    }
}

fn pending_tab_release_switch_target(
    pending: PendingTabClick,
    released_body_tab: Option<TabId>,
) -> Option<TabId> {
    released_body_tab.filter(|tab_id| *tab_id == pending.tab_id)
}

fn tab_body_target_from_hit(hit: Option<TabHit>) -> Option<TabId> {
    hit.and_then(|hit| match hit.target {
        TabHitTarget::Body => Some(hit.tab_id),
        TabHitTarget::CloseButton => None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabContextAction {
    Rename,
    Close,
    CloseOther,
}

fn tab_context_action_from_command(command_id: u16) -> Option<TabContextAction> {
    match command_id {
        CMD_TAB_RENAME_CONTEXT => Some(TabContextAction::Rename),
        CMD_TAB_CLOSE_CONTEXT => Some(TabContextAction::Close),
        CMD_TAB_CLOSE_OTHER_CONTEXT => Some(TabContextAction::CloseOther),
        _ => None,
    }
}

fn popup_selected_command(selected: i32, hwnd: HWND) -> Option<u16> {
    if selected == 0 || hwnd.is_null() {
        return None;
    }
    u16::try_from(selected).ok()
}

fn command_for_index(base: u16, end: u16, index: usize) -> Option<u16> {
    let offset = u16::try_from(index).ok()?;
    let command = base.checked_add(offset)?;
    if command < end { Some(command) } else { None }
}

fn command_index_from_range(command_id: u16, base: u16, end: u16) -> Option<usize> {
    if (base..end).contains(&command_id) {
        Some(usize::from(command_id - base))
    } else {
        None
    }
}

fn tab_overflow_command_for_index(index: usize) -> Option<u16> {
    command_for_index(CMD_TAB_OVERFLOW_BASE, CMD_TAB_OVERFLOW_END, index)
}

fn append_preset_menu_items<'a>(
    menu: HMENU,
    preset_names: impl IntoIterator<Item = &'a str>,
    base: u16,
    end: u16,
) -> bool {
    let mut appended = false;
    for (index, name) in preset_names.into_iter().enumerate() {
        let Some(command) = command_for_index(base, end, index) else {
            break;
        };
        append_menu(menu, command, name);
        appended = true;
    }
    appended
}

impl TabContextAction {
    const fn trace_name(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Close => "close",
            Self::CloseOther => "close-other",
        }
    }
}

fn tab_context_target_from_hit(hit: Option<TabHit>) -> Option<TabId> {
    hit.map(|hit| hit.tab_id)
}

fn close_other_tab_targets(tab_ids: &[TabId], target_tab_id: TabId) -> Option<Vec<TabId>> {
    let mut found_target = false;
    let mut targets = Vec::new();

    for tab_id in tab_ids {
        if *tab_id == target_tab_id {
            found_target = true;
        } else {
            targets.push(*tab_id);
        }
    }

    if found_target { Some(targets) } else { None }
}

#[derive(Debug, Clone, Copy)]
struct PendingTabClick {
    tab_id: TabId,
    point: ClientPoint,
    action: PendingTabAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingTabAction {
    ClickOrReorder,
    ClickOrWindowMove,
}

#[derive(Debug, Clone, Copy)]
struct TabReorderDrag {
    tab_id: TabId,
    insertion: Option<TabReorderInsertion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabReorderInsertion {
    before_tab_id: Option<TabId>,
    x: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabHit {
    tab_id: TabId,
    target: TabHitTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackedWindowRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl TrackedWindowRect {
    #[cfg(test)]
    const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    const fn from_rect(rect: RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn to_domain_rect(self) -> Option<Rect> {
        let width = self.right.checked_sub(self.left)?;
        let height = self.bottom.checked_sub(self.top)?;
        Rect::new(self.left, self.top, width, height).ok()
    }

    fn contains_point(self, point: ScreenPoint) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    fn differs_by_at_least(self, other: Self, threshold: i32) -> bool {
        if threshold <= 0 {
            return self != other;
        }

        let threshold = i64::from(threshold);
        [
            i64::from(self.left) - i64::from(other.left),
            i64::from(self.top) - i64::from(other.top),
            i64::from(self.right) - i64::from(other.right),
            i64::from(self.bottom) - i64::from(other.bottom),
        ]
        .into_iter()
        .any(|delta| delta.abs() >= threshold)
    }
}

fn tracked_window_rect(raw: HWND) -> Option<TrackedWindowRect> {
    if raw.is_null() {
        return None;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { GetWindowRect(raw, &mut rect) };
    if ok == 0 {
        None
    } else {
        Some(TrackedWindowRect::from_rect(rect))
    }
}

#[cfg(test)]
fn hwnd_is_owned_by_with(hwnd: HWND, owner: HWND, mut owner_of: impl FnMut(HWND) -> HWND) -> bool {
    if hwnd.is_null() || owner.is_null() {
        return false;
    }

    owner_of(hwnd) == owner
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExternalRootCandidate {
    hwnd: WindowHandle,
    docker_owned: bool,
}

fn external_root_candidate_from_hwnd_with(
    hwnd: HWND,
    docker_hwnd: HWND,
    mut root_of: impl FnMut(HWND) -> HWND,
    mut owner_of: impl FnMut(HWND) -> HWND,
) -> Option<ExternalRootCandidate> {
    if hwnd.is_null() {
        return None;
    }

    let root = root_of(hwnd);
    if root.is_null() || root == docker_hwnd {
        return None;
    }

    let hwnd = WindowHandle::new(root as isize).ok()?;
    let docker_owned = !docker_hwnd.is_null() && owner_of(root) == docker_hwnd;

    Some(ExternalRootCandidate { hwnd, docker_owned })
}

#[derive(Debug, Clone, Copy)]
struct DropCandidate {
    hwnd: WindowHandle,
    initial_rect: TrackedWindowRect,
    window_moved: bool,
}

impl DropCandidate {
    const fn new(hwnd: WindowHandle, initial_rect: TrackedWindowRect) -> Self {
        Self {
            hwnd,
            initial_rect,
            window_moved: false,
        }
    }

    fn observe_rect(&mut self, rect: TrackedWindowRect, threshold: i32) {
        if rect.differs_by_at_least(self.initial_rect, threshold) {
            self.window_moved = true;
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct DropTracker {
    left_button_down: bool,
    candidate: Option<DropCandidate>,
    suppress_drop: bool,
}

impl DropTracker {
    fn begin_press(&mut self) {
        if !self.left_button_down {
            *self = Self {
                left_button_down: true,
                candidate: None,
                suppress_drop: false,
            };
        }
    }

    fn is_tracking(&self) -> bool {
        self.left_button_down
    }

    fn needs_candidate(&self) -> bool {
        self.left_button_down && self.candidate.is_none() && !self.suppress_drop
    }

    fn set_candidate(&mut self, hwnd: WindowHandle, rect: TrackedWindowRect) {
        if self.needs_candidate() {
            self.candidate = Some(DropCandidate::new(hwnd, rect));
        }
    }

    fn candidate_hwnd(&self) -> Option<WindowHandle> {
        self.candidate.map(|candidate| candidate.hwnd)
    }

    fn observe_candidate_rect(&mut self, rect: TrackedWindowRect, threshold: i32) {
        if let Some(candidate) = self.candidate.as_mut() {
            candidate.observe_rect(rect, threshold);
        }
    }

    fn suppress_drop(&mut self) {
        self.candidate = None;
        self.suppress_drop = true;
    }

    fn finish_press(&mut self) -> Option<WindowHandle> {
        if !self.left_button_down {
            return None;
        }

        let candidate = if self.suppress_drop {
            None
        } else {
            self.candidate
                .take()
                .filter(|candidate| candidate.window_moved)
                .map(|candidate| candidate.hwnd)
        };
        *self = Self::default();
        candidate
    }
}

fn visible_tab_label_bounds(layout: TabStripLayout, tab_count: usize) -> (usize, usize) {
    let first_visible_index = layout.first_visible_index.min(tab_count);
    let visible_end_index = layout.visible_end_index().min(tab_count);
    (first_visible_index, visible_end_index)
}

fn sync_paint_tab_labels<'a>(
    paint_tab_labels: &mut Vec<CachedTabLabel>,
    labels: impl IntoIterator<Item = (TabId, &'a str)>,
) {
    let mut write_index = 0;

    for (tab_id, label) in labels {
        if write_index < paint_tab_labels.len() {
            paint_tab_labels[write_index].set(tab_id, label);
        } else {
            paint_tab_labels.push(CachedTabLabel::new(tab_id, label));
        }

        write_index += 1;
    }

    paint_tab_labels.truncate(write_index);
}

#[derive(Debug, Clone)]
struct TabRect<'a> {
    index: usize,
    tab_id: TabId,
    rect: UiRect,
    label: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct ButtonSpec {
    command: u16,
    width: i32,
}

#[derive(Debug, Clone, Copy)]
struct ButtonRect {
    command: u16,
    rect: UiRect,
}

#[derive(Debug, Clone)]
struct CachedTabLabel {
    tab_id: TabId,
    label: WideText,
}

impl CachedTabLabel {
    fn new(tab_id: TabId, label: &str) -> Self {
        Self {
            tab_id,
            label: WideText::new(label),
        }
    }

    fn set(&mut self, tab_id: TabId, label: &str) {
        self.tab_id = tab_id;
        self.label.replace_str(label);
    }

    fn tab_id(&self) -> TabId {
        self.tab_id
    }

    fn wide(&self) -> &[u16] {
        self.label.wide()
    }
}

#[derive(Debug, Clone)]
struct WideText {
    text: String,
    wide: Vec<u16>,
}

impl WideText {
    fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let wide = wide_null(&text);
        Self { text, wide }
    }

    fn replace(&mut self, text: String) {
        if self.text == text {
            return;
        }

        self.text = text;
        replace_wide_null(&mut self.wide, &self.text);
    }

    fn replace_str(&mut self, text: &str) {
        if self.text == text {
            return;
        }

        self.text.clear();
        self.text.push_str(text);
        replace_wide_null(&mut self.wide, &self.text);
    }

    fn replace_with(&mut self, write: impl FnOnce(&mut String)) {
        self.text.clear();
        write(&mut self.text);
        replace_wide_null(&mut self.wide, &self.text);
    }

    fn wide(&self) -> &[u16] {
        &self.wide
    }
}

#[derive(Debug, Default)]
struct PaintLayoutCache {
    key: Option<PaintLayoutCacheKey>,
    regions: Vec<CachedPaintRegion>,
    splitters: Vec<UiRect>,
}

impl PaintLayoutCache {
    fn matches(&self, key: PaintLayoutCacheKey) -> bool {
        self.key == Some(key)
    }

    fn region_rect(&self, key: PaintLayoutCacheKey, region_id: RegionId) -> Option<UiRect> {
        if !self.matches(key) {
            return None;
        }

        self.regions
            .iter()
            .find(|region| region.region_id() == region_id)
            .map(CachedPaintRegion::rect)
    }

    fn rebuild(
        &mut self,
        key: PaintLayoutCacheKey,
        regions: &[RegionRect],
        splitters: &[SplitterRect],
    ) {
        self.rebuild_regions(key.content_top, key.bounds, regions);
        self.rebuild_splitters(key.content_top, key.bounds, splitters);
        self.key = Some(key);
    }

    fn invalidate(&mut self) {
        self.key = None;
    }

    fn clear(&mut self) {
        self.key = None;
        self.regions.clear();
        self.splitters.clear();
    }

    fn regions_mut(&mut self) -> &mut [CachedPaintRegion] {
        &mut self.regions
    }

    fn splitters(&self) -> &[UiRect] {
        &self.splitters
    }

    fn rebuild_regions(&mut self, content_top: i32, bounds: Rect, regions: &[RegionRect]) {
        let mut write_index = 0;

        for region in regions {
            let Some(rect) = layout_rect_to_client_rect(content_top, bounds, region.rect()) else {
                continue;
            };
            let region_id = region.region_id();

            if write_index < self.regions.len()
                && self.regions[write_index].region_id() == region_id
            {
                self.regions[write_index].set_rect(rect);
            } else if write_index < self.regions.len() {
                self.regions[write_index] = CachedPaintRegion::new(region_id, rect);
            } else {
                self.regions.push(CachedPaintRegion::new(region_id, rect));
            }

            write_index += 1;
        }

        self.regions.truncate(write_index);
    }

    fn rebuild_splitters(&mut self, content_top: i32, bounds: Rect, splitters: &[SplitterRect]) {
        let mut write_index = 0;

        for splitter in splitters {
            let Some(rect) = layout_rect_to_client_rect(content_top, bounds, splitter.rect())
            else {
                continue;
            };

            if write_index < self.splitters.len() {
                self.splitters[write_index] = rect;
            } else {
                self.splitters.push(rect);
            }

            write_index += 1;
        }

        self.splitters.truncate(write_index);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintLayoutCacheKey {
    tab_id: TabId,
    bounds: Rect,
    content_top: i32,
}

#[derive(Debug, Clone)]
struct CachedPaintRegion {
    region_id: RegionId,
    rect: UiRect,
    title_key: Option<PaintRegionTitleKey>,
    title: WideText,
}

impl CachedPaintRegion {
    fn new(region_id: RegionId, rect: UiRect) -> Self {
        Self {
            region_id,
            rect,
            title_key: None,
            title: WideText::new(""),
        }
    }

    fn region_id(&self) -> RegionId {
        self.region_id
    }

    fn rect(&self) -> UiRect {
        self.rect
    }

    fn set_rect(&mut self, rect: UiRect) {
        self.rect = rect;
    }

    fn title_wide(&mut self, language: UiLanguage, is_occupied: bool) -> &[u16] {
        let key = PaintRegionTitleKey {
            language,
            region_id: self.region_id,
            is_occupied,
        };
        if self.title_key != Some(key) {
            self.title.replace_with(|text| {
                write_region_title_text(text, language, self.region_id, is_occupied);
            });
            self.title_key = Some(key);
        }
        self.title.wide()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintRegionTitleKey {
    language: UiLanguage,
    region_id: RegionId,
    is_occupied: bool,
}

struct TextInputDialogState {
    prompt: Vec<u16>,
    initial_value: Vec<u16>,
    ok_label: Vec<u16>,
    cancel_label: Vec<u16>,
    edit_hwnd: HWND,
    result: Option<String>,
    read_error: Option<u32>,
    done: bool,
}

impl TextInputDialogState {
    fn new(prompt: &str, initial_value: &str, ok_label: &str, cancel_label: &str) -> Self {
        Self {
            prompt: wide_null(prompt),
            initial_value: wide_null(initial_value),
            ok_label: wide_null(ok_label),
            cancel_label: wide_null(cancel_label),
            edit_hwnd: null_mut(),
            result: None,
            read_error: None,
            done: false,
        }
    }
}

struct AboutDialogState {
    language: UiLanguage,
    version: Vec<u16>,
    link_markup: Vec<u16>,
    ok_label: Vec<u16>,
    done: bool,
}

impl AboutDialogState {
    fn new(language: UiLanguage) -> Self {
        Self {
            language,
            version: wide_null(&format!("j3GridDocker {}", env!("CARGO_PKG_VERSION"))),
            link_markup: wide_null(&about_dialog_link_markup()),
            ok_label: wide_null(ui_text(language, "OK", "확인")),
            done: false,
        }
    }
}

unsafe extern "system" fn about_dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if create.is_null() {
            return 0;
        }

        let state = unsafe { (*create).lpCreateParams as *mut AboutDialogState };
        if state.is_null() {
            return 0;
        }

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        return 1;
    }

    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AboutDialogState };
    match message {
        WM_CREATE => {
            if state.is_null() {
                return -1;
            }

            if create_about_dialog_controls(hwnd, state) {
                0
            } else {
                -1
            }
        }
        WM_COMMAND => {
            if !state.is_null() {
                match low_word(wparam) {
                    ABOUT_DIALOG_OK_ID => close_about_dialog(hwnd, state),
                    ABOUT_DIALOG_LINK_ID => open_about_link_from_dialog(hwnd, state),
                    _ => {}
                }
            }
            0
        }
        WM_NOTIFY => {
            if !state.is_null() {
                handle_about_dialog_notify(hwnd, state, lparam);
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            unsafe {
                SetBkMode(hdc, TRANSPARENT as i32);
                GetSysColorBrush(COLOR_WINDOW) as LRESULT
            }
        }
        WM_CLOSE => {
            if !state.is_null() {
                close_about_dialog(hwnd, state);
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_NCDESTROY => {
            if !state.is_null() {
                unsafe {
                    (*state).done = true;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_about_dialog_controls(hwnd: HWND, state: *mut AboutDialogState) -> bool {
    set_dialog_font(hwnd);
    let static_class = wide_null("STATIC");
    let button_class = wide_null("BUTTON");

    let version = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).version.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            300,
            22,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if version.is_null() {
        return false;
    }
    set_dialog_font(version);

    let mut link = unsafe {
        CreateWindowExW(
            0,
            WC_LINK,
            (*state).link_markup.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            20,
            54,
            300,
            26,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if link.is_null() {
        let link_label = wide_null(ABOUT_LINK_URL);
        link = unsafe {
            CreateWindowExW(
                0,
                button_class.as_ptr(),
                link_label.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                20,
                54,
                300,
                26,
                hwnd,
                control_id(ABOUT_DIALOG_LINK_ID),
                null_mut(),
                null_mut(),
            )
        };
    }
    if link.is_null() {
        return false;
    }
    set_dialog_font(link);

    let ok = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            (*state).ok_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            256,
            96,
            78,
            26,
            hwnd,
            control_id(ABOUT_DIALOG_OK_ID),
            null_mut(),
            null_mut(),
        )
    };
    if ok.is_null() {
        return false;
    }
    set_dialog_font(ok);

    unsafe {
        SetFocus(ok);
    }
    true
}

fn handle_about_dialog_notify(hwnd: HWND, state: *mut AboutDialogState, lparam: LPARAM) {
    if !about_dialog_link_activated(lparam) {
        return;
    }

    open_about_link_from_dialog(hwnd, state);
}

fn open_about_link_from_dialog(hwnd: HWND, state: *mut AboutDialogState) {
    if let Err(last_error) = open_url_in_browser(hwnd, ABOUT_LINK_URL) {
        let message = wide_null(ui_text(
            unsafe { (*state).language },
            "Could not open the link in a browser.",
            "브라우저에서 링크를 열 수 없습니다.",
        ));
        let title = wide_null(ui_text(
            unsafe { (*state).language },
            "About j3GridDocker",
            "j3GridDocker 정보",
        ));
        let _ = last_error;
        unsafe {
            MessageBoxW(
                hwnd,
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
}

fn about_dialog_link_activated(lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }

    let header = unsafe { &*(lparam as *const NMHDR) };
    header.code == NM_CLICK || header.code == NM_RETURN
}

fn open_url_in_browser(owner: HWND, url: &str) -> Result<(), u32> {
    let operation = wide_null("open");
    let url = wide_null(url);
    let result = unsafe {
        ShellExecuteW(
            owner,
            operation.as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if shell_execute_succeeded(result) {
        Ok(())
    } else {
        Err(result as usize as u32)
    }
}

fn shell_execute_succeeded(result: HINSTANCE) -> bool {
    (result as isize) > 32
}

fn about_dialog_link_markup() -> String {
    format!(r#"<a href="{ABOUT_LINK_URL}">{ABOUT_LINK_URL}</a>"#)
}

fn close_about_dialog(hwnd: HWND, state: *mut AboutDialogState) {
    unsafe {
        (*state).done = true;
        DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn text_input_dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if create.is_null() {
            return 0;
        }

        let state = unsafe { (*create).lpCreateParams as *mut TextInputDialogState };
        if state.is_null() {
            return 0;
        }

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        return 1;
    }

    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TextInputDialogState };
    match message {
        WM_CREATE => {
            if state.is_null() {
                return -1;
            }

            if create_text_input_dialog_controls(hwnd, state) {
                0
            } else {
                -1
            }
        }
        WM_COMMAND => {
            if !state.is_null() {
                match low_word(wparam) {
                    INPUT_DIALOG_OK_ID => accept_text_input_dialog(hwnd, state),
                    INPUT_DIALOG_CANCEL_ID => cancel_text_input_dialog(hwnd, state),
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            if !state.is_null() {
                cancel_text_input_dialog(hwnd, state);
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_NCDESTROY => {
            if !state.is_null() {
                unsafe {
                    (*state).done = true;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_text_input_dialog_controls(hwnd: HWND, state: *mut TextInputDialogState) -> bool {
    let static_class = wide_null("STATIC");
    let edit_class = wide_null("EDIT");
    let button_class = wide_null("BUTTON");

    let prompt = unsafe { (*state).prompt.as_ptr() };
    let label = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            prompt,
            WS_CHILD | WS_VISIBLE,
            14,
            14,
            340,
            18,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if label.is_null() {
        return false;
    }

    let initial_value = unsafe { (*state).initial_value.as_ptr() };
    let edit = unsafe {
        CreateWindowExW(
            0,
            edit_class.as_ptr(),
            initial_value,
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            14,
            38,
            340,
            24,
            hwnd,
            control_id(INPUT_DIALOG_EDIT_ID),
            null_mut(),
            null_mut(),
        )
    };
    if edit.is_null() {
        return false;
    }
    unsafe {
        (*state).edit_hwnd = edit;
    }

    let ok = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            (*state).ok_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            190,
            76,
            78,
            26,
            hwnd,
            control_id(INPUT_DIALOG_OK_ID),
            null_mut(),
            null_mut(),
        )
    };
    if ok.is_null() {
        return false;
    }

    let cancel = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            (*state).cancel_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            276,
            76,
            78,
            26,
            hwnd,
            control_id(INPUT_DIALOG_CANCEL_ID),
            null_mut(),
            null_mut(),
        )
    };
    if cancel.is_null() {
        return false;
    }

    unsafe {
        SetFocus(edit);
    }
    true
}

fn accept_text_input_dialog(hwnd: HWND, state: *mut TextInputDialogState) {
    match read_window_text(unsafe { (*state).edit_hwnd }) {
        Ok(text) => unsafe {
            (*state).result = Some(text);
            (*state).done = true;
            DestroyWindow(hwnd);
        },
        Err(last_error) => unsafe {
            (*state).read_error = Some(last_error);
            (*state).done = true;
            DestroyWindow(hwnd);
        },
    }
}

fn cancel_text_input_dialog(hwnd: HWND, state: *mut TextInputDialogState) {
    unsafe {
        (*state).result = None;
        (*state).done = true;
        DestroyWindow(hwnd);
    }
}

fn read_window_text(hwnd: HWND) -> Result<String, u32> {
    unsafe {
        SetLastError(0);
    }
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let last_error = unsafe { GetLastError() };
    if length == 0 && last_error != 0 {
        return Err(last_error);
    }
    let (length, capacity) = window_text_length_and_capacity(length, last_error)?;
    let mut buffer = vec![0u16; capacity];
    let max_count = i32::try_from(buffer.len()).map_err(|_| 0u32)?;

    unsafe {
        SetLastError(0);
    }
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), max_count) };
    if copied == 0 {
        let last_error = unsafe { GetLastError() };
        if let Some(error) = get_window_text_zero_return_error(length, last_error) {
            return Err(error);
        }
    }

    let copied = usize::try_from(copied).map_err(|_| unsafe { GetLastError() })?;
    buffer.truncate(copied);
    Ok(String::from_utf16_lossy(&buffer))
}

fn window_text_length_and_capacity(length: i32, last_error: u32) -> Result<(usize, usize), u32> {
    let length = usize::try_from(length).map_err(|_| last_error)?;
    if length > MAX_WINDOW_TEXT_CHARS {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    let Some(capacity) = length.checked_add(1) else {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    };
    Ok((length, capacity))
}

fn get_window_text_zero_return_error(expected_length: usize, last_error: u32) -> Option<u32> {
    if last_error != 0 {
        Some(last_error)
    } else if expected_length > 0 {
        Some(0)
    } else {
        None
    }
}

fn control_id(id: u16) -> HMENU {
    usize::from(id) as HMENU
}

unsafe extern "system" fn drop_move_event_proc(
    _hook: HWinEventHook,
    event: u32,
    hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    DROP_MOVE_EVENT_ROUTER.post_move_size_event(event, hwnd);
}

unsafe extern "system" fn window_name_change_event_proc(
    _hook: HWinEventHook,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event == EVENT_OBJECT_NAMECHANGE && object_id == OBJID_WINDOW && child_id == CHILDID_SELF {
        WINDOW_NAME_CHANGE_EVENT_ROUTER.post_name_change_event(hwnd);
    }
}

unsafe extern "system" fn tab_preset_program_window_event_proc(
    _hook: HWinEventHook,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if (event == EVENT_OBJECT_CREATE || event == EVENT_OBJECT_SHOW)
        && object_id == OBJID_WINDOW
        && child_id == CHILDID_SELF
    {
        TAB_PRESET_PROGRAM_WINDOW_EVENT_ROUTER.post_window_event(hwnd);
    }
}

unsafe extern "system" fn splitter_overlay_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_LBUTTONDOWN => {
            let owner = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND };
            if !owner.is_null() && unsafe { IsWindow(owner) } != 0 {
                unsafe {
                    SendMessageW(owner, WM_SPLITTER_OVERLAY_LBUTTONDOWN, 0, 0);
                }
            }
            0
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            if !hdc.is_null() {
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if unsafe { GetClientRect(hwnd, &mut rect) } != 0 {
                    fill(hdc, UiRect::from_rect(rect), COLOR_SPLITTER);
                }
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCDESTROY => {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if create.is_null() {
            return 0;
        }

        let state = unsafe { (*create).lpCreateParams as *mut MainWindow };
        if state.is_null() {
            return 0;
        }

        unsafe {
            (*state).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        return 1;
    }

    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MainWindow };
    if message == WM_NCDESTROY {
        if !state.is_null() {
            unsafe {
                let owned_by_window = (*state).owned_by_window;
                let active_message_handlers = (*state).active_message_handlers;
                (*state).hwnd = null_mut();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                // Before CreateWindowExW returns success, run() still owns the Box.
                if owned_by_window {
                    if active_message_handlers == 0 {
                        drop(Box::from_raw(state));
                    } else {
                        // A nested modal loop can destroy the HWND while an outer
                        // MainWindow method still has &mut self on the stack.
                        (*state).destroy_pending = true;
                    }
                }
            }
        }
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }

    if state.is_null() {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    } else {
        unsafe {
            (*state).active_message_handlers += 1;
        }
        let result = unsafe { (*state).handle_message(message, wparam, lparam) };
        let should_drop = unsafe {
            (*state).active_message_handlers -= 1;
            (*state).destroy_pending
                && (*state).active_message_handlers == 0
                && (*state).owned_by_window
        };
        if should_drop {
            unsafe {
                drop(Box::from_raw(state));
            }
        }
        result
    }
}

fn module_handle() -> Result<HINSTANCE, EntryError> {
    let handle = unsafe { GetModuleHandleW(null()) };
    if handle.is_null() {
        Err(EntryError::win32(
            "GetModuleHandleW",
            "프로세스 module handle을 가져올 수 없습니다.",
        ))
    } else {
        Ok(handle)
    }
}

fn register_window_class(hinstance: HINSTANCE) -> Result<(), EntryError> {
    let class_name = wide_null(CLASS_NAME);
    let class = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), 32512 as _) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };

    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        Err(EntryError::win32(
            "RegisterClassW",
            "j3GridDocker window class를 등록할 수 없습니다.",
        ))
    } else {
        Ok(())
    }
}

fn register_splitter_overlay_class(hinstance: HINSTANCE) -> Result<(), EntryError> {
    let class_name = wide_null(SPLITTER_OVERLAY_CLASS_NAME);
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(splitter_overlay_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), 32512 as _) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };

    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_CLASS_ALREADY_EXISTS {
            Ok(())
        } else {
            Err(EntryError::Win32 {
                api: "RegisterClassW",
                last_error,
                user_message: "splitter overlay window class를 등록할 수 없습니다.",
            })
        }
    } else {
        Ok(())
    }
}

fn create_splitter_overlay_window(owner: HWND) -> Result<HWND, Win32StatusFailure> {
    let hinstance = unsafe { GetModuleHandleW(null()) };
    if hinstance.is_null() {
        let last_error = unsafe { GetLastError() };
        return Err(Win32StatusFailure::new(
            "GetModuleHandleW",
            last_error,
            "splitter overlay window를 만들 수 없습니다.",
        ));
    }

    let class_name = wide_null(SPLITTER_OVERLAY_CLASS_NAME);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            owner,
            null_mut(),
            hinstance,
            owner as *const core::ffi::c_void,
        )
    };
    if hwnd.is_null() {
        let last_error = unsafe { GetLastError() };
        return Err(Win32StatusFailure::new(
            "CreateWindowExW",
            last_error,
            "splitter overlay window를 만들 수 없습니다.",
        ));
    }

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner as isize);
    }

    Ok(hwnd)
}

fn cleanup_text_input_dialog(hwnd: HWND, owner: HWND) {
    let dialog_alive = unsafe { IsWindow(hwnd) } != 0;
    cleanup_text_input_dialog_with(
        dialog_alive,
        || unsafe {
            DestroyWindow(hwnd);
        },
        || unsafe {
            EnableWindow(owner, 1);
        },
        || unsafe {
            SetFocus(owner);
        },
    );
}

fn cleanup_text_input_dialog_with(
    dialog_alive: bool,
    mut destroy_dialog: impl FnMut(),
    mut enable_owner: impl FnMut(),
    mut focus_owner: impl FnMut(),
) {
    if dialog_alive {
        destroy_dialog();
    }
    enable_owner();
    focus_owner();
}

fn show_about_dialog(owner: HWND, language: UiLanguage) -> Result<(), Win32StatusFailure> {
    let hinstance = unsafe { GetModuleHandleW(null()) };
    if hinstance.is_null() {
        return Err(Win32StatusFailure::new(
            "GetModuleHandleW",
            unsafe { GetLastError() },
            "About dialog를 열 수 없습니다.",
        ));
    }
    register_about_dialog_class(hinstance)?;
    let _ = init_about_link_control();

    let class_name = wide_null(ABOUT_DIALOG_CLASS_NAME);
    let title = wide_null(ui_text(language, "About j3GridDocker", "j3GridDocker 정보"));
    let (x, y) = centered_window_position(owner, ABOUT_DIALOG_WIDTH, ABOUT_DIALOG_HEIGHT);
    let mut state = Box::new(AboutDialogState::new(language));
    let state_ptr = state.as_mut() as *mut AboutDialogState;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            ABOUT_DIALOG_WIDTH,
            ABOUT_DIALOG_HEIGHT,
            owner,
            null_mut(),
            hinstance,
            state_ptr.cast(),
        )
    };
    if hwnd.is_null() {
        return Err(Win32StatusFailure::new(
            "CreateWindowExW",
            unsafe { GetLastError() },
            "About dialog를 열 수 없습니다.",
        ));
    }

    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    let mut message = unsafe { zeroed::<MSG>() };
    while !state.done {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result == -1 {
            cleanup_text_input_dialog(hwnd, owner);
            return Err(Win32StatusFailure::new(
                "GetMessageW",
                unsafe { GetLastError() },
                "About dialog 메시지를 가져올 수 없습니다.",
            ));
        }
        if result == 0 {
            unsafe {
                PostQuitMessage(message.wParam as i32);
            }
            state.done = true;
            break;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if unsafe { IsWindow(hwnd) } == 0 {
            state.done = true;
        }
    }

    cleanup_text_input_dialog(hwnd, owner);
    Ok(())
}

fn prompt_text_input(
    owner: HWND,
    title: &str,
    prompt: &str,
    initial_value: &str,
    ok_label: &str,
    cancel_label: &str,
) -> Result<Option<String>, EntryError> {
    let hinstance = module_handle()?;
    register_text_input_dialog_class(hinstance)?;

    let class_name = wide_null(TEXT_INPUT_DIALOG_CLASS_NAME);
    let title = wide_null(title);
    let (x, y) = centered_window_position(owner, TEXT_INPUT_DIALOG_WIDTH, TEXT_INPUT_DIALOG_HEIGHT);
    let mut state = Box::new(TextInputDialogState::new(
        prompt,
        initial_value,
        ok_label,
        cancel_label,
    ));
    let state_ptr = state.as_mut() as *mut TextInputDialogState;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            TEXT_INPUT_DIALOG_WIDTH,
            TEXT_INPUT_DIALOG_HEIGHT,
            owner,
            null_mut(),
            hinstance,
            state_ptr.cast(),
        )
    };

    if hwnd.is_null() {
        return Err(EntryError::win32(
            "CreateWindowExW",
            "탭 이름 입력 창을 열 수 없습니다.",
        ));
    }

    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    let mut message = unsafe { zeroed::<MSG>() };
    while !state.done {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result == -1 {
            cleanup_text_input_dialog(hwnd, owner);
            return Err(EntryError::win32(
                "GetMessageW",
                "탭 이름 입력 메시지를 가져올 수 없습니다.",
            ));
        }
        if result == 0 {
            unsafe {
                PostQuitMessage(message.wParam as i32);
            }
            state.done = true;
            break;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if unsafe { IsWindow(hwnd) } == 0 {
            state.done = true;
        }
    }

    cleanup_text_input_dialog(hwnd, owner);

    if let Some(last_error) = state.read_error {
        return Err(EntryError::Win32 {
            api: "GetWindowTextW",
            last_error,
            user_message: "입력한 탭 이름을 읽을 수 없습니다.",
        });
    }

    Ok(state.result.take())
}

fn register_about_dialog_class(hinstance: HINSTANCE) -> Result<(), Win32StatusFailure> {
    let class_name = wide_null(ABOUT_DIALOG_CLASS_NAME);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(about_dialog_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), 32512 as _) },
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };

    unsafe {
        SetLastError(0);
    }
    let atom = unsafe { RegisterClassW(&class) };
    if atom != 0 {
        return Ok(());
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_CLASS_ALREADY_EXISTS {
        Ok(())
    } else {
        Err(Win32StatusFailure::new(
            "RegisterClassW",
            last_error,
            "About dialog class를 등록할 수 없습니다.",
        ))
    }
}

fn init_about_link_control() -> bool {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LINK_CLASS,
    };
    (unsafe { InitCommonControlsEx(&controls) }) != 0
}

fn register_text_input_dialog_class(hinstance: HINSTANCE) -> Result<(), EntryError> {
    let class_name = wide_null(TEXT_INPUT_DIALOG_CLASS_NAME);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(text_input_dialog_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), 32512 as _) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };

    unsafe {
        SetLastError(0);
    }
    let atom = unsafe { RegisterClassW(&class) };
    if atom != 0 {
        return Ok(());
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_CLASS_ALREADY_EXISTS {
        Ok(())
    } else {
        Err(EntryError::Win32 {
            api: "RegisterClassW",
            last_error,
            user_message: "탭 이름 입력 창 class를 등록할 수 없습니다.",
        })
    }
}

fn set_dialog_font(hwnd: HWND) {
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    if font.is_null() {
        return;
    }
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    }
}

fn centered_window_position(owner: HWND, width: i32, height: i32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { GetWindowRect(owner, &mut rect) };
    if ok == 0 {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }

    let owner_width = rect.right.saturating_sub(rect.left);
    let owner_height = rect.bottom.saturating_sub(rect.top);
    let x = rect.left + (owner_width.saturating_sub(width)) / 2;
    let y = rect.top + (owner_height.saturating_sub(height)) / 2;
    (x, y)
}

fn message_loop() -> Result<(), EntryError> {
    let mut message = unsafe { zeroed::<MSG>() };

    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result == -1 {
            return Err(EntryError::win32(
                "GetMessageW",
                "Win32 message를 가져올 수 없습니다.",
            ));
        }
        if result == 0 {
            return Ok(());
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn create_menu_handle() -> Option<HMENU> {
    let menu = unsafe { CreateMenu() };
    if menu.is_null() { None } else { Some(menu) }
}

fn append_menu(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    command: u16,
    text: &str,
) -> bool {
    append_menu_enabled(menu, command, text, true)
}

fn append_menu_enabled(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    command: u16,
    text: &str,
    enabled: bool,
) -> bool {
    let text = wide_null(text);
    let state = if enabled { 0 } else { MF_GRAYED };
    unsafe { AppendMenuW(menu, MF_STRING | state, usize::from(command), text.as_ptr()) != 0 }
}

fn append_disabled_menu(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    text: &str,
) -> bool {
    let text = wide_null(text);
    unsafe { AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, text.as_ptr()) != 0 }
}

fn append_separator(menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU) -> bool {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) != 0 }
}

fn append_checked_menu(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    command: u16,
    text: &str,
    checked: bool,
) -> bool {
    let text = wide_null(text);
    let checked_flag = if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        AppendMenuW(
            menu,
            MF_STRING | checked_flag,
            usize::from(command),
            text.as_ptr(),
        ) != 0
    }
}

fn append_submenu(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    submenu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    text: &str,
) -> bool {
    let text = wide_null(text);
    unsafe { AppendMenuW(menu, MF_POPUP, submenu as usize, text.as_ptr()) != 0 }
}

fn load_icon(size: i32) -> Option<HICON> {
    load_resource_icon(size).or_else(|| load_file_icon(size))
}

fn load_resource_icon(size: i32) -> Option<HICON> {
    let hinstance = module_handle().ok()?;
    let handle = unsafe {
        LoadImageW(
            hinstance,
            make_int_resource(APP_ICON_RESOURCE_ID),
            IMAGE_ICON,
            size,
            size,
            0,
        )
    };

    if handle.is_null() {
        None
    } else {
        Some(handle as HICON)
    }
}

fn load_file_icon(size: i32) -> Option<HICON> {
    let path = Path::new("icon.ico");
    if !path.exists() {
        return None;
    }

    let wide_path = wide_os_null(path.as_os_str());
    let handle = unsafe {
        LoadImageW(
            null_mut(),
            wide_path.as_ptr(),
            IMAGE_ICON,
            size,
            size,
            LR_LOADFROMFILE,
        )
    };

    if handle.is_null() {
        None
    } else {
        Some(handle as HICON)
    }
}

fn make_int_resource(id: usize) -> *const u16 {
    id as *const u16
}

fn cursor_position() -> Option<ScreenPoint> {
    let mut point = POINT { x: 0, y: 0 };
    let ok = unsafe { GetCursorPos(&mut point) };
    if ok == 0 {
        None
    } else {
        Some(ScreenPoint {
            x: point.x,
            y: point.y,
        })
    }
}

fn point_from_lparam(lparam: LPARAM) -> ClientPoint {
    let raw = lparam as u32;
    let x = (raw & 0xFFFF) as i16 as i32;
    let y = ((raw >> 16) & 0xFFFF) as i16 as i32;
    ClientPoint { x, y }
}

fn low_word(value: WPARAM) -> u16 {
    (value & 0xFFFF) as u16
}

const WIDE_REPLACEMENT_CHARACTER: u16 = 0xFFFD;

fn wide_null(value: &str) -> Vec<u16> {
    let mut buffer = Vec::with_capacity(value.encode_utf16().count().saturating_add(1));
    replace_wide_null(&mut buffer, value);
    buffer
}

fn replace_wide_null(buffer: &mut Vec<u16>, value: &str) {
    buffer.clear();
    buffer.extend(value.encode_utf16().map(|unit| {
        if unit == 0 {
            WIDE_REPLACEMENT_CHARACTER
        } else {
            unit
        }
    }));
    buffer.push(0);
}

fn wide_os_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestPaintTab {
        tab_id: TabId,
        name: String,
    }

    impl TestPaintTab {
        fn new(tab_id: u64, name: impl Into<String>) -> Self {
            Self {
                tab_id: TabId::new(tab_id),
                name: name.into(),
            }
        }
    }

    fn test_hwnd(value: isize) -> HWND {
        value as HWND
    }

    fn assert_rect_eq(actual: RECT, expected: RECT) {
        assert_eq!(actual.left, expected.left);
        assert_eq!(actual.top, expected.top);
        assert_eq!(actual.right, expected.right);
        assert_eq!(actual.bottom, expected.bottom);
    }

    fn test_tab_index(tabs: &[TestPaintTab], tab_id: TabId) -> usize {
        for (index, tab) in tabs.iter().enumerate() {
            if tab.tab_id == tab_id {
                return index;
            }
        }

        panic!("expected test tab to exist");
    }

    fn overflow_tab_label_layout(
        tab_count: usize,
        active_index: usize,
        first_visible_index: usize,
    ) -> TabStripLayout {
        for width in (TAB_BAR_LEFT + 1)..DEFAULT_WIDTH {
            let layout = tab_strip_layout(
                UiRect::new(0, 0, width, TAB_BAR_HEIGHT),
                tab_count,
                Some(active_index),
                first_visible_index,
            );

            if layout.first_visible_index > 0
                && layout.visible_end_index() < tab_count
                && layout
                    .visible_end_index()
                    .saturating_sub(layout.first_visible_index)
                    >= 2
            {
                return layout;
            }
        }

        panic!("expected overflow tab label layout");
    }

    fn sync_test_visible_tab_labels(
        cache: &mut Vec<CachedTabLabel>,
        tabs: &[TestPaintTab],
        layout: TabStripLayout,
    ) {
        let (first_visible_index, visible_end_index) = visible_tab_label_bounds(layout, tabs.len());

        sync_paint_tab_labels(
            cache,
            tabs[first_visible_index..visible_end_index]
                .iter()
                .map(|tab| (tab.tab_id, tab.name.as_str())),
        );
    }

    fn assert_cache_matches_visible_tabs(
        cache: &[CachedTabLabel],
        tabs: &[TestPaintTab],
        layout: TabStripLayout,
    ) {
        let (first_visible_index, visible_end_index) = visible_tab_label_bounds(layout, tabs.len());
        let actual = cache
            .iter()
            .map(|label| (label.tab_id(), label.label.text.as_str()))
            .collect::<Vec<_>>();
        let expected = tabs[first_visible_index..visible_end_index]
            .iter()
            .map(|tab| (tab.tab_id, tab.name.as_str()))
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert_eq!(cache.len(), visible_end_index - first_visible_index);
    }

    #[test]
    fn wide_text_replace_str_updates_nul_terminated_buffer() {
        let mut text = WideText::new("Ready");

        assert_eq!(
            text.wide(),
            &[
                b'R' as u16,
                b'e' as u16,
                b'a' as u16,
                b'd' as u16,
                b'y' as u16,
                0
            ]
        );

        text.replace_str("Go");

        assert_eq!(text.wide(), &[b'G' as u16, b'o' as u16, 0]);
    }

    #[test]
    fn wide_text_helpers_replace_internal_nul_before_terminator() {
        let wide = wide_null("A\0B");

        assert_eq!(
            wide,
            vec![b'A' as u16, WIDE_REPLACEMENT_CHARACTER, b'B' as u16, 0]
        );

        let mut buffer = Vec::new();
        replace_wide_null(&mut buffer, "C\0D");

        assert_eq!(
            buffer,
            vec![b'C' as u16, WIDE_REPLACEMENT_CHARACTER, b'D' as u16, 0]
        );
    }

    #[test]
    fn pending_process_window_search_waits_between_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10, 20], start);

        assert!(search.should_scan(start, deadline));
        search.mark_scan_completed(start, false);
        assert!(!search.should_scan(start + Duration::from_millis(100), deadline));
        assert!(search.should_scan(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
    }

    #[test]
    fn pending_process_window_search_can_defer_initial_scan() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let search = PendingProcessWindowSearch::from_process_ids_with_scan_delay(
            [10, 20],
            start,
            TAB_PRESET_WINDOW_SCAN_INTERVAL,
        );

        assert!(!search.should_scan(start, deadline));
        assert!(!search.should_scan(start + TAB_PRESET_WINDOW_POLL, deadline));
        assert!(search.should_scan(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
    }

    #[test]
    fn pending_process_window_search_hooked_fallback_uses_slower_window_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids_with_scan_policy(
            [10, 20],
            start,
            PendingProcessWindowScanPolicy::hooked_fallback(),
        );

        assert!(!search.should_scan(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
        assert!(search.should_scan(start + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL, deadline));
        search.mark_scan_completed(start + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL, false);

        assert!(!search.should_scan(
            start + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL + TAB_PRESET_WINDOW_SCAN_INTERVAL,
            deadline
        ));
        assert!(search.should_scan(
            start + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL,
            deadline
        ));
    }

    #[test]
    fn pending_process_window_search_keeps_cached_match_between_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let hwnd = test_hwnd(100);

        search.hwnds.insert(10, hwnd);
        search.mark_scan_completed(start, false);
        search.refresh(start + Duration::from_millis(100), deadline);

        assert!(search.has_matches());
        assert_eq!(search.hwnd_for(10), Some(hwnd));
    }

    #[test]
    fn pending_process_window_search_backs_off_empty_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10, 20], start);
        let second_scan = start + TAB_PRESET_WINDOW_SCAN_INTERVAL;
        let third_scan =
            second_scan + TAB_PRESET_WINDOW_SCAN_INTERVAL + TAB_PRESET_WINDOW_SCAN_INTERVAL;

        search.mark_scan_completed(start, false);
        search.mark_scan_completed(second_scan, false);

        assert!(!search.should_scan(third_scan - Duration::from_millis(100), deadline));
        assert!(search.should_scan(third_scan, deadline));
    }

    #[test]
    fn pending_process_window_search_extends_empty_scan_backoff() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10, 20], start);
        let second_scan = start + TAB_PRESET_WINDOW_SCAN_INTERVAL;
        let third_scan =
            second_scan + TAB_PRESET_WINDOW_SCAN_INTERVAL + TAB_PRESET_WINDOW_SCAN_INTERVAL;
        let fourth_scan = third_scan + TAB_PRESET_WINDOW_MAX_SCAN_INTERVAL;

        search.mark_scan_completed(start, false);
        search.mark_scan_completed(second_scan, false);
        search.mark_scan_completed(third_scan, false);

        assert!(!search.should_scan(fourth_scan - TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
        assert!(search.should_scan(fourth_scan, deadline));
    }

    #[test]
    fn pending_process_window_search_caps_empty_scan_backoff() {
        let start = Instant::now();
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);

        search.mark_scan_completed(start, false);
        search.mark_scan_completed(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, false);
        search.mark_scan_completed(start + TAB_PRESET_WINDOW_SCAN_INTERVAL * 3, false);
        search.mark_scan_completed(start + TAB_PRESET_WINDOW_SCAN_INTERVAL * 7, false);

        assert_eq!(search.scan_interval, TAB_PRESET_WINDOW_MAX_SCAN_INTERVAL);
    }

    #[test]
    fn pending_process_window_search_resets_scan_backoff_after_window_match() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10, 20], start);
        let matched_at = start + TAB_PRESET_WINDOW_SCAN_INTERVAL;

        search.mark_scan_completed(start, false);
        search.mark_scan_completed(matched_at, true);

        assert_eq!(search.scan_interval, TAB_PRESET_WINDOW_SCAN_INTERVAL);
        assert!(!search.should_scan(matched_at + Duration::from_millis(100), deadline));
        assert!(search.should_scan(matched_at + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
    }

    #[test]
    fn pending_process_window_search_allows_deadline_scan() {
        let start = Instant::now();
        let search = PendingProcessWindowSearch::from_process_ids_with_scan_delay(
            [10],
            start,
            TAB_PRESET_WINDOW_SCAN_INTERVAL,
        );
        let before_next_scan = start + Duration::from_millis(100);

        assert!(search.should_scan(before_next_scan, before_next_scan));
    }

    #[test]
    fn pending_process_window_search_skips_recent_deadline_scan() {
        let start = Instant::now();
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        search.mark_scan_completed(start, false);
        let before_next_scan = start + Duration::from_millis(100);

        assert!(!search.should_scan(before_next_scan, before_next_scan));
    }

    #[test]
    fn pending_process_window_search_delays_thread_snapshots_until_interval() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let first_thread_snapshot = start + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;

        assert!(!search.should_scan_threads(start, deadline));
        assert!(!search.should_scan_threads(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
        assert!(search.should_scan_threads(first_thread_snapshot, deadline));
        search.mark_thread_scan_completed(first_thread_snapshot, false);

        assert!(!search.should_scan_threads(
            first_thread_snapshot + TAB_PRESET_WINDOW_SCAN_INTERVAL,
            deadline
        ));
        assert!(search.should_scan_threads(
            first_thread_snapshot + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            deadline
        ));
    }

    #[test]
    fn pending_process_window_search_hooked_fallback_delays_thread_snapshots() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let search = PendingProcessWindowSearch::from_process_ids_with_scan_policy(
            [10],
            start,
            PendingProcessWindowScanPolicy::hooked_fallback(),
        );

        assert!(!search.should_scan_threads(start + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL, deadline));
        assert!(
            !search.should_scan_threads(start + TAB_PRESET_HOOKED_WINDOW_SCAN_INTERVAL, deadline)
        );
        assert!(
            search.should_scan_threads(start + TAB_PRESET_HOOKED_THREAD_SNAPSHOT_DELAY, deadline)
        );
    }

    #[test]
    fn pending_process_window_search_allows_deadline_thread_snapshot() {
        let start = Instant::now();
        let search = PendingProcessWindowSearch::from_process_ids([10], start);
        let before_thread_snapshot = start + TAB_PRESET_WINDOW_SCAN_INTERVAL;

        assert!(search.should_scan_threads(before_thread_snapshot, before_thread_snapshot));
    }

    #[test]
    fn pending_process_window_search_skips_recent_deadline_thread_snapshot() {
        let start = Instant::now();
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        search.mark_thread_scan_completed(start, false);
        let before_thread_snapshot = start + TAB_PRESET_WINDOW_POLL;

        assert!(!search.should_scan_threads(before_thread_snapshot, before_thread_snapshot));
    }

    #[test]
    fn pending_process_window_search_removes_process_without_shrinking_buffers() {
        let mut search = PendingProcessWindowSearch::from_process_ids([10, 20], Instant::now());
        let process_capacity = search.process_ids.capacity();
        let hwnd_capacity = search.hwnds.capacity();
        let remaining_capacity = search.remaining_process_ids.capacity();
        search.hwnds.insert(10, test_hwnd(100));
        search.remaining_process_ids.insert(10);

        search.remove_process(10);

        assert!(!search.process_ids.contains(&10));
        assert!(search.process_ids.contains(&20));
        assert!(!search.hwnds.contains_key(&10));
        assert!(!search.remaining_process_ids.contains(&10));
        assert!(search.process_ids.capacity() >= process_capacity);
        assert!(search.hwnds.capacity() >= hwnd_capacity);
        assert!(search.remaining_process_ids.capacity() >= remaining_capacity);
    }

    #[test]
    fn pending_process_window_search_tracks_added_processes() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);

        search.mark_scan_completed(start, false);
        search.add_process_ids([0, 10, 20]);

        assert!(search.process_ids.contains(&10));
        assert!(search.process_ids.contains(&20));
        assert!(search.remaining_process_ids.contains(&20));
        assert!(search.should_scan(start + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline));
    }

    #[test]
    fn pending_process_window_search_excludes_event_match_from_future_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let hwnd = test_hwnd(100);

        search.record_process_window(10, hwnd);

        assert!(search.has_matches());
        assert_eq!(search.hwnd_for(10), Some(hwnd));
        assert!(!search.remaining_process_ids.contains(&10));
        assert!(!search.needs_window_for_process(10));
        assert!(!search.should_scan(start, deadline));
        assert!(!search.should_scan(deadline, deadline));
    }

    #[test]
    fn pending_process_window_search_skips_thread_snapshot_without_remaining_processes() {
        let start = Instant::now();
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let hwnd = test_hwnd(100);
        let thread_snapshot_at = start + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;

        search.record_process_window(10, hwnd);

        assert!(!search.should_scan_threads(thread_snapshot_at, thread_snapshot_at));
    }

    #[test]
    fn pending_process_window_search_backs_off_empty_thread_snapshots() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let first_snapshot = start + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;
        let second_snapshot = first_snapshot + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;

        search.mark_thread_scan_completed(first_snapshot, false);
        search.mark_thread_scan_completed(second_snapshot, false);

        assert_eq!(
            search.thread_snapshot_interval,
            TAB_PRESET_THREAD_SNAPSHOT_MAX_INTERVAL
        );
        assert!(!search.should_scan_threads(
            second_snapshot + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            deadline
        ));
        assert!(search.should_scan_threads(
            second_snapshot + TAB_PRESET_THREAD_SNAPSHOT_MAX_INTERVAL,
            deadline
        ));
    }

    #[test]
    fn pending_process_window_search_resets_thread_snapshot_backoff_after_match() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut search = PendingProcessWindowSearch::from_process_ids([10], start);
        let first_snapshot = start + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;
        let second_snapshot = first_snapshot + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL;

        search.mark_thread_scan_completed(first_snapshot, false);
        search.mark_thread_scan_completed(second_snapshot, true);

        assert_eq!(
            search.thread_snapshot_interval,
            TAB_PRESET_THREAD_SNAPSHOT_INTERVAL
        );
        assert!(
            !search
                .should_scan_threads(second_snapshot + TAB_PRESET_WINDOW_SCAN_INTERVAL, deadline)
        );
        assert!(search.should_scan_threads(
            second_snapshot + TAB_PRESET_THREAD_SNAPSHOT_INTERVAL,
            deadline
        ));
    }

    #[test]
    fn process_tree_scan_schedule_waits_between_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut schedule = ProcessTreeScanSchedule::new(start);
        let first_scan = start + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;

        assert!(!schedule.should_scan(start, deadline));
        assert!(!schedule.should_scan(start + TAB_PRESET_WINDOW_POLL, deadline));
        assert!(schedule.should_scan(first_scan, deadline));
        schedule.mark_completed(first_scan, false);

        assert!(!schedule.should_scan(first_scan + TAB_PRESET_WINDOW_POLL, deadline));
        assert!(schedule.should_scan(first_scan + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL, deadline));
    }

    #[test]
    fn process_tree_scan_schedule_backs_off_empty_scans() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut schedule = ProcessTreeScanSchedule::new(start);
        let first_scan = start + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let second_scan = first_scan + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let third_scan = second_scan
            + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL
            + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;

        schedule.mark_completed(first_scan, false);
        schedule.mark_completed(second_scan, false);

        assert!(!schedule.should_scan(third_scan - TAB_PRESET_WINDOW_POLL, deadline));
        assert!(schedule.should_scan(third_scan, deadline));
    }

    #[test]
    fn process_tree_scan_schedule_extends_empty_scan_backoff() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut schedule = ProcessTreeScanSchedule::new(start);
        let first_scan = start + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let second_scan = first_scan + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let third_scan = second_scan
            + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL
            + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let fourth_scan = third_scan + TAB_PRESET_PROCESS_TREE_MAX_SCAN_INTERVAL;

        schedule.mark_completed(first_scan, false);
        schedule.mark_completed(second_scan, false);
        schedule.mark_completed(third_scan, false);

        assert!(!schedule.should_scan(
            fourth_scan - TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL,
            deadline
        ));
        assert!(schedule.should_scan(fourth_scan, deadline));
    }

    #[test]
    fn process_tree_scan_schedule_resets_backoff_after_discovery() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut schedule = ProcessTreeScanSchedule::new(start);
        let first_scan = start + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;
        let discovered_at = first_scan + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL;

        schedule.mark_completed(first_scan, false);
        schedule.mark_completed(discovered_at, true);

        assert_eq!(
            schedule.scan_interval,
            TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL
        );
        assert!(!schedule.should_scan(discovered_at + TAB_PRESET_WINDOW_POLL, deadline));
        assert!(schedule.should_scan(
            discovered_at + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL,
            deadline
        ));
    }

    #[test]
    fn process_tree_scan_schedule_hooked_fallback_is_less_frequent() {
        let start = Instant::now();
        let deadline = start + TAB_PRESET_WINDOW_WAIT;
        let mut schedule = ProcessTreeScanSchedule::hooked_fallback(start);
        let first_scan = start + TAB_PRESET_HOOKED_PROCESS_TREE_SCAN_INTERVAL;
        let second_scan = first_scan + TAB_PRESET_HOOKED_PROCESS_TREE_SCAN_INTERVAL;
        let third_scan = second_scan + TAB_PRESET_HOOKED_PROCESS_TREE_MAX_SCAN_INTERVAL;

        assert!(!schedule.should_scan(start + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL, deadline));
        assert!(schedule.should_scan(first_scan, deadline));
        schedule.mark_completed(first_scan, false);

        assert!(
            !schedule.should_scan(first_scan + TAB_PRESET_PROCESS_TREE_SCAN_INTERVAL, deadline)
        );
        assert!(schedule.should_scan(second_scan, deadline));
        schedule.mark_completed(second_scan, false);

        assert!(!schedule.should_scan(third_scan - TAB_PRESET_WINDOW_POLL, deadline));
        assert!(schedule.should_scan(third_scan, deadline));
    }

    #[test]
    fn process_tree_scan_schedule_allows_deadline_scan() {
        let start = Instant::now();
        let schedule = ProcessTreeScanSchedule::new(start);
        let before_next_scan = start + TAB_PRESET_WINDOW_POLL;

        assert!(schedule.should_scan(before_next_scan, before_next_scan));
    }

    #[test]
    fn process_tree_scan_schedule_skips_recent_deadline_scan() {
        let start = Instant::now();
        let mut schedule = ProcessTreeScanSchedule::new(start);
        schedule.mark_completed(start, false);
        let before_next_scan = start + TAB_PRESET_WINDOW_POLL;

        assert!(!schedule.should_scan(before_next_scan, before_next_scan));
    }

    #[test]
    fn process_tree_descendant_ids_expand_recursively_from_known_roots() {
        let known = HashSet::from([10]);
        let mut descendants = descendant_process_ids_from_entries(
            &known,
            &[
                ProcessTreeEntry {
                    process_id: 30,
                    parent_process_id: 20,
                },
                ProcessTreeEntry {
                    process_id: 40,
                    parent_process_id: 99,
                },
                ProcessTreeEntry {
                    process_id: 20,
                    parent_process_id: 10,
                },
            ],
        );

        descendants.sort_unstable();

        assert_eq!(descendants, vec![20, 30]);
    }

    #[test]
    fn process_tree_child_index_appends_descendants_to_tracked_processes() {
        let index = ProcessTreeChildIndex::from_entries([
            ProcessTreeEntry {
                process_id: 20,
                parent_process_id: 10,
            },
            ProcessTreeEntry {
                process_id: 30,
                parent_process_id: 20,
            },
            ProcessTreeEntry {
                process_id: 60,
                parent_process_id: 50,
            },
        ]);
        let mut tracked = HashSet::from([10, 50]);
        let mut descendants = Vec::new();
        let mut stack = Vec::new();

        index.append_new_descendants(&mut tracked, &mut descendants, &mut stack);
        descendants.sort_unstable();

        assert_eq!(descendants, vec![20, 30, 60]);
        assert_eq!(tracked, HashSet::from([10, 20, 30, 50, 60]));
    }

    #[test]
    fn process_tree_child_index_skips_already_tracked_descendants() {
        let index = ProcessTreeChildIndex::from_entries([
            ProcessTreeEntry {
                process_id: 20,
                parent_process_id: 10,
            },
            ProcessTreeEntry {
                process_id: 30,
                parent_process_id: 20,
            },
        ]);
        let mut tracked = HashSet::from([10, 20]);
        let mut descendants = Vec::new();
        let mut stack = Vec::new();

        index.append_new_descendants(&mut tracked, &mut descendants, &mut stack);

        assert_eq!(descendants, vec![30]);
        assert_eq!(tracked, HashSet::from([10, 20, 30]));
    }

    #[test]
    fn process_tree_child_index_prunes_missing_processes_after_descendant_expansion() {
        let index = ProcessTreeChildIndex::from_entries([
            ProcessTreeEntry {
                process_id: 10,
                parent_process_id: 1,
            },
            ProcessTreeEntry {
                process_id: 30,
                parent_process_id: 20,
            },
        ]);
        let mut tracked = HashSet::from([10, 20]);
        let mut descendants = Vec::new();
        let mut removed = Vec::new();
        let mut stack = Vec::new();

        index.append_new_descendants(&mut tracked, &mut descendants, &mut stack);
        index.remove_missing_processes(&mut tracked, |_| false, &mut removed);
        descendants.sort_unstable();
        removed.sort_unstable();

        assert_eq!(descendants, vec![30]);
        assert_eq!(removed, vec![20]);
        assert_eq!(tracked, HashSet::from([10, 30]));
    }

    #[test]
    fn process_tree_child_index_keeps_missing_processes_with_cached_window_match() {
        let index = ProcessTreeChildIndex::from_entries([ProcessTreeEntry {
            process_id: 10,
            parent_process_id: 1,
        }]);
        let mut tracked = HashSet::from([10, 20]);
        let keep = HashSet::from([20]);
        let mut removed = Vec::new();

        index.remove_missing_processes(
            &mut tracked,
            |process_id| keep.contains(&process_id),
            &mut removed,
        );

        assert!(removed.is_empty());
        assert_eq!(tracked, HashSet::from([10, 20]));
    }

    #[test]
    fn thread_entry32_initializes_toolhelp_size() {
        let entry = ThreadEntry32::new();

        assert_eq!(entry.size as usize, std::mem::size_of::<ThreadEntry32>());
    }

    #[test]
    fn process_entry32_initializes_toolhelp_size() {
        let entry = ProcessEntry32::new();

        assert_eq!(entry.size as usize, std::mem::size_of::<ProcessEntry32>());
    }

    #[test]
    fn tab_tooltip_sync_key_spec_matches_same_window_sequence()
    -> Result<(), Box<dyn std::error::Error>> {
        let tab_id = TabId::new(7);
        let rect = UiRect::new(20, 0, 140, TAB_BAR_HEIGHT);
        let first = WindowHandle::new(100)?;
        let second = WindowHandle::new(200)?;
        let spec = TabTooltipSyncKeySpec::new(
            tab_id,
            rect,
            vec![first.raw(), second.raw()],
            vec![Some("First".to_owned()), Some("Second".to_owned())],
        );
        let layout = TabTooltipSyncLayoutKey::new(
            UiLanguage::English,
            0,
            UiRect::new(0, 0, 320, TAB_BAR_HEIGHT),
            1,
            0,
            1,
        );
        let key = TabTooltipSyncKey {
            layout,
            tabs: vec![spec.clone()],
        };
        let renamed_title = "Renamed".to_owned();
        let mut renamed_spec = spec.clone();

        assert!(key.matches_layout(layout));
        assert!(!key.matches_layout(TabTooltipSyncLayoutKey::new(
            UiLanguage::English,
            1,
            UiRect::new(0, 0, 320, TAB_BAR_HEIGHT),
            1,
            0,
            1,
        )));
        assert!(key.contains_window(first.raw()));
        assert!(!key.contains_window(300));
        assert_eq!(
            key.tabs_for_window(second.raw()).collect::<Vec<_>>(),
            vec![(tab_id, rect)]
        );
        assert_eq!(key.tabs_for_window(300).next(), None);
        assert!(spec.matches_windows(tab_id, rect, [first, second]));
        assert!(!spec.matches_windows(tab_id, rect, [first]));
        assert!(!spec.matches_windows(tab_id, rect, [second, first]));
        assert!(!spec.matches_windows(TabId::new(8), rect, [first, second]));
        assert!(!spec.matches_windows(
            tab_id,
            UiRect::new(21, 0, 141, TAB_BAR_HEIGHT),
            [first, second]
        ));
        assert!(renamed_spec.update_window_title(second.raw(), Some(renamed_title.as_str())));
        assert_eq!(
            renamed_spec.tooltip_text(),
            tab_tooltip_text_from_titles(["First".to_owned(), "Renamed".to_owned()].into_iter())
        );
        assert!(renamed_spec.update_window_title(first.raw(), None));
        assert_eq!(
            renamed_spec.tooltip_text(),
            tab_tooltip_text_from_titles(["Renamed".to_owned()].into_iter())
        );

        Ok(())
    }

    #[test]
    fn tab_tooltip_sync_key_updates_only_tabs_containing_renamed_window()
    -> Result<(), Box<dyn std::error::Error>> {
        let first_tab_id = TabId::new(7);
        let second_tab_id = TabId::new(8);
        let first_rect = UiRect::new(20, 0, 140, TAB_BAR_HEIGHT);
        let second_rect = UiRect::new(140, 0, 260, TAB_BAR_HEIGHT);
        let first = WindowHandle::new(100)?;
        let renamed = WindowHandle::new(200)?;
        let unrelated = WindowHandle::new(300)?;
        let layout = TabTooltipSyncLayoutKey::new(
            UiLanguage::English,
            0,
            UiRect::new(0, 0, 320, TAB_BAR_HEIGHT),
            2,
            0,
            2,
        );
        let mut key = TabTooltipSyncKey {
            layout,
            tabs: vec![
                TabTooltipSyncKeySpec::new(
                    first_tab_id,
                    first_rect,
                    vec![first.raw(), renamed.raw()],
                    vec![Some("First".to_owned()), Some("Before".to_owned())],
                ),
                TabTooltipSyncKeySpec::new(
                    second_tab_id,
                    second_rect,
                    vec![unrelated.raw()],
                    vec![Some("Unrelated".to_owned())],
                ),
            ],
        };

        let updates =
            key.tooltip_sync_specs_for_window_title_change(renamed.raw(), Some("After".to_owned()));

        assert_eq!(updates.len(), 1);
        let (updated_tab_id, updated_spec) = &updates[0];
        assert_eq!(*updated_tab_id, first_tab_id);
        let expected_text = tab_tooltip_text_from_titles(["First".to_owned(), "After".to_owned()]);
        assert_eq!(
            updated_spec
                .as_ref()
                .map(|spec| (spec.tab_id, spec.rect, &spec.text)),
            expected_text
                .as_ref()
                .map(|text| (first_tab_id, first_rect, text))
        );

        let updates = key.tooltip_sync_specs_for_window_title_change(renamed.raw(), None);

        assert_eq!(updates.len(), 1);
        let (updated_tab_id, updated_spec) = &updates[0];
        assert_eq!(*updated_tab_id, first_tab_id);
        let expected_text = tab_tooltip_text_from_titles(["First".to_owned()]);
        assert_eq!(
            updated_spec
                .as_ref()
                .map(|spec| (spec.tab_id, spec.rect, &spec.text)),
            expected_text
                .as_ref()
                .map(|text| (first_tab_id, first_rect, text))
        );

        Ok(())
    }

    #[test]
    fn window_name_change_router_filters_uninterested_windows() {
        let target = test_hwnd(10);
        let first = test_hwnd(100);
        let second = test_hwnd(200);
        let router = WindowNameChangeEventRouter {
            target_hwnd: AtomicIsize::new(target as isize),
            interested_hwnd_filter: AtomicUsize::new(0),
            interested_hwnds: Mutex::new(Vec::new()),
        };

        router.replace_interested_hwnds([first as isize, second as isize, first as isize, 0]);

        assert_eq!(router.should_post_name_change_event(first), Some(target));
        assert_eq!(router.should_post_name_change_event(second), Some(target));
        assert_eq!(router.should_post_name_change_event(test_hwnd(300)), None);
        assert_eq!(router.should_post_name_change_event(test_hwnd(0)), None);

        router.clear_interested_hwnds();

        assert_eq!(router.should_post_name_change_event(first), None);
    }

    #[test]
    fn tab_preset_program_window_event_router_requires_target_and_window() {
        let target = test_hwnd(10);
        let window = test_hwnd(100);
        let router = TabPresetProgramWindowEventRouter {
            target_hwnd: AtomicIsize::new(target as isize),
        };

        assert_eq!(router.target_for_window_event(window), Some(target));
        assert_eq!(router.target_for_window_event(test_hwnd(0)), None);

        router.clear_target(target);

        assert_eq!(router.target_for_window_event(window), None);
    }

    #[test]
    fn dirty_paint_sections_status_only_skips_top_buttons_and_regions() {
        let client = UiRect::new(0, 0, 800, 600);
        let dirty = UiRect::new(0, 600 - STATUS_BAR_HEIGHT, 800, 600);

        let sections = DirtyPaintSections::for_dirty(client, dirty, true);

        assert!(!sections.top_bar);
        assert!(!sections.tab_strip);
        assert!(!sections.command_buttons);
        assert!(!sections.workspace_regions);
        assert!(sections.status_bar);
    }

    #[test]
    fn dirty_paint_sections_workspace_body_skips_chrome_and_status() {
        let client = UiRect::new(0, 0, 800, 600);
        let dirty = UiRect::new(
            12,
            top_bar_height(true) + 8,
            240,
            600 - STATUS_BAR_HEIGHT - 8,
        );

        let sections = DirtyPaintSections::for_dirty(client, dirty, true);

        assert!(!sections.top_bar);
        assert!(!sections.tab_strip);
        assert!(!sections.command_buttons);
        assert!(sections.workspace_regions);
        assert!(!sections.status_bar);
    }

    #[test]
    fn dirty_paint_sections_command_buttons_skip_tab_strip_and_regions() {
        let client = UiRect::new(0, 0, 800, 600);
        let dirty = UiRect::new(12, TAB_BAR_HEIGHT + 6, 80, TAB_BAR_HEIGHT + 16);

        let sections = DirtyPaintSections::for_dirty(client, dirty, true);

        assert!(sections.top_bar);
        assert!(!sections.tab_strip);
        assert!(sections.command_buttons);
        assert!(!sections.workspace_regions);
        assert!(!sections.status_bar);
    }

    #[test]
    fn command_buttons_fit_inside_compact_client_with_margin() {
        let client = UiRect::new(0, 0, 496, 600);
        let mut buttons = Vec::new();

        visit_command_button_rects(client, |_, button| {
            buttons.push(button);
            true
        });

        assert_eq!(buttons.len(), BUTTON_SPECS.len());
        assert!(buttons.iter().all(|button| {
            button.rect.left >= client.left
                && button.rect.right <= client.right - COMMAND_BUTTON_RIGHT_MARGIN
        }));
        assert_eq!(
            buttons.last().map(|button| button.command),
            Some(CMD_UNDOCK)
        );
    }

    #[test]
    fn command_buttons_do_not_expose_partially_clipped_button() {
        let client = UiRect::new(0, 0, 460, 600);
        let mut buttons = Vec::new();

        visit_command_button_rects(client, |_, button| {
            buttons.push(button);
            true
        });

        assert_eq!(buttons.len(), BUTTON_SPECS.len() - 1);
        assert!(!buttons.iter().any(|button| button.command == CMD_UNDOCK));
        assert!(buttons.iter().all(|button| {
            button.rect.left >= client.left
                && button.rect.right <= client.right - COMMAND_BUTTON_RIGHT_MARGIN
        }));
        assert_eq!(
            command_buttons_rect_for_client(client).map(|rect| rect.right),
            buttons.last().map(|button| button.rect.right)
        );
    }

    #[test]
    fn get_window_text_zero_return_uses_last_error_to_distinguish_empty_text() {
        assert_eq!(get_window_text_zero_return_error(0, 0), None);
        assert_eq!(get_window_text_zero_return_error(0, 5), Some(5));
        assert_eq!(get_window_text_zero_return_error(3, 0), Some(0));
    }

    #[test]
    fn window_text_length_and_capacity_allows_configured_limit() {
        assert_eq!(
            window_text_length_and_capacity(MAX_WINDOW_TEXT_CHARS as i32, 0),
            Ok((MAX_WINDOW_TEXT_CHARS, MAX_WINDOW_TEXT_CHARS + 1))
        );
    }

    #[test]
    fn window_text_length_and_capacity_rejects_oversized_length() {
        assert_eq!(
            window_text_length_and_capacity(MAX_WINDOW_TEXT_CHARS as i32 + 1, 0),
            Err(ERROR_INSUFFICIENT_BUFFER)
        );
    }

    #[test]
    fn wide_text_keeps_buffer_for_same_value() {
        let mut text = WideText::new("Ready");
        let wide = text.wide().as_ptr();

        text.replace_str("Ready");

        assert_eq!(text.wide().as_ptr(), wide);
        assert_eq!(
            text.wide(),
            &[
                b'R' as u16,
                b'e' as u16,
                b'a' as u16,
                b'd' as u16,
                b'y' as u16,
                0
            ]
        );
    }

    #[test]
    fn visible_tab_label_cache_tracks_only_overflow_range_after_tab_changes() {
        let active_tab = TabId::new(7);
        let mut tabs = (1..=12)
            .map(|tab_id| TestPaintTab::new(tab_id, format!("Tab {tab_id}")))
            .collect::<Vec<_>>();
        let mut cache = vec![
            CachedTabLabel::new(TabId::new(1), "stale hidden"),
            CachedTabLabel::new(TabId::new(2), "stale hidden 2"),
        ];

        let mut layout =
            overflow_tab_label_layout(tabs.len(), test_tab_index(&tabs, active_tab), 0);
        sync_test_visible_tab_labels(&mut cache, &tabs, layout);
        assert_cache_matches_visible_tabs(&cache, &tabs, layout);
        assert!(cache.len() < tabs.len());
        assert!(!cache.iter().any(|label| label.label.text == "Tab 1"));

        tabs[layout.first_visible_index].name = "Renamed visible tab".to_owned();
        sync_test_visible_tab_labels(&mut cache, &tabs, layout);
        assert_cache_matches_visible_tabs(&cache, &tabs, layout);

        tabs.push(TestPaintTab::new(99, "Added tab"));
        layout = overflow_tab_label_layout(
            tabs.len(),
            test_tab_index(&tabs, active_tab),
            layout.first_visible_index,
        );
        sync_test_visible_tab_labels(&mut cache, &tabs, layout);
        assert_cache_matches_visible_tabs(&cache, &tabs, layout);

        let Some(delete_index) = (layout.first_visible_index..layout.visible_end_index())
            .find(|&index| tabs[index].tab_id != active_tab)
        else {
            panic!("expected a non-active visible tab to delete");
        };
        tabs.remove(delete_index);
        layout = overflow_tab_label_layout(
            tabs.len(),
            test_tab_index(&tabs, active_tab),
            layout.first_visible_index,
        );
        sync_test_visible_tab_labels(&mut cache, &tabs, layout);
        assert_cache_matches_visible_tabs(&cache, &tabs, layout);

        let reorder_from = layout.first_visible_index;
        let moved = tabs.remove(reorder_from);
        let reorder_to = layout.visible_end_index().min(tabs.len());
        tabs.insert(reorder_to, moved);
        layout = overflow_tab_label_layout(
            tabs.len(),
            test_tab_index(&tabs, active_tab),
            layout.first_visible_index,
        );
        sync_test_visible_tab_labels(&mut cache, &tabs, layout);
        assert_cache_matches_visible_tabs(&cache, &tabs, layout);
        assert!(cache.len() < tabs.len());
    }

    #[test]
    fn button_specs_have_localized_labels() {
        for spec in BUTTON_SPECS {
            assert!(!command_button_label(UiLanguage::English, spec.command).is_empty());
            assert!(!command_button_label(UiLanguage::Korean, spec.command).is_empty());
        }

        assert_eq!(
            workspace_ui_toggle_button_label(UiLanguage::English, true),
            "Hide"
        );
        assert_eq!(
            workspace_ui_toggle_button_label(UiLanguage::Korean, false),
            "표시"
        );
        assert_eq!(
            workspace_ui_toggle_menu_label(UiLanguage::English, true),
            "Hide Workspace Controls"
        );
    }

    #[test]
    fn about_dialog_text_includes_package_version() {
        let text = about_dialog_text(UiLanguage::English);

        assert!(text.contains("j3GridDocker"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("https://github.com/edgarp9"));
        assert_eq!(
            about_dialog_link_markup(),
            r#"<a href="https://github.com/edgarp9">https://github.com/edgarp9</a>"#
        );
    }

    #[test]
    fn shell_execute_success_policy_matches_documented_threshold() {
        assert!(!shell_execute_succeeded(null_mut()));
        assert!(!shell_execute_succeeded(32usize as HINSTANCE));
        assert!(shell_execute_succeeded(33usize as HINSTANCE));
    }

    #[test]
    fn settings_save_policy_waits_after_load_failure() {
        let policy = SettingsSavePolicy::WaitForWorkspaceChange;

        assert!(!policy.can_save());
    }

    #[test]
    fn settings_save_policy_keeps_waiting_after_load_failure_options_change() {
        let mut policy = SettingsSavePolicy::WaitForWorkspaceChange;

        policy.allow_after_workspace_options_change();

        assert_eq!(policy, SettingsSavePolicy::WaitForWorkspaceChange);
        assert_eq!(policy.save_mode(), None);
        assert!(!policy.can_save());
    }

    #[test]
    fn settings_save_policy_allows_save_after_workspace_change() {
        let mut policy = SettingsSavePolicy::WaitForWorkspaceChange;

        policy.allow_after_workspace_change();

        assert_eq!(policy, SettingsSavePolicy::Enabled);
        assert!(policy.can_save());
        assert_eq!(policy.save_mode(), Some(SettingsSaveMode::FullWorkspace));
    }

    #[test]
    fn settings_save_policy_preserves_startup_session_for_options_only_change() {
        let mut policy = SettingsSavePolicy::PreserveStartupSessionUntilWorkspaceChange;

        policy.allow_after_workspace_options_change();

        assert_eq!(
            policy.save_mode(),
            Some(SettingsSaveMode::OptionsOnlyPreservingStartupSession)
        );
        assert!(policy.can_save());
    }

    #[test]
    fn settings_save_policy_saves_full_workspace_after_explicit_workspace_change() {
        let mut policy = SettingsSavePolicy::PreserveStartupSessionUntilWorkspaceChange;

        policy.allow_after_workspace_change();

        assert_eq!(policy.save_mode(), Some(SettingsSaveMode::FullWorkspace));
        assert!(policy.can_save());
    }

    #[test]
    fn next_tab_number_allows_zero_based_empty_workspace() {
        assert_eq!(next_tab_number(0), 0);
        assert_eq!(next_tab_number(3), 3);
    }

    #[test]
    fn tab_deletion_status_mentions_active_tab_and_undock_counts() {
        let report = TabDeletionReport::new(TabId::new(3), Some(TabId::new(3)), None);

        assert_eq!(
            tab_deletion_status_text(&report),
            "탭 3 삭제 완료. 현재 활성 탭: 없음. Undock: attempted 0, restored 0, missing 0, failures 0"
        );
    }

    #[test]
    fn docked_window_selection_status_explains_move_and_detach() {
        let status = docked_window_selection_status_text();

        assert!(status.contains("빈 영역으로 끌면 이동"));
        assert!(status.contains("바깥으로 끌면 배치 해제"));
    }

    #[test]
    fn drop_registration_error_status_names_move_to_occupied_region() {
        let source = RegionId::new(1);
        let target = RegionId::new(2);
        let error = AppError::Domain(DomainError::RegionAlreadyOccupied(target));

        let status = drop_registration_error_status_text(Some(source), target, &error);

        assert_eq!(
            status,
            "외부 윈도우 이동 실패: 대상 영역에 이미 다른 외부 윈도우가 있습니다."
        );
    }

    #[test]
    fn drop_registration_error_status_distinguishes_place_and_resync_failures() {
        let target = RegionId::new(2);
        let occupied = AppError::Domain(DomainError::RegionAlreadyOccupied(target));
        let position = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "외부 윈도우 위치를 변경할 수 없습니다.",
            Some("set position failed".to_owned()),
        ));

        assert_eq!(
            drop_registration_error_status_text(None, target, &occupied),
            "외부 윈도우 배치 실패: 대상 영역에 이미 다른 외부 윈도우가 있습니다."
        );
        assert_eq!(
            drop_registration_error_status_text(Some(target), target, &position),
            "외부 윈도우 현재 영역 재맞춤 실패: 기존 영역을 유지했습니다. 외부 윈도우 위치를 변경할 수 없습니다."
        );
    }

    #[test]
    fn shutdown_completion_waits_for_failed_undocks() {
        let complete = ShutdownReport::new(2, 1, 1, Vec::new());
        let failure = crate::app::WindowControlError::new(
            WindowOperation::Restore,
            None,
            "외부 윈도우를 복원할 수 없습니다.",
            Some(String::from("test restore failure")),
        );
        let incomplete = ShutdownReport::new(1, 0, 0, vec![failure]);

        assert!(shutdown_report_is_complete(&complete));
        assert!(!shutdown_report_is_complete(&incomplete));
    }

    #[test]
    fn shutdown_report_after_settings_save_cancels_cancellable_shutdown_on_save_failure() {
        let shutdown_called = std::cell::Cell::new(false);
        let error =
            ShutdownSettingsSaveError::App(AppError::Window(crate::app::WindowControlError::new(
                WindowOperation::Restore,
                None,
                "외부 윈도우를 복원할 수 없습니다.",
                Some(String::from("test settings save failure")),
            )));

        let result =
            shutdown_report_after_settings_save(Err(error), ShutdownMode::Cancellable, || {
                shutdown_called.set(true);
                ShutdownReport::new(1, 1, 0, Vec::new())
            });

        assert!(result.is_err());
        assert!(!shutdown_called.get());
    }

    #[test]
    fn shutdown_report_after_settings_save_runs_forced_shutdown_on_save_failure() {
        let shutdown_called = std::cell::Cell::new(false);
        let error =
            ShutdownSettingsSaveError::App(AppError::Window(crate::app::WindowControlError::new(
                WindowOperation::Restore,
                None,
                "외부 윈도우를 복원할 수 없습니다.",
                Some(String::from("test settings save failure")),
            )));

        let result = shutdown_report_after_settings_save(Err(error), ShutdownMode::Forced, || {
            shutdown_called.set(true);
            let failure = crate::app::WindowControlError::new(
                WindowOperation::Restore,
                None,
                "외부 윈도우를 복원할 수 없습니다.",
                Some(String::from("test undock failure")),
            );
            ShutdownReport::new(1, 0, 0, vec![failure])
        });
        let attempt = match result {
            Ok(attempt) => attempt,
            Err(_) => panic!("forced shutdown should continue after settings save failure"),
        };

        assert!(shutdown_called.get());
        assert!(attempt.settings_save_error.is_some());
        assert_eq!(attempt.report.attempted(), 1);
        assert_eq!(attempt.report.failures().len(), 1);
        assert!(!shutdown_report_is_complete(&attempt.report));
    }

    #[test]
    fn shutdown_report_after_settings_save_runs_shutdown_on_save_success() {
        let shutdown_called = std::cell::Cell::new(false);

        let result = shutdown_report_after_settings_save(Ok(()), ShutdownMode::Cancellable, || {
            shutdown_called.set(true);
            ShutdownReport::new(1, 1, 0, Vec::new())
        });
        let attempt = match result {
            Ok(attempt) => attempt,
            Err(_) => panic!("settings save success should run shutdown"),
        };

        assert!(shutdown_called.get());
        assert!(attempt.settings_save_error.is_none());
    }

    #[test]
    fn switch_tab_success_status_names_tabs_and_stale_removals() {
        let context = TabSwitchStatusContext {
            target: TabStatusLabel::new(TabId::new(2), Some("Second".to_owned())),
            previous_active: Some(TabStatusLabel::new(TabId::new(1), Some("First".to_owned()))),
        };
        let report = TabSwitchReport::with_stale_placements(
            crate::domain::ActiveTabChange::new(Some(TabId::new(1)), TabId::new(2)),
            1,
            1,
        );

        let status = switch_tab_success_status_text(&context, report);

        assert!(status.contains("탭을 전환했습니다: Second (탭 2)"));
        assert!(status.contains("이전 활성 탭: First (탭 1)"));
        assert!(status.contains("유효하지 않은 대상 창 1개"));
        assert!(status.contains("유효하지 않은 이전 활성 탭 창 1개"));
    }

    #[test]
    fn switch_tab_show_failure_status_says_previous_tab_is_kept() {
        let context = TabSwitchStatusContext {
            target: TabStatusLabel::new(TabId::new(2), Some("Second".to_owned())),
            previous_active: Some(TabStatusLabel::new(TabId::new(1), Some("First".to_owned()))),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::Show,
            None,
            "외부 윈도우를 표시할 수 없습니다.",
            Some("show failed".to_owned()),
        ));

        let status = switch_tab_failure_status_text(&context, Some(TabId::new(1)), &error);

        assert!(status.contains("대상 탭 창 표시 실패: Second (탭 2)"));
        assert!(status.contains("이전 활성 탭 First (탭 1)을 유지했습니다"));
        assert!(status.contains("외부 윈도우를 표시할 수 없습니다."));
    }

    #[test]
    fn switch_tab_position_failure_status_mentions_target_hide_rollback() {
        let context = TabSwitchStatusContext {
            target: TabStatusLabel::new(TabId::new(2), Some("Second".to_owned())),
            previous_active: Some(TabStatusLabel::new(TabId::new(1), Some("First".to_owned()))),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "외부 윈도우 위치를 조정할 수 없습니다.",
            Some("set position failed".to_owned()),
        ));

        let status = switch_tab_failure_status_text(&context, Some(TabId::new(1)), &error);

        assert!(status.contains("대상 탭 창 배치 실패"));
        assert!(status.contains("대상 탭 창 숨김 롤백을 시도했습니다"));
        assert!(status.contains("이전 활성 탭 First (탭 1)을 유지했습니다"));
    }

    #[test]
    fn switch_tab_same_active_failure_status_is_distinct() {
        let context = TabSwitchStatusContext {
            target: TabStatusLabel::new(TabId::new(1), Some("First".to_owned())),
            previous_active: Some(TabStatusLabel::new(TabId::new(1), Some("First".to_owned()))),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "외부 윈도우 위치를 조정할 수 없습니다.",
            Some("set position failed".to_owned()),
        ));

        let status = switch_tab_failure_status_text(&context, Some(TabId::new(1)), &error);

        assert!(status.contains("같은 탭 재표시 실패: First (탭 1)"));
        assert!(status.contains("활성 탭 First (탭 1)을 그대로 유지했습니다"));
    }

    #[test]
    fn tab_deletion_auto_switch_failure_status_names_target_and_rollback() {
        let context = TabDeletionStatusContext {
            deleted: TabStatusLabel::new(TabId::new(1), Some("First".to_owned())),
            previous_active: Some(TabStatusLabel::new(TabId::new(1), Some("First".to_owned()))),
            automatic_target: Some(TabStatusLabel::new(
                TabId::new(2),
                Some("Second".to_owned()),
            )),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "외부 윈도우 위치를 조정할 수 없습니다.",
            Some("set position failed".to_owned()),
        ));

        let status = tab_deletion_error_status_text(&context, Some(TabId::new(1)), &error);

        assert!(status.contains("탭 삭제 후 자동 전환 실패"));
        assert!(status.contains("삭제 대상 First (탭 1)"));
        assert!(status.contains("전환 대상 Second (탭 2)"));
        assert!(status.contains("삭제를 롤백했고 현재 활성 탭: First (탭 1)"));
    }

    #[test]
    fn command_ids_do_not_overlap_with_dynamic_menu_ranges() {
        let mut fixed_commands = BUTTON_SPECS
            .iter()
            .map(|spec| ("toolbar", spec.command))
            .collect::<Vec<_>>();
        fixed_commands.extend([
            ("view-workspace-toggle", CMD_WORKSPACE_UI_TOGGLE),
            ("top-bar-new-tab", CMD_TAB_ADD),
            ("tab-preset-save", CMD_TAB_PRESET_SAVE),
            ("tab-preset-load", CMD_TAB_PRESET_LOAD),
            ("tab-preset-delete", CMD_TAB_PRESET_DELETE),
            ("tab-preset-edit", CMD_TAB_PRESET_EDIT),
            ("legacy-options-menu", CMD_OPTIONS),
            ("tab-context-rename", CMD_TAB_RENAME_CONTEXT),
            ("tab-context-close", CMD_TAB_CLOSE_CONTEXT),
            ("tab-context-close-other", CMD_TAB_CLOSE_OTHER_CONTEXT),
            (
                "options-dock-hidden-workspace-ui",
                CMD_DOCK_HIDDEN_WORKSPACE_UI_TOGGLE,
            ),
            ("options-language-english", CMD_LANGUAGE_ENGLISH),
            ("options-language-korean", CMD_LANGUAGE_KOREAN),
            ("window-minimize", CMD_WINDOW_MINIMIZE),
            ("window-maximize-restore", CMD_WINDOW_MAXIMIZE_RESTORE),
            ("window-close", CMD_WINDOW_CLOSE),
            ("help-about", CMD_ABOUT),
        ]);

        assert!(BUTTON_SPECS.iter().all(|spec| matches!(
            spec.command,
            CMD_SPLIT_VERTICAL | CMD_SPLIT_HORIZONTAL | CMD_REGION_DELETE | CMD_UNDOCK
        )));
        assert!(
            !BUTTON_SPECS
                .iter()
                .any(|spec| matches!(spec.command, CMD_OPTIONS | CMD_ABOUT))
        );

        let mut seen = std::collections::HashSet::new();
        for (label, command) in fixed_commands {
            assert!(seen.insert(command), "duplicate command id {command}");
            assert!(
                command < CMD_TAB_OVERFLOW_BASE,
                "{label} command {command} overlaps dynamic menu command space"
            );
            assert!(!(CMD_TAB_OVERFLOW_BASE..CMD_TAB_OVERFLOW_END).contains(&command));
            assert!(!(CMD_TAB_PRESET_BASE..CMD_TAB_PRESET_END).contains(&command));
            assert!(!(CMD_TAB_PRESET_DELETE_BASE..CMD_TAB_PRESET_DELETE_END).contains(&command));
            assert!(!(CMD_TAB_PRESET_EDIT_BASE..CMD_TAB_PRESET_EDIT_END).contains(&command));
        }

        const {
            assert!(CMD_TAB_OVERFLOW_BASE < CMD_TAB_OVERFLOW_END);
            assert!(CMD_TAB_OVERFLOW_END <= CMD_TAB_PRESET_BASE);
            assert!(CMD_TAB_PRESET_BASE < CMD_TAB_PRESET_END);
            assert!(CMD_TAB_PRESET_END <= CMD_TAB_PRESET_DELETE_BASE);
            assert!(CMD_TAB_PRESET_DELETE_BASE < CMD_TAB_PRESET_DELETE_END);
            assert!(CMD_TAB_PRESET_DELETE_END <= CMD_TAB_PRESET_EDIT_BASE);
            assert!(CMD_TAB_PRESET_EDIT_BASE < CMD_TAB_PRESET_EDIT_END);
        };

        let overflow_last_index = usize::from(CMD_TAB_OVERFLOW_END - CMD_TAB_OVERFLOW_BASE - 1);
        assert_eq!(
            tab_overflow_command_for_index(overflow_last_index),
            Some(CMD_TAB_OVERFLOW_END - 1)
        );
        assert_eq!(
            tab_overflow_command_for_index(overflow_last_index + 1),
            None
        );
        assert_eq!(
            command_index_from_range(CMD_TAB_PRESET_BASE, CMD_TAB_PRESET_BASE, CMD_TAB_PRESET_END),
            Some(0)
        );
        assert_eq!(
            command_index_from_range(
                CMD_TAB_PRESET_DELETE_BASE,
                CMD_TAB_PRESET_DELETE_BASE,
                CMD_TAB_PRESET_DELETE_END
            ),
            Some(0)
        );
        assert_eq!(
            command_index_from_range(
                CMD_TAB_PRESET_EDIT_BASE,
                CMD_TAB_PRESET_EDIT_BASE,
                CMD_TAB_PRESET_EDIT_END
            ),
            Some(0)
        );
    }

    #[test]
    fn program_arguments_parser_splits_whitespace_and_quotes() {
        let arguments = parse_program_arguments(r#"--profile "Work A" "" "quoted \"value\"""#)
            .expect("arguments should parse");

        assert_eq!(
            arguments,
            vec![
                String::from("--profile"),
                String::from("Work A"),
                String::new(),
                String::from("quoted \"value\"")
            ]
        );
    }

    #[test]
    fn program_edit_dialog_scroll_metrics_enable_large_program_lists() {
        let three_programs_height = program_edit_dialog_content_height(3);
        let ten_programs_height = program_edit_dialog_content_height(10);

        assert_eq!(
            program_edit_dialog_max_scroll_position(three_programs_height),
            0
        );
        assert!(program_edit_dialog_max_scroll_position(ten_programs_height) > 0);
    }

    #[test]
    fn program_edit_dialog_buttons_fit_inside_client_area() {
        assert!(program_edit_dialog_button_bottom() <= program_edit_dialog_client_height());
    }

    #[test]
    fn program_edit_dialog_resize_metrics_keep_scroll_area_visible() {
        let min_viewport =
            program_edit_dialog_viewport_height_for_test(program_edit_dialog_min_client_height());
        let default_viewport =
            program_edit_dialog_viewport_height_for_test(program_edit_dialog_client_height());
        let taller_viewport =
            program_edit_dialog_viewport_height_for_test(program_edit_dialog_client_height() + 120);

        assert!(min_viewport > 0);
        assert!(default_viewport >= min_viewport);
        assert!(taller_viewport > default_viewport);
    }

    #[test]
    fn program_arguments_formatter_round_trips_shell_like_input() {
        let arguments = vec![
            String::from("--profile"),
            String::from("Work A"),
            String::new(),
            String::from("quoted \"value\""),
        ];
        let formatted = format_program_arguments(&arguments);

        assert_eq!(
            parse_program_arguments(&formatted).expect("formatted arguments should parse"),
            arguments
        );
    }

    #[test]
    fn program_arguments_formatter_round_trips_trailing_backslashes() {
        let arguments = vec![
            String::from("C:\\Program Files\\Tool\\"),
            String::from("quoted \"folder\" ending\\"),
        ];
        let formatted = format_program_arguments(&arguments);

        assert_eq!(
            parse_program_arguments(&formatted).expect("formatted arguments should parse"),
            arguments
        );
    }

    #[test]
    fn program_arguments_parser_rejects_unterminated_quote() {
        assert_eq!(
            parse_program_arguments(r#""Work A"#),
            Err(ProgramArgumentsParseError::UnterminatedQuote)
        );
    }

    #[test]
    fn tab_context_command_dispatch_maps_menu_ids_to_actions() {
        assert_eq!(
            tab_context_action_from_command(CMD_TAB_RENAME_CONTEXT),
            Some(TabContextAction::Rename)
        );
        assert_eq!(
            tab_context_action_from_command(CMD_TAB_CLOSE_CONTEXT),
            Some(TabContextAction::Close)
        );
        assert_eq!(
            tab_context_action_from_command(CMD_TAB_CLOSE_OTHER_CONTEXT),
            Some(TabContextAction::CloseOther)
        );
        assert_eq!(tab_context_action_from_command(CMD_TAB_ADD), None);
    }

    #[test]
    fn popup_selected_command_ignores_dismissed_or_destroyed_owner() {
        assert_eq!(popup_selected_command(0, test_hwnd(100)), None);
        assert_eq!(
            popup_selected_command(i32::from(CMD_OPTIONS), null_mut()),
            None
        );
    }

    #[test]
    fn popup_selected_command_returns_alive_owner_command_without_truncation() {
        assert_eq!(
            popup_selected_command(i32::from(CMD_OPTIONS), test_hwnd(100)),
            Some(CMD_OPTIONS)
        );
        assert_eq!(
            popup_selected_command(i32::from(u16::MAX) + 1, test_hwnd(100)),
            None
        );
    }

    #[test]
    fn tab_context_target_selection_uses_tab_hit_even_on_close_button() {
        let tab_id = TabId::new(7);

        assert_eq!(
            tab_context_target_from_hit(Some(TabHit {
                tab_id,
                target: TabHitTarget::Body
            })),
            Some(tab_id)
        );
        assert_eq!(
            tab_context_target_from_hit(Some(TabHit {
                tab_id,
                target: TabHitTarget::CloseButton
            })),
            Some(tab_id)
        );
        assert_eq!(tab_context_target_from_hit(None), None);
    }

    #[test]
    fn tab_context_target_selection_uses_clicked_tab_not_active_tab() {
        let active_tab = TabId::new(1);
        let clicked_tab = TabId::new(3);

        assert_ne!(active_tab, clicked_tab);
        assert_eq!(
            tab_context_target_from_hit(Some(TabHit {
                tab_id: clicked_tab,
                target: TabHitTarget::Body
            })),
            Some(clicked_tab)
        );
    }

    #[test]
    fn tab_press_action_separates_body_close_and_hidden_workspace_policy() {
        let tab_id = TabId::new(4);

        assert_eq!(
            tab_press_action_from_hit(
                TabHit {
                    tab_id,
                    target: TabHitTarget::Body
                },
                true
            ),
            TabPressAction::Pending(PendingTabAction::ClickOrReorder)
        );
        assert_eq!(
            tab_press_action_from_hit(
                TabHit {
                    tab_id,
                    target: TabHitTarget::Body
                },
                false
            ),
            TabPressAction::Pending(PendingTabAction::ClickOrWindowMove)
        );
        assert_eq!(
            tab_press_action_from_hit(
                TabHit {
                    tab_id,
                    target: TabHitTarget::CloseButton
                },
                false
            ),
            TabPressAction::Close(tab_id)
        );
    }

    #[test]
    fn tab_body_target_ignores_close_button_hit() {
        let tab_id = TabId::new(5);

        assert_eq!(
            tab_body_target_from_hit(Some(TabHit {
                tab_id,
                target: TabHitTarget::Body
            })),
            Some(tab_id)
        );
        assert_eq!(
            tab_body_target_from_hit(Some(TabHit {
                tab_id,
                target: TabHitTarget::CloseButton
            })),
            None
        );
    }

    #[test]
    fn close_other_tab_targets_keep_context_tab_and_preserve_iteration_order() {
        let tabs = [TabId::new(1), TabId::new(2), TabId::new(3), TabId::new(4)];

        assert_eq!(
            close_other_tab_targets(&tabs, TabId::new(3)),
            Some(vec![TabId::new(1), TabId::new(2), TabId::new(4)])
        );
        assert_eq!(close_other_tab_targets(&tabs, TabId::new(9)), None);
    }

    #[test]
    fn close_other_tabs_failure_status_names_failed_tab_operation() {
        let failures = [TabOperationFailure {
            tab_id: TabId::new(4),
            operation: "삭제",
            message: "외부 윈도우를 복원할 수 없습니다.".to_owned(),
        }];

        let status = close_other_tabs_status_text(
            TabId::new(2),
            3,
            2,
            Some(TabId::new(2)),
            UndockCounts {
                attempted: 2,
                restored: 1,
                missing: 0,
                failures: 1,
            },
            &failures,
        );

        assert!(status.contains("탭 4 삭제"));
        assert!(status.contains("성공 2/3"));
        assert!(status.contains("failures 1"));
    }

    #[test]
    fn tab_reorder_status_reports_target_and_noop() {
        let changed = tab_reorder_status_text(TabId::new(3), Some(TabId::new(1)), true);
        let unchanged = tab_reorder_status_text(TabId::new(3), None, false);

        assert_eq!(changed, "탭 순서를 변경했습니다: 탭 3 -> 탭 1 앞");
        assert_eq!(
            unchanged,
            "탭 순서를 변경하지 않았습니다: 탭 3 위치가 그대로입니다."
        );
    }

    #[test]
    fn tab_context_actions_have_stable_trace_names() {
        assert_eq!(TabContextAction::Rename.trace_name(), "rename");
        assert_eq!(TabContextAction::Close.trace_name(), "close");
        assert_eq!(TabContextAction::CloseOther.trace_name(), "close-other");
    }

    #[test]
    fn window_maximize_restore_menu_label_matches_current_state() {
        assert_eq!(
            window_maximize_restore_menu_label(UiLanguage::English, false),
            "Maximize window"
        );
        assert_eq!(
            window_maximize_restore_menu_label(UiLanguage::English, true),
            "Restore window"
        );
        assert_eq!(
            window_maximize_restore_menu_label(UiLanguage::Korean, false),
            "창 최대화"
        );
    }

    #[test]
    fn text_input_dialog_cleanup_destroys_alive_dialog_before_owner_restore() {
        let steps = std::cell::RefCell::new(Vec::new());

        cleanup_text_input_dialog_with(
            true,
            || steps.borrow_mut().push("destroy"),
            || steps.borrow_mut().push("enable"),
            || steps.borrow_mut().push("focus"),
        );

        assert_eq!(steps.into_inner(), vec!["destroy", "enable", "focus"]);
    }

    #[test]
    fn text_input_dialog_cleanup_skips_destroy_for_missing_dialog() {
        let steps = std::cell::RefCell::new(Vec::new());

        cleanup_text_input_dialog_with(
            false,
            || steps.borrow_mut().push("destroy"),
            || steps.borrow_mut().push("enable"),
            || steps.borrow_mut().push("focus"),
        );

        assert_eq!(steps.into_inner(), vec!["enable", "focus"]);
    }

    #[test]
    fn hwnd_owner_filter_matches_direct_owner() {
        let owner = test_hwnd(100);
        let owned = test_hwnd(200);

        assert!(hwnd_is_owned_by_with(owned, owner, |candidate| {
            if candidate == owned {
                owner
            } else {
                null_mut()
            }
        }));
    }

    #[test]
    fn hwnd_owner_filter_ignores_null_and_other_owner() {
        let owner = test_hwnd(100);
        let owned = test_hwnd(200);
        let other_owner = test_hwnd(300);

        assert!(!hwnd_is_owned_by_with(null_mut(), owner, |_| owner));
        assert!(!hwnd_is_owned_by_with(owned, null_mut(), |_| owner));
        assert!(!hwnd_is_owned_by_with(owned, owner, |_| other_owner));
        assert!(!hwnd_is_owned_by_with(owned, owner, |_| null_mut()));
    }

    #[test]
    fn external_root_candidate_marks_docker_owned_roots() -> Result<(), Box<dyn std::error::Error>>
    {
        let docker = test_hwnd(100);
        let child = test_hwnd(150);
        let root = test_hwnd(200);

        let candidate = external_root_candidate_from_hwnd_with(
            child,
            docker,
            |hwnd| if hwnd == child { root } else { hwnd },
            |hwnd| if hwnd == root { docker } else { null_mut() },
        )
        .ok_or("expected external root candidate")?;

        assert_eq!(candidate.hwnd, WindowHandle::new(root as isize)?);
        assert!(candidate.docker_owned);

        Ok(())
    }

    #[test]
    fn external_root_candidate_keeps_unowned_external_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        let docker = test_hwnd(100);
        let child = test_hwnd(150);
        let root = test_hwnd(200);

        let candidate = external_root_candidate_from_hwnd_with(
            child,
            docker,
            |hwnd| if hwnd == child { root } else { hwnd },
            |_| null_mut(),
        )
        .ok_or("expected external root candidate")?;

        assert_eq!(candidate.hwnd, WindowHandle::new(root as isize)?);
        assert!(!candidate.docker_owned);

        Ok(())
    }

    #[test]
    fn external_root_candidate_rejects_docker_window_root() {
        let docker = test_hwnd(100);

        assert_eq!(
            external_root_candidate_from_hwnd_with(docker, docker, |hwnd| hwnd, |_| null_mut()),
            None
        );
    }

    #[test]
    fn drop_hit_test_policy_allows_hidden_workspace_only_when_option_is_enabled() {
        assert!(drop_uses_workspace_hit_test(
            true,
            WorkspaceOptions::default()
        ));
        assert!(!drop_uses_workspace_hit_test(
            false,
            WorkspaceOptions::default()
        ));
        assert!(drop_uses_workspace_hit_test(
            false,
            WorkspaceOptions::new(true)
        ));
    }

    #[test]
    fn splitter_overlay_policy_requires_visible_idle_workspace_and_ctrl() {
        assert!(splitter_overlay_should_show(
            true, false, false, false, true
        ));
        assert!(splitter_overlay_should_show(
            false, true, false, false, true
        ));
        assert!(!splitter_overlay_should_show(
            false, false, false, false, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, true, false, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, false, true, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, false, false, false
        ));
    }

    #[test]
    fn drop_tracker_ignores_unmoved_click() -> Result<(), Box<dyn std::error::Error>> {
        let hwnd = WindowHandle::new(100)?;
        let mut tracker = DropTracker::default();
        let rect = TrackedWindowRect::new(10, 10, 210, 160);

        tracker.begin_press();
        tracker.set_candidate(hwnd, rect);

        assert_eq!(tracker.finish_press(), None);
        assert_eq!(tracker.finish_press(), None);

        Ok(())
    }

    #[test]
    fn drop_tracker_returns_candidate_after_window_moves() -> Result<(), Box<dyn std::error::Error>>
    {
        let hwnd = WindowHandle::new(100)?;
        let mut tracker = DropTracker::default();

        tracker.begin_press();
        tracker.set_candidate(hwnd, TrackedWindowRect::new(10, 10, 210, 160));
        tracker.observe_candidate_rect(
            TrackedWindowRect::new(15, 15, 215, 165),
            DROP_WINDOW_MOVE_THRESHOLD,
        );

        assert_eq!(tracker.finish_press(), Some(hwnd));
        assert_eq!(tracker.finish_press(), None);

        Ok(())
    }

    #[test]
    fn drop_tracker_keeps_first_candidate() -> Result<(), Box<dyn std::error::Error>> {
        let first = WindowHandle::new(100)?;
        let second = WindowHandle::new(200)?;
        let mut tracker = DropTracker::default();

        tracker.begin_press();
        tracker.set_candidate(first, TrackedWindowRect::new(10, 10, 210, 160));
        tracker.set_candidate(second, TrackedWindowRect::new(20, 20, 220, 170));
        tracker.observe_candidate_rect(
            TrackedWindowRect::new(16, 10, 216, 160),
            DROP_WINDOW_MOVE_THRESHOLD,
        );

        assert_eq!(tracker.finish_press(), Some(first));

        Ok(())
    }

    #[test]
    fn drop_tracker_suppression_cancels_candidate() -> Result<(), Box<dyn std::error::Error>> {
        let hwnd = WindowHandle::new(100)?;
        let mut tracker = DropTracker::default();

        tracker.begin_press();
        tracker.set_candidate(hwnd, TrackedWindowRect::new(10, 10, 210, 160));
        tracker.observe_candidate_rect(
            TrackedWindowRect::new(20, 20, 220, 170),
            DROP_WINDOW_MOVE_THRESHOLD,
        );
        tracker.suppress_drop();

        assert_eq!(tracker.finish_press(), None);

        Ok(())
    }

    #[test]
    fn tracked_window_rect_converts_to_domain_rect() -> Result<(), DomainError> {
        let tracked = TrackedWindowRect::new(320, 240, 620, 460);

        assert_eq!(
            tracked.to_domain_rect(),
            Some(Rect::new(320, 240, 300, 220)?)
        );
        assert_eq!(
            TrackedWindowRect::new(320, 240, 320, 460).to_domain_rect(),
            None
        );

        Ok(())
    }

    #[test]
    fn tracked_window_rect_contains_screen_point_with_half_open_edges() {
        let tracked = TrackedWindowRect::new(10, 20, 110, 120);

        assert!(tracked.contains_point(ScreenPoint { x: 10, y: 20 }));
        assert!(tracked.contains_point(ScreenPoint { x: 109, y: 119 }));
        assert!(!tracked.contains_point(ScreenPoint { x: 110, y: 119 }));
        assert!(!tracked.contains_point(ScreenPoint { x: 109, y: 120 }));
    }

    #[test]
    fn hidden_workspace_ui_style_removes_title_bar_bits() {
        let current = WS_OVERLAPPEDWINDOW as isize;
        let hidden = window_style_for_workspace_ui_visibility(current, false, false);

        assert_eq!(hidden & WS_CAPTION as isize, 0);
        assert_eq!(hidden & WS_THICKFRAME as isize, WS_THICKFRAME as isize);
        assert_eq!(
            window_style_for_workspace_ui_visibility(hidden, true, false) & WS_CAPTION as isize,
            WS_CAPTION as isize
        );
    }

    #[test]
    fn hidden_maximized_workspace_ui_removes_resize_frame_bits() {
        let current = WS_OVERLAPPEDWINDOW as isize;
        let hidden = window_style_for_workspace_ui_visibility(current, false, true);

        assert_eq!(hidden & WS_CAPTION as isize, 0);
        assert_eq!(hidden & WS_THICKFRAME as isize, 0);

        let visible = window_style_for_workspace_ui_visibility(hidden, true, true);
        assert_eq!(visible & WS_CAPTION as isize, WS_CAPTION as isize);
        assert_eq!(visible & WS_THICKFRAME as isize, WS_THICKFRAME as isize);
    }

    #[test]
    fn workspace_ui_visibility_controls_native_main_menu() {
        assert!(main_menu_visible_for_workspace_ui(true));
        assert!(!main_menu_visible_for_workspace_ui(false));
    }

    #[test]
    fn main_menu_size_refresh_runs_only_for_visible_maximize_state_changes() {
        assert!(!main_menu_needs_refresh_after_size(
            true,
            Some(false),
            false
        ));
        assert!(main_menu_needs_refresh_after_size(true, Some(false), true));
        assert!(main_menu_needs_refresh_after_size(true, None, false));
        assert!(!main_menu_needs_refresh_after_size(
            false,
            Some(false),
            true
        ));
        assert!(!main_menu_needs_refresh_after_size(false, None, true));
    }

    #[test]
    fn workspace_ui_transition_exposes_target_state_during_chrome_updates() {
        let mut visibility = WorkspaceUiVisibility::new(true);

        let transition = visibility.begin_toggle();

        assert_eq!(
            transition,
            WorkspaceUiVisibilityTransition {
                previous_visible: true,
                desired_visible: false
            }
        );
        assert!(visibility.committed_visible());
        assert!(!visibility.effective_visible());

        visibility.rollback(transition);

        assert!(visibility.committed_visible());
        assert!(visibility.effective_visible());
    }

    #[test]
    fn workspace_ui_transition_commit_updates_committed_state() {
        let mut visibility = WorkspaceUiVisibility::new(true);
        let transition = visibility.begin_toggle();

        visibility.commit(transition);

        assert!(!visibility.committed_visible());
        assert!(!visibility.effective_visible());
    }

    #[test]
    fn hidden_maximized_bounds_restore_runs_when_later_chrome_step_fails() {
        let previous_bounds = RECT {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 1048,
        };
        let failure = Win32StatusFailure::new("SetWindowRgn", 5, "region failed");
        let mut restored_bounds = None;

        let result: Result<(), Win32StatusFailure> =
            restore_hidden_maximized_bounds_after_chrome_failure(
                Err(failure),
                Some(previous_bounds),
                |bounds| {
                    restored_bounds = Some(bounds);
                    Ok(())
                },
            );

        assert_eq!(result, Err(failure));
        let Some(restored_bounds) = restored_bounds else {
            panic!("expected previous bounds to be restored");
        };
        assert_rect_eq(
            restored_bounds,
            RECT {
                left: -8,
                top: -8,
                right: 1928,
                bottom: 1048,
            },
        );
    }

    #[test]
    fn hidden_maximized_bounds_restore_is_skipped_after_successful_chrome_steps() {
        let previous_bounds = RECT {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 1048,
        };
        let mut restore_called = false;

        let result = restore_hidden_maximized_bounds_after_chrome_failure(
            Ok(()),
            Some(previous_bounds),
            |_bounds| {
                restore_called = true;
                Ok(())
            },
        );

        assert_eq!(result, Ok(()));
        assert!(!restore_called);
    }

    #[test]
    fn hidden_workspace_ui_region_size_uses_window_width_and_client_top() {
        let window = RECT {
            left: 10,
            top: 20,
            right: 210,
            bottom: 180,
        };
        let client_origin = ScreenPoint { x: 10, y: 50 };

        assert_eq!(
            hidden_workspace_ui_region_size(window, client_origin),
            Ok((200, 30 + TAB_BAR_HEIGHT))
        );
    }

    #[test]
    fn hidden_workspace_ui_region_size_reports_calculation_failures() {
        assert_eq!(
            hidden_workspace_ui_region_size(
                RECT {
                    left: 10,
                    top: 0,
                    right: 10,
                    bottom: 100,
                },
                ScreenPoint { x: 0, y: 20 },
            ),
            Err(Win32StatusFailure::new(
                "GetWindowRect",
                0,
                "작업 영역 UI window region 너비가 올바르지 않습니다.",
            ))
        );
        assert_eq!(
            hidden_workspace_ui_region_size(
                RECT {
                    left: 0,
                    top: 1,
                    right: 100,
                    bottom: 100,
                },
                ScreenPoint { x: 0, y: i32::MIN },
            ),
            Err(Win32StatusFailure::new(
                "ClientToScreen",
                0,
                "작업 영역 UI window region 상단 좌표를 계산할 수 없습니다.",
            ))
        );
    }

    #[test]
    fn tab_drag_threshold_distinguishes_click_from_drag() {
        let start = ClientPoint { x: 100, y: 12 };

        assert!(!tab_drag_exceeds_move_threshold(
            start,
            ClientPoint {
                x: 100 + TAB_DRAG_MOVE_THRESHOLD - 1,
                y: 12
            }
        ));
        assert!(tab_drag_exceeds_move_threshold(
            start,
            ClientPoint {
                x: 100 + TAB_DRAG_MOVE_THRESHOLD,
                y: 12
            }
        ));
    }

    #[test]
    fn pending_tab_release_switches_only_same_body_short_click() {
        let tab_id = TabId::new(2);
        let pending = PendingTabClick {
            tab_id,
            point: ClientPoint { x: 100, y: 12 },
            action: PendingTabAction::ClickOrReorder,
        };

        assert_eq!(
            pending_tab_release_switch_target(pending, Some(tab_id)),
            Some(tab_id)
        );
        assert_eq!(
            pending_tab_release_switch_target(pending, Some(TabId::new(3))),
            None
        );
        assert_eq!(pending_tab_release_switch_target(pending, None), None);
    }

    #[test]
    fn pending_tab_move_policy_distinguishes_click_reorder_and_window_move() {
        let tab_id = TabId::new(2);
        let start = ClientPoint { x: 100, y: 12 };
        let short_move = ClientPoint {
            x: 100 + TAB_DRAG_MOVE_THRESHOLD - 1,
            y: 12,
        };
        let threshold_move = ClientPoint {
            x: 100 + TAB_DRAG_MOVE_THRESHOLD,
            y: 12,
        };
        let visible_pending = PendingTabClick {
            tab_id,
            point: start,
            action: PendingTabAction::ClickOrReorder,
        };
        let hidden_pending = PendingTabClick {
            tab_id,
            point: start,
            action: PendingTabAction::ClickOrWindowMove,
        };

        assert_eq!(
            pending_tab_move_outcome(visible_pending, short_move),
            PendingTabMoveOutcome::ContinueClick
        );
        assert_eq!(
            pending_tab_move_outcome(visible_pending, threshold_move),
            PendingTabMoveOutcome::StartReorder(tab_id)
        );
        assert_eq!(
            pending_tab_move_outcome(hidden_pending, threshold_move),
            PendingTabMoveOutcome::StartWindowMove
        );
    }
}
