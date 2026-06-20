use core::ptr::null_mut;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::{
    ActivationPolicy, WindowControlError, WindowController, WindowOperation, WindowPositionRequest,
    WindowPositionResult,
};
use crate::domain::{
    DomainError, ExternalProgramSpec, Rect, WindowDisplayState, WindowHandle, WindowIdentity,
    WindowSnapshot, ZOrderHint,
};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HWND, RECT, SetLastError};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GA_ROOT, GW_HWNDPREV, GW_OWNER,
    GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetAncestor, GetPropW, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
    RemovePropW, SW_HIDE, SW_SHOW, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    SetPropW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
};

const USER_INVALID_WINDOW_MESSAGE: &str = "유효하지 않은 외부 윈도우입니다.";
const USER_WINDOW_ACCESS_MESSAGE: &str = "외부 윈도우 상태를 조회할 수 없습니다.";
const USER_PROGRAM_ACCESS_MESSAGE: &str = "외부 프로그램 정보를 조회할 수 없습니다.";
const USER_WINDOW_MOVE_MESSAGE: &str = "외부 윈도우 위치를 변경할 수 없습니다.";
const USER_WINDOW_RESTORE_MESSAGE: &str = "외부 윈도우를 복원할 수 없습니다.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZOrderPolicy {
    Preserve,
    RestoreAfter(HWND),
    ActiveDock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreOwnerPolicy {
    RestoreSnapshotOwner,
    ClearOwner,
}

#[derive(Debug, Clone, Copy)]
struct ZOrderApplication {
    insert_after: HWND,
    flags: u32,
    owner_rollback: Option<HWND>,
    repair_owner_after_move: bool,
}

#[derive(Debug, Clone, Copy)]
struct PendingActiveDockPosition {
    index: usize,
    raw: HWND,
    hwnd: WindowHandle,
    rect: Rect,
    insert_after: HWND,
    flags: u32,
    owner_rollback: Option<HWND>,
}

#[derive(Debug, Clone, Copy)]
struct RestoreAttemptRollback {
    owner: HWND,
    rect: Rect,
    style: Option<u32>,
    ex_style: Option<u32>,
}

static NEXT_OWNER_GUARD_TOKEN: AtomicUsize = AtomicUsize::new(1);
static NEXT_WINDOW_IDENTITY_TOKEN: AtomicUsize = AtomicUsize::new(1);
const WINDOW_PROPERTY_NAME_CAPACITY: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowPropertyName {
    buffer: [u16; WINDOW_PROPERTY_NAME_CAPACITY],
    len: usize,
}

impl WindowPropertyName {
    fn new(prefix: &str, hwnd: WindowHandle, token: usize) -> Self {
        let mut name = Self {
            buffer: [0; WINDOW_PROPERTY_NAME_CAPACITY],
            len: 0,
        };

        name.push_ascii_str(prefix);
        name.push_unsigned(std::process::id() as u128);
        name.push_unit(b'.' as u16);
        name.push_isize(hwnd.raw());
        name.push_unit(b'.' as u16);
        name.push_unsigned(token as u128);
        name.push_unit(0);
        name
    }

    fn as_ptr(&self) -> *const u16 {
        self.buffer.as_ptr()
    }

    #[cfg(test)]
    fn as_slice(&self) -> &[u16] {
        &self.buffer[..self.len]
    }

    fn push_ascii_str(&mut self, value: &str) {
        debug_assert!(value.is_ascii());
        for byte in value.as_bytes() {
            self.push_unit(u16::from(*byte));
        }
    }

    fn push_isize(&mut self, value: isize) {
        if value < 0 {
            self.push_unit(b'-' as u16);
            self.push_unsigned(value.wrapping_neg() as usize as u128);
        } else {
            self.push_unsigned(value as u128);
        }
    }

    fn push_unsigned(&mut self, mut value: u128) {
        let mut digits = [0u16; 39];
        let mut len = 0;

        loop {
            digits[len] = b'0' as u16 + (value % 10) as u16;
            len += 1;
            value /= 10;

            if value == 0 {
                break;
            }
        }

        for index in (0..len).rev() {
            self.push_unit(digits[index]);
        }
    }

    fn push_unit(&mut self, unit: u16) {
        if self.len < self.buffer.len() {
            self.buffer[self.len] = unit;
            self.len += 1;
        } else {
            debug_assert!(
                self.len < self.buffer.len(),
                "Win32 property name buffer is too small"
            );
        }
    }
}

#[derive(Debug, Default)]
pub struct Win32WindowController {
    excluded_owner: Option<WindowHandle>,
    snapshot_identity_guards: Vec<SnapshotIdentityGuard>,
    snapshot_owner_guards: Vec<SnapshotOwnerGuard>,
    #[cfg(test)]
    injected_set_prop_failure: Option<SetPropFailure>,
    #[cfg(test)]
    injected_set_window_style_failure: Option<i32>,
    #[cfg(test)]
    injected_owner_z_order_failure: Option<u32>,
}

impl Clone for Win32WindowController {
    fn clone(&self) -> Self {
        Self {
            excluded_owner: self.excluded_owner,
            snapshot_identity_guards: Vec::new(),
            snapshot_owner_guards: Vec::new(),
            #[cfg(test)]
            injected_set_prop_failure: None,
            #[cfg(test)]
            injected_set_window_style_failure: None,
            #[cfg(test)]
            injected_owner_z_order_failure: None,
        }
    }
}

impl Drop for Win32WindowController {
    fn drop(&mut self) {
        for guard in self.snapshot_identity_guards.drain(..) {
            remove_window_identity_property(guard.hwnd, guard.token);
        }
        for guard in self.snapshot_owner_guards.drain(..) {
            remove_owner_guard_property(guard.hwnd, guard.owner, guard.token);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotIdentityGuard {
    hwnd: WindowHandle,
    token: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotOwnerGuard {
    hwnd: WindowHandle,
    owner: WindowHandle,
    token: usize,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetPropFailure {
    Identity,
    Owner,
}

#[cfg(test)]
const INJECTED_SET_PROP_ERROR: u32 = 5;
#[cfg(test)]
const INJECTED_SET_WINDOW_STYLE_ERROR: u32 = 5;

impl Win32WindowController {
    pub const fn new() -> Self {
        Self {
            excluded_owner: None,
            snapshot_identity_guards: Vec::new(),
            snapshot_owner_guards: Vec::new(),
            #[cfg(test)]
            injected_set_prop_failure: None,
            #[cfg(test)]
            injected_set_window_style_failure: None,
            #[cfg(test)]
            injected_owner_z_order_failure: None,
        }
    }

    pub fn exclude_owner_window(&mut self, hwnd: WindowHandle) {
        self.excluded_owner = Some(hwnd);
    }

    pub fn program_spec_for_snapshot(
        &self,
        snapshot: &WindowSnapshot,
        title: Option<String>,
    ) -> Result<ExternalProgramSpec, WindowControlError> {
        let raw =
            self.ensure_snapshot_external_window(WindowOperation::InspectProgram, snapshot)?;
        let identity =
            self.window_identity(WindowOperation::InspectProgram, snapshot.hwnd(), raw)?;
        let executable_path = process_image_path(snapshot.hwnd(), identity.process_id())?;

        ExternalProgramSpec::new(executable_path, title).map_err(|error| {
            WindowControlError::new(
                WindowOperation::InspectProgram,
                Some(snapshot.hwnd()),
                USER_PROGRAM_ACCESS_MESSAGE,
                Some(format!("invalid external program spec: {error}")),
            )
        })
    }

    fn ensure_external_window(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
    ) -> Result<HWND, WindowControlError> {
        let raw = hwnd_to_raw(hwnd);

        if self.is_excluded_window(hwnd) {
            return Err(invalid_window_error(
                operation,
                hwnd,
                "HWND is registered as the j3GridDocker owner window.",
            ));
        }

        if !is_window(raw) {
            return Err(invalid_window_error(
                operation,
                hwnd,
                "IsWindow returned false.",
            ));
        }

        if !is_top_level_window(raw) {
            return Err(invalid_window_error(
                operation,
                hwnd,
                "GetAncestor(hwnd, GA_ROOT) did not return the original HWND.",
            ));
        }

        Ok(raw)
    }

    fn window_identity(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: HWND,
    ) -> Result<WindowIdentity, WindowControlError> {
        let mut process_id = 0;
        clear_last_error();
        // Callers pass a HWND that was already checked with IsWindow.
        let thread_id = unsafe { GetWindowThreadProcessId(raw, &mut process_id) };

        if thread_id == 0 || process_id == 0 {
            return Err(win32_error(
                operation,
                Some(hwnd),
                "GetWindowThreadProcessId",
                last_error(),
                USER_WINDOW_ACCESS_MESSAGE,
            ));
        }

        Ok(WindowIdentity::new(thread_id, process_id))
    }

    fn is_same_snapshot_window(
        &self,
        operation: WindowOperation,
        snapshot: &WindowSnapshot,
    ) -> Result<bool, WindowControlError> {
        Ok(self
            .same_snapshot_window_raw(operation, snapshot)?
            .is_some())
    }

    fn same_snapshot_window_raw(
        &self,
        operation: WindowOperation,
        snapshot: &WindowSnapshot,
    ) -> Result<Option<HWND>, WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = hwnd_to_raw(hwnd);

        if self.is_excluded_window(hwnd) || !is_window(raw) || !is_top_level_window(raw) {
            return Ok(None);
        }

        let Some(expected) = snapshot.identity() else {
            return Ok(Some(raw));
        };

        let actual = self.window_identity(operation, hwnd, raw)?;
        if window_identity_matches(hwnd, raw, expected, actual) {
            Ok(Some(raw))
        } else {
            Ok(None)
        }
    }

    fn ensure_snapshot_external_window(
        &self,
        operation: WindowOperation,
        snapshot: &WindowSnapshot,
    ) -> Result<HWND, WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_external_window(operation, hwnd)?;

        if let Some(expected) = snapshot.identity() {
            let actual = self.window_identity(operation, hwnd, raw)?;
            if !window_identity_matches(hwnd, raw, expected, actual) {
                return Err(invalid_window_error(
                    operation,
                    hwnd,
                    "HWND identity does not match the captured snapshot identity.",
                ));
            }
        }

        Ok(raw)
    }

    fn is_excluded_window(&self, hwnd: WindowHandle) -> bool {
        self.excluded_owner == Some(hwnd)
    }

    #[cfg(test)]
    fn take_injected_set_prop_failure(&mut self, failure: SetPropFailure) -> bool {
        if self.injected_set_prop_failure == Some(failure) {
            self.injected_set_prop_failure = None;
            true
        } else {
            false
        }
    }

    fn clear_snapshot_identity_guard(&mut self, hwnd: WindowHandle) {
        if let Some(index) = self
            .snapshot_identity_guards
            .iter()
            .position(|guard| guard.hwnd == hwnd)
        {
            let guard = self.snapshot_identity_guards.swap_remove(index);
            remove_window_identity_property(guard.hwnd, guard.token);
        }
    }

    fn record_snapshot_identity_guard(
        &mut self,
        hwnd: WindowHandle,
        raw: HWND,
    ) -> Result<usize, WindowControlError> {
        self.clear_snapshot_identity_guard(hwnd);

        let token = next_window_identity_token();
        #[cfg(test)]
        if self.take_injected_set_prop_failure(SetPropFailure::Identity) {
            return Err(win32_error(
                WindowOperation::Snapshot,
                Some(hwnd),
                "SetPropW(snapshot identity guard)",
                INJECTED_SET_PROP_ERROR,
                USER_WINDOW_ACCESS_MESSAGE,
            ));
        }
        set_window_identity_property(hwnd, raw, token)?;

        self.snapshot_identity_guards
            .push(SnapshotIdentityGuard { hwnd, token });
        Ok(token)
    }

    fn clear_snapshot_owner_guard(&mut self, hwnd: WindowHandle) {
        if let Some(index) = self
            .snapshot_owner_guards
            .iter()
            .position(|guard| guard.hwnd == hwnd)
        {
            let guard = self.snapshot_owner_guards.swap_remove(index);
            remove_owner_guard_property(guard.hwnd, guard.owner, guard.token);
        }
    }

    fn record_snapshot_owner_guard(
        &mut self,
        hwnd: WindowHandle,
        owner: WindowHandle,
        owner_raw: HWND,
    ) -> Result<(), WindowControlError> {
        self.clear_snapshot_owner_guard(hwnd);

        let token = next_owner_guard_token();
        #[cfg(test)]
        if self.take_injected_set_prop_failure(SetPropFailure::Owner) {
            return Err(win32_error(
                WindowOperation::Snapshot,
                Some(hwnd),
                "SetPropW(snapshot owner guard)",
                INJECTED_SET_PROP_ERROR,
                USER_WINDOW_ACCESS_MESSAGE,
            ));
        }
        set_owner_guard_property(hwnd, owner_raw, token)?;

        self.snapshot_owner_guards
            .push(SnapshotOwnerGuard { hwnd, owner, token });
        Ok(())
    }

    fn window_rect(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        operation: WindowOperation,
        user_message: &'static str,
    ) -> Result<Rect, WindowControlError> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };

        clear_last_error();
        let ok = unsafe { GetWindowRect(raw, &mut rect) };
        if ok == 0 {
            let error = last_error();
            return Err(win32_error(
                operation,
                Some(hwnd),
                "GetWindowRect",
                error,
                user_message,
            ));
        }

        win32_rect_to_domain(hwnd, rect)
    }

    fn snapshot_rect(&self, raw: HWND, hwnd: WindowHandle) -> Result<Rect, WindowControlError> {
        self.window_rect(
            raw,
            hwnd,
            WindowOperation::Snapshot,
            USER_WINDOW_ACCESS_MESSAGE,
        )
    }

    fn window_style(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        index: i32,
        api: &'static str,
    ) -> Result<u32, WindowControlError> {
        self.window_style_for_operation(
            raw,
            hwnd,
            index,
            api,
            WindowOperation::Snapshot,
            USER_WINDOW_ACCESS_MESSAGE,
        )
    }

    fn window_style_for_operation(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        index: i32,
        api: &'static str,
        operation: WindowOperation,
        user_message: &'static str,
    ) -> Result<u32, WindowControlError> {
        clear_last_error();
        let value = unsafe { GetWindowLongPtrW(raw, index) };
        let error = last_error();

        if value == 0 && error != 0 {
            Err(win32_error(operation, Some(hwnd), api, error, user_message))
        } else {
            Ok(value as u32)
        }
    }

    fn restore_styles(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        snapshot: &WindowSnapshot,
    ) -> Result<(), WindowControlError> {
        let mut style_changed = false;

        if let Some(style) = snapshot.style() {
            self.set_window_style(raw, hwnd, GWL_STYLE, style, "SetWindowLongPtrW(GWL_STYLE)")?;
            style_changed = true;
        }

        if let Some(ex_style) = snapshot.ex_style() {
            self.set_window_style(
                raw,
                hwnd,
                GWL_EXSTYLE,
                ex_style,
                "SetWindowLongPtrW(GWL_EXSTYLE)",
            )?;
            style_changed = true;
        }

        if style_changed {
            self.apply_frame_changed(raw, hwnd)?;
        }

        Ok(())
    }

    fn apply_active_dock_owner(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
    ) -> Result<Option<HWND>, WindowControlError> {
        let Some(owner) = self.excluded_owner else {
            return Ok(None);
        };
        let owner_raw = hwnd_to_raw(owner);
        let previous_owner = unsafe { GetWindow(raw, GW_OWNER) };

        if previous_owner == owner_raw {
            return Ok(None);
        }

        self.set_window_owner(raw, hwnd, Some(owner_raw), WindowOperation::SetPosition)?;
        Ok(Some(previous_owner))
    }

    fn restore_owner_raw_best_effort(&self, raw: HWND, hwnd: WindowHandle, owner: HWND) {
        let owner = if owner.is_null() { None } else { Some(owner) };
        let _ = self.set_window_owner(raw, hwnd, owner, WindowOperation::Restore);
    }

    fn sync_active_dock_owner_z_order(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
    ) -> Result<(), WindowControlError> {
        let Some(owner) = self.excluded_owner else {
            return Ok(());
        };
        let owner_raw = hwnd_to_raw(owner);

        if owner_raw.is_null() || !is_window(owner_raw) {
            return Err(invalid_window_error(
                WindowOperation::SetPosition,
                hwnd,
                "Registered j3GridDocker owner window is no longer valid.",
            ));
        }

        #[cfg(test)]
        if let Some(error) = self.injected_owner_z_order_failure {
            return Err(win32_error(
                WindowOperation::SetPosition,
                Some(hwnd),
                "SetWindowPos(owner z-order)",
                error,
                USER_WINDOW_MOVE_MESSAGE,
            ));
        }

        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        clear_last_error();
        let ok = unsafe { SetWindowPos(owner_raw, raw, 0, 0, 0, 0, flags) };

        if ok == 0 {
            let error = last_error();
            Err(win32_error(
                WindowOperation::SetPosition,
                Some(hwnd),
                "SetWindowPos(owner z-order)",
                error,
                USER_WINDOW_MOVE_MESSAGE,
            ))
        } else {
            Ok(())
        }
    }

    fn z_order_application(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        z_order: ZOrderPolicy,
    ) -> Result<ZOrderApplication, WindowControlError> {
        let mut application = match z_order {
            ZOrderPolicy::Preserve => ZOrderApplication {
                insert_after: null_mut(),
                flags: SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
                owner_rollback: None,
                repair_owner_after_move: false,
            },
            ZOrderPolicy::RestoreAfter(insert_after) => ZOrderApplication {
                insert_after,
                flags: SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                owner_rollback: None,
                repair_owner_after_move: false,
            },
            ZOrderPolicy::ActiveDock => {
                // HWND_TOP is encoded as a null HWND. Without SWP_NOZORDER this
                // raises the owned dock window without making it topmost.
                ZOrderApplication {
                    insert_after: null_mut(),
                    flags: SWP_NOACTIVATE,
                    owner_rollback: None,
                    repair_owner_after_move: true,
                }
            }
        };

        if z_order == ZOrderPolicy::ActiveDock {
            application.owner_rollback = self.apply_active_dock_owner(raw, hwnd)?;
        }

        Ok(application)
    }

    fn complete_z_order_application(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        application: ZOrderApplication,
    ) -> Result<(), WindowControlError> {
        if application.repair_owner_after_move {
            self.sync_active_dock_owner_z_order(raw, hwnd)?;
        }

        Ok(())
    }

    fn handle_z_order_completion_result(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        application: ZOrderApplication,
        result: Result<(), WindowControlError>,
    ) -> Result<(), WindowControlError> {
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Some(previous_owner) = application.owner_rollback {
                    self.restore_owner_raw_best_effort(raw, hwnd, previous_owner);
                }
                Err(error)
            }
        }
    }

    fn restore_owner(
        &mut self,
        raw: HWND,
        hwnd: WindowHandle,
        snapshot: &WindowSnapshot,
        policy: RestoreOwnerPolicy,
    ) -> Result<(), WindowControlError> {
        let owner_raw = match policy {
            RestoreOwnerPolicy::RestoreSnapshotOwner => {
                self.validated_restore_owner(raw, hwnd, snapshot.owner())
            }
            RestoreOwnerPolicy::ClearOwner => None,
        };
        self.set_window_owner(raw, hwnd, owner_raw, WindowOperation::Restore)
    }

    fn validated_restore_owner(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        owner: Option<WindowHandle>,
    ) -> Option<HWND> {
        let owner = owner?;
        let owner_raw = hwnd_to_raw(owner);

        if !self.is_valid_restore_owner_target(raw, hwnd, owner, owner_raw) {
            return None;
        }

        let guard = self
            .snapshot_owner_guards
            .iter()
            .find(|guard| guard.hwnd == hwnd && guard.owner == owner)?;

        if owner_guard_property_matches(hwnd, owner_raw, guard.token) {
            Some(owner_raw)
        } else {
            None
        }
    }

    fn is_valid_restore_owner_target(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        owner: WindowHandle,
        owner_raw: HWND,
    ) -> bool {
        if owner_raw.is_null() || owner == hwnd || self.is_excluded_window(owner) {
            return false;
        }

        if !is_window(owner_raw) || !is_top_level_window(owner_raw) {
            return false;
        }

        unsafe { GetWindow(owner_raw, GW_OWNER) != raw }
    }

    fn set_window_owner(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        owner: Option<HWND>,
        operation: WindowOperation,
    ) -> Result<(), WindowControlError> {
        let owner = owner.unwrap_or_else(null_mut);

        if unsafe { GetWindow(raw, GW_OWNER) } == owner {
            return Ok(());
        }

        clear_last_error();
        let previous = unsafe { SetWindowLongPtrW(raw, GWLP_HWNDPARENT, owner as isize) };
        let error = last_error();

        if previous == 0 && error != 0 {
            let user_message = match operation {
                WindowOperation::Restore => USER_WINDOW_RESTORE_MESSAGE,
                _ => USER_WINDOW_MOVE_MESSAGE,
            };
            Err(win32_error(
                operation,
                Some(hwnd),
                "SetWindowLongPtrW(GWLP_HWNDPARENT)",
                error,
                user_message,
            ))
        } else {
            Ok(())
        }
    }

    fn set_window_style(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        index: i32,
        value: u32,
        api: &'static str,
    ) -> Result<(), WindowControlError> {
        #[cfg(test)]
        if self.injected_set_window_style_failure == Some(index) {
            return Err(win32_error(
                WindowOperation::Restore,
                Some(hwnd),
                api,
                INJECTED_SET_WINDOW_STYLE_ERROR,
                USER_WINDOW_RESTORE_MESSAGE,
            ));
        }

        clear_last_error();
        let previous = unsafe { SetWindowLongPtrW(raw, index, value as isize) };
        let error = last_error();

        if previous == 0 && error != 0 {
            Err(win32_error(
                WindowOperation::Restore,
                Some(hwnd),
                api,
                error,
                USER_WINDOW_RESTORE_MESSAGE,
            ))
        } else {
            Ok(())
        }
    }

    fn apply_frame_changed(&self, raw: HWND, hwnd: WindowHandle) -> Result<(), WindowControlError> {
        let flags = SWP_NOMOVE
            | SWP_NOSIZE
            | SWP_NOZORDER
            | SWP_NOOWNERZORDER
            | SWP_NOACTIVATE
            | SWP_FRAMECHANGED;
        clear_last_error();
        let ok = unsafe { SetWindowPos(raw, null_mut(), 0, 0, 0, 0, flags) };

        if ok == 0 {
            let error = last_error();
            Err(win32_error(
                WindowOperation::Restore,
                Some(hwnd),
                "SetWindowPos(SWP_FRAMECHANGED)",
                error,
                USER_WINDOW_RESTORE_MESSAGE,
            ))
        } else {
            Ok(())
        }
    }

    fn capture_restore_attempt_rollback(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        snapshot: &WindowSnapshot,
    ) -> Result<RestoreAttemptRollback, WindowControlError> {
        let owner = unsafe { GetWindow(raw, GW_OWNER) };
        let rect = self.window_rect(
            raw,
            hwnd,
            WindowOperation::Restore,
            USER_WINDOW_RESTORE_MESSAGE,
        )?;
        let style = if snapshot.style().is_some() {
            Some(self.window_style_for_operation(
                raw,
                hwnd,
                GWL_STYLE,
                "GetWindowLongPtrW(GWL_STYLE)",
                WindowOperation::Restore,
                USER_WINDOW_RESTORE_MESSAGE,
            )?)
        } else {
            None
        };
        let ex_style = if snapshot.ex_style().is_some() {
            Some(self.window_style_for_operation(
                raw,
                hwnd,
                GWL_EXSTYLE,
                "GetWindowLongPtrW(GWL_EXSTYLE)",
                WindowOperation::Restore,
                USER_WINDOW_RESTORE_MESSAGE,
            )?)
        } else {
            None
        };

        Ok(RestoreAttemptRollback {
            owner,
            rect,
            style,
            ex_style,
        })
    }

    fn rollback_restore_attempt(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        rollback: RestoreAttemptRollback,
    ) {
        self.restore_owner_raw_best_effort(raw, hwnd, rollback.owner);

        let mut style_changed = false;
        if let Some(style) = rollback.style {
            style_changed |= self
                .set_window_style(raw, hwnd, GWL_STYLE, style, "SetWindowLongPtrW(GWL_STYLE)")
                .is_ok();
        }
        if let Some(ex_style) = rollback.ex_style {
            style_changed |= self
                .set_window_style(
                    raw,
                    hwnd,
                    GWL_EXSTYLE,
                    ex_style,
                    "SetWindowLongPtrW(GWL_EXSTYLE)",
                )
                .is_ok();
        }
        if style_changed {
            let _ = self.apply_frame_changed(raw, hwnd);
        }

        let _ = self.set_position_raw(
            raw,
            hwnd,
            rollback.rect,
            WindowOperation::Restore,
            ZOrderPolicy::Preserve,
        );
    }

    fn set_position_raw(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        rect: Rect,
        operation: WindowOperation,
        z_order: ZOrderPolicy,
    ) -> Result<(), WindowControlError> {
        self.set_position_raw_with(
            raw,
            hwnd,
            rect,
            operation,
            z_order,
            |raw, insert_after, x, y, width, height, flags| unsafe {
                SetWindowPos(raw, insert_after, x, y, width, height, flags)
            },
        )
    }

    fn set_position_raw_with<F>(
        &self,
        raw: HWND,
        hwnd: WindowHandle,
        rect: Rect,
        operation: WindowOperation,
        z_order: ZOrderPolicy,
        set_window_pos: F,
    ) -> Result<(), WindowControlError>
    where
        F: FnOnce(HWND, HWND, i32, i32, i32, i32, u32) -> i32,
    {
        let z_order_application = self.z_order_application(raw, hwnd, z_order)?;

        clear_last_error();
        let ok = set_window_pos(
            raw,
            z_order_application.insert_after,
            rect.left(),
            rect.top(),
            rect.width(),
            rect.height(),
            z_order_application.flags,
        );

        if ok == 0 {
            let error = last_error();
            if let Some(previous_owner) = z_order_application.owner_rollback {
                self.restore_owner_raw_best_effort(raw, hwnd, previous_owner);
            }
            Err(win32_error(
                operation,
                Some(hwnd),
                "SetWindowPos",
                error,
                USER_WINDOW_MOVE_MESSAGE,
            ))
        } else {
            let result = self.complete_z_order_application(raw, hwnd, z_order_application);
            self.handle_z_order_completion_result(raw, hwnd, z_order_application, result)
        }
    }

    fn set_positions_individually_if_same_external_windows<'a, I>(
        &mut self,
        positions: I,
    ) -> Vec<WindowPositionResult>
    where
        I: IntoIterator<Item = WindowPositionRequest<'a>>,
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

    fn set_active_dock_positions_with_shared_owner_repair<'a, I>(
        &mut self,
        positions: I,
    ) -> Vec<WindowPositionResult>
    where
        I: Clone + IntoIterator<Item = WindowPositionRequest<'a>>,
    {
        let mut probe = positions.clone().into_iter();
        if self.excluded_owner.is_none() || probe.next().is_none() || probe.next().is_none() {
            return self.set_positions_individually_if_same_external_windows(positions);
        }

        let fallback_positions = positions.clone();
        let positions = positions.into_iter();
        let (lower_bound, upper_bound) = positions.size_hint();
        let capacity = upper_bound.unwrap_or(lower_bound);
        let mut results = Vec::with_capacity(capacity);
        let mut pending = Vec::with_capacity(capacity);

        for (index, position) in positions.enumerate() {
            let snapshot = position.snapshot();
            let hwnd = snapshot.hwnd();
            let raw = match self.same_snapshot_window_raw(WindowOperation::Validate, snapshot) {
                Ok(Some(raw)) => raw,
                Ok(None) => {
                    results.push(WindowPositionResult::Stale);
                    continue;
                }
                Err(error) => {
                    results.push(WindowPositionResult::Failed(error));
                    continue;
                }
            };

            let z_order_application =
                match self.z_order_application(raw, hwnd, ZOrderPolicy::ActiveDock) {
                    Ok(application) => application,
                    Err(error) => {
                        results.push(WindowPositionResult::Failed(error));
                        continue;
                    }
                };

            results.push(WindowPositionResult::Stale);
            pending.push(PendingActiveDockPosition {
                index,
                raw,
                hwnd,
                rect: position.rect(),
                insert_after: z_order_application.insert_after,
                flags: z_order_application.flags,
                owner_rollback: z_order_application.owner_rollback,
            });
        }

        let Ok(defer_count) = i32::try_from(pending.len()) else {
            self.rollback_pending_active_dock_owner_repairs(&pending);
            return self.set_positions_individually_if_same_external_windows(fallback_positions);
        };
        if defer_count == 0 {
            return results;
        }

        clear_last_error();
        let mut defer = unsafe { BeginDeferWindowPos(defer_count) };
        if defer.is_null() {
            self.rollback_pending_active_dock_owner_repairs(&pending);
            return self.set_positions_individually_if_same_external_windows(fallback_positions);
        }

        for position in &pending {
            clear_last_error();
            let next_defer = unsafe {
                DeferWindowPos(
                    defer,
                    position.raw,
                    position.insert_after,
                    position.rect.left(),
                    position.rect.top(),
                    position.rect.width(),
                    position.rect.height(),
                    position.flags,
                )
            };
            if next_defer.is_null() {
                self.rollback_pending_active_dock_owner_repairs(&pending);
                return self
                    .set_positions_individually_if_same_external_windows(fallback_positions);
            }

            defer = next_defer;
        }

        clear_last_error();
        let batch_ok = unsafe { EndDeferWindowPos(defer) };
        if batch_ok == 0 {
            self.rollback_pending_active_dock_owner_repairs(&pending);
            return self.set_positions_individually_if_same_external_windows(fallback_positions);
        }

        let Some(anchor) = pending.first().copied() else {
            return results;
        };

        match self.sync_active_dock_owner_z_order(anchor.raw, anchor.hwnd) {
            Ok(()) => {
                for position in &pending {
                    results[position.index] = WindowPositionResult::Positioned;
                }
            }
            Err(error) => {
                self.rollback_pending_active_dock_owner_repairs(&pending);
                for position in &pending {
                    results[position.index] = WindowPositionResult::Failed(error.clone());
                }
            }
        }

        results
    }

    fn rollback_pending_active_dock_owner_repairs(&self, pending: &[PendingActiveDockPosition]) {
        for position in pending {
            if let Some(previous_owner) = position.owner_rollback {
                self.restore_owner_raw_best_effort(position.raw, position.hwnd, previous_owner);
            }
        }
    }

    fn restore_z_order_policy(&self, raw: HWND, snapshot: &WindowSnapshot) -> ZOrderPolicy {
        let Some(z_order_hint) = snapshot.z_order_hint() else {
            return ZOrderPolicy::Preserve;
        };

        let insert_after = z_order_hint.value() as HWND;
        if is_valid_z_order_insert_after(raw, insert_after) {
            ZOrderPolicy::RestoreAfter(insert_after)
        } else {
            ZOrderPolicy::Preserve
        }
    }

    fn show_raw(&self, raw: HWND, command: i32) {
        // ShowWindow returns the previous visibility state, not a success flag.
        unsafe {
            ShowWindow(raw, command);
        }
    }

    fn snapshot_with<R, S>(
        &mut self,
        hwnd: WindowHandle,
        snapshot_rect: R,
        mut window_style: S,
    ) -> Result<WindowSnapshot, WindowControlError>
    where
        R: FnOnce(&Self, HWND, WindowHandle) -> Result<Rect, WindowControlError>,
        S: FnMut(&Self, HWND, WindowHandle, i32, &'static str) -> Result<u32, WindowControlError>,
    {
        let raw = self.ensure_external_window(WindowOperation::Snapshot, hwnd)?;
        let mut identity = self.window_identity(WindowOperation::Snapshot, hwnd, raw)?;
        let token = self.record_snapshot_identity_guard(hwnd, raw)?;
        identity = identity.with_validation_token(token);

        let result = (|| {
            self.clear_snapshot_owner_guard(hwnd);
            let rect = snapshot_rect(self, raw, hwnd)?;
            let display_state = display_state(raw);
            let style = window_style(self, raw, hwnd, GWL_STYLE, "GetWindowLongPtrW(GWL_STYLE)")?;
            let ex_style = window_style(
                self,
                raw,
                hwnd,
                GWL_EXSTYLE,
                "GetWindowLongPtrW(GWL_EXSTYLE)",
            )?;

            let mut snapshot = WindowSnapshot::new(hwnd, rect, display_state)
                .with_identity(identity)
                .with_style(style)
                .with_ex_style(ex_style);

            if let Some(owner) = owner_window(raw) {
                let owner_raw = hwnd_to_raw(owner);
                if self.is_valid_restore_owner_target(raw, hwnd, owner, owner_raw) {
                    self.record_snapshot_owner_guard(hwnd, owner, owner_raw)?;
                    snapshot = snapshot.with_owner(owner);
                }
            }

            if let Some(z_order_hint) = z_order_hint(raw) {
                snapshot = snapshot.with_z_order_hint(z_order_hint);
            }

            Ok(snapshot)
        })();

        if result.is_err() {
            self.clear_snapshot_identity_guard(hwnd);
        }

        result
    }

    fn restore_with<F>(
        &mut self,
        snapshot: &WindowSnapshot,
        set_window_pos: F,
    ) -> Result<(), WindowControlError>
    where
        F: FnOnce(HWND, HWND, i32, i32, i32, i32, u32) -> i32,
    {
        self.restore_with_owner_policy(
            snapshot,
            RestoreOwnerPolicy::RestoreSnapshotOwner,
            set_window_pos,
        )
    }

    fn restore_detached_with<F>(
        &mut self,
        snapshot: &WindowSnapshot,
        set_window_pos: F,
    ) -> Result<(), WindowControlError>
    where
        F: FnOnce(HWND, HWND, i32, i32, i32, i32, u32) -> i32,
    {
        self.restore_with_owner_policy(snapshot, RestoreOwnerPolicy::ClearOwner, set_window_pos)
    }

    fn restore_with_owner_policy<F>(
        &mut self,
        snapshot: &WindowSnapshot,
        owner_policy: RestoreOwnerPolicy,
        set_window_pos: F,
    ) -> Result<(), WindowControlError>
    where
        F: FnOnce(HWND, HWND, i32, i32, i32, i32, u32) -> i32,
    {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_snapshot_external_window(WindowOperation::Restore, snapshot)?;
        let rollback = self.capture_restore_attempt_rollback(raw, hwnd, snapshot)?;

        let mut rollback_needed = false;
        let result = (|| {
            self.restore_owner(raw, hwnd, snapshot, owner_policy)?;
            rollback_needed = true;
            self.restore_styles(raw, hwnd, snapshot)?;
            self.set_position_raw_with(
                raw,
                hwnd,
                snapshot.rect(),
                WindowOperation::Restore,
                self.restore_z_order_policy(raw, snapshot),
                set_window_pos,
            )?;
            self.show_raw(
                raw,
                show_command_for_display_state(snapshot.display_state()),
            );

            Ok(())
        })();

        if let Err(error) = result {
            if rollback_needed {
                self.rollback_restore_attempt(raw, hwnd, rollback);
            }
            return Err(error);
        }

        self.clear_snapshot_identity_guard(hwnd);
        self.clear_snapshot_owner_guard(hwnd);

        Ok(())
    }
}

impl WindowController for Win32WindowController {
    fn is_valid_external_window(&mut self, hwnd: WindowHandle) -> Result<bool, WindowControlError> {
        let raw = hwnd_to_raw(hwnd);
        Ok(!self.is_excluded_window(hwnd) && is_window(raw) && is_top_level_window(raw))
    }

    fn is_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
    ) -> Result<bool, WindowControlError> {
        self.is_same_snapshot_window(WindowOperation::Validate, snapshot)
    }

    fn snapshot(&mut self, hwnd: WindowHandle) -> Result<WindowSnapshot, WindowControlError> {
        self.snapshot_with(hwnd, Self::snapshot_rect, Self::window_style)
    }

    fn hide(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        let raw = self.ensure_snapshot_external_window(WindowOperation::Hide, snapshot)?;
        self.show_raw(raw, SW_HIDE);
        Ok(())
    }

    fn show(
        &mut self,
        snapshot: &WindowSnapshot,
        activation: ActivationPolicy,
    ) -> Result<(), WindowControlError> {
        let raw = self.ensure_snapshot_external_window(WindowOperation::Show, snapshot)?;
        self.show_raw(raw, show_command_for_activation(activation));
        Ok(())
    }

    fn set_position(
        &mut self,
        snapshot: &WindowSnapshot,
        rect: Rect,
    ) -> Result<(), WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_snapshot_external_window(WindowOperation::SetPosition, snapshot)?;
        self.set_position_raw(
            raw,
            hwnd,
            rect,
            WindowOperation::SetPosition,
            ZOrderPolicy::ActiveDock,
        )
    }

    fn set_position_if_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
        rect: Rect,
    ) -> Result<bool, WindowControlError> {
        let Some(raw) = self.same_snapshot_window_raw(WindowOperation::Validate, snapshot)? else {
            return Ok(false);
        };
        let hwnd = snapshot.hwnd();

        match self.set_position_raw(
            raw,
            hwnd,
            rect,
            WindowOperation::SetPosition,
            ZOrderPolicy::ActiveDock,
        ) {
            Ok(()) => Ok(true),
            Err(error) => match self.is_same_snapshot_window(WindowOperation::Validate, snapshot) {
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
        self.set_active_dock_positions_with_shared_owner_repair(positions)
    }

    fn restore(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        self.restore_with(
            snapshot,
            |raw, insert_after, x, y, width, height, flags| unsafe {
                SetWindowPos(raw, insert_after, x, y, width, height, flags)
            },
        )
    }

    fn restore_detached(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        self.restore_detached_with(
            snapshot,
            |raw, insert_after, x, y, width, height, flags| unsafe {
                SetWindowPos(raw, insert_after, x, y, width, height, flags)
            },
        )
    }
}

fn hwnd_to_raw(hwnd: WindowHandle) -> HWND {
    hwnd.raw() as HWND
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn process_image_path(hwnd: WindowHandle, process_id: u32) -> Result<String, WindowControlError> {
    clear_last_error();
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(win32_error(
            WindowOperation::InspectProgram,
            Some(hwnd),
            "OpenProcess",
            last_error(),
            USER_PROGRAM_ACCESS_MESSAGE,
        ));
    }
    let _process = OwnedHandle(process);

    const ERROR_INSUFFICIENT_BUFFER_CODE: u32 = 122;
    const INITIAL_PATH_BUFFER_LEN: u32 = 260;
    const MAX_PATH_BUFFER_LEN: u32 = 32_768;

    let mut buffer_len = INITIAL_PATH_BUFFER_LEN;
    let mut buffer = vec![0u16; buffer_len as usize];

    loop {
        let mut length = buffer_len;
        clear_last_error();
        let ok =
            unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) };
        if ok != 0 {
            let length = usize::try_from(length).map_err(|_| {
                WindowControlError::new(
                    WindowOperation::InspectProgram,
                    Some(hwnd),
                    USER_PROGRAM_ACCESS_MESSAGE,
                    Some(String::from(
                        "QueryFullProcessImageNameW returned a path length that does not fit usize.",
                    )),
                )
            })?;
            buffer.truncate(length);

            return Ok(String::from_utf16_lossy(&buffer));
        }

        let error = last_error();
        if error != ERROR_INSUFFICIENT_BUFFER_CODE || buffer_len >= MAX_PATH_BUFFER_LEN {
            return Err(win32_error(
                WindowOperation::InspectProgram,
                Some(hwnd),
                "QueryFullProcessImageNameW",
                error,
                USER_PROGRAM_ACCESS_MESSAGE,
            ));
        }

        buffer_len = buffer_len.saturating_mul(2).min(MAX_PATH_BUFFER_LEN);
        buffer.resize(buffer_len as usize, 0);
    }
}

fn next_owner_guard_token() -> usize {
    NEXT_OWNER_GUARD_TOKEN
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            let next = current.wrapping_add(1);
            Some(if next == 0 { 1 } else { next })
        })
        .unwrap_or(1)
}

fn next_window_identity_token() -> usize {
    NEXT_WINDOW_IDENTITY_TOKEN
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            let next = current.wrapping_add(1);
            Some(if next == 0 { 1 } else { next })
        })
        .unwrap_or(1)
}

fn window_identity_matches(
    hwnd: WindowHandle,
    raw: HWND,
    expected: WindowIdentity,
    actual: WindowIdentity,
) -> bool {
    if expected.thread_id() != actual.thread_id() || expected.process_id() != actual.process_id() {
        return false;
    }

    match expected.validation_token() {
        Some(token) => window_identity_property_matches(hwnd, raw, token),
        None => true,
    }
}

fn set_window_identity_property(
    hwnd: WindowHandle,
    raw: HWND,
    token: usize,
) -> Result<(), WindowControlError> {
    let property_name = window_identity_property_name(hwnd, token);
    // The HANDLE value is a non-null process-local token. Win32 stores it
    // in the target window property table and never dereferences it.
    clear_last_error();
    let ok = unsafe { SetPropW(raw, property_name.as_ptr(), token_to_handle(token)) };

    if ok == 0 {
        Err(win32_error(
            WindowOperation::Snapshot,
            Some(hwnd),
            "SetPropW(snapshot identity guard)",
            last_error(),
            USER_WINDOW_ACCESS_MESSAGE,
        ))
    } else {
        Ok(())
    }
}

fn window_identity_property_matches(hwnd: WindowHandle, raw: HWND, token: usize) -> bool {
    let property_name = window_identity_property_name(hwnd, token);
    unsafe { GetPropW(raw, property_name.as_ptr()) == token_to_handle(token) }
}

fn remove_window_identity_property(hwnd: WindowHandle, token: usize) {
    let raw = hwnd_to_raw(hwnd);
    if !is_window(raw) {
        return;
    }

    let property_name = window_identity_property_name(hwnd, token);
    unsafe {
        RemovePropW(raw, property_name.as_ptr());
    }
}

fn window_identity_property_name(hwnd: WindowHandle, token: usize) -> WindowPropertyName {
    WindowPropertyName::new("j3GridDocker.WindowIdentity.", hwnd, token)
}

fn set_owner_guard_property(
    hwnd: WindowHandle,
    owner_raw: HWND,
    token: usize,
) -> Result<(), WindowControlError> {
    let property_name = owner_guard_property_name(hwnd, token);
    // The HANDLE value is a non-null process-local token. Win32 stores it
    // in the owner window property table and never dereferences it.
    clear_last_error();
    let ok = unsafe { SetPropW(owner_raw, property_name.as_ptr(), token_to_handle(token)) };

    if ok == 0 {
        Err(win32_error(
            WindowOperation::Snapshot,
            Some(hwnd),
            "SetPropW(snapshot owner guard)",
            last_error(),
            USER_WINDOW_ACCESS_MESSAGE,
        ))
    } else {
        Ok(())
    }
}

fn owner_guard_property_matches(hwnd: WindowHandle, owner_raw: HWND, token: usize) -> bool {
    let property_name = owner_guard_property_name(hwnd, token);
    unsafe { GetPropW(owner_raw, property_name.as_ptr()) == token_to_handle(token) }
}

fn remove_owner_guard_property(hwnd: WindowHandle, owner: WindowHandle, token: usize) {
    let owner_raw = hwnd_to_raw(owner);
    if !is_window(owner_raw) {
        return;
    }

    let property_name = owner_guard_property_name(hwnd, token);
    unsafe {
        RemovePropW(owner_raw, property_name.as_ptr());
    }
}

fn owner_guard_property_name(hwnd: WindowHandle, token: usize) -> WindowPropertyName {
    WindowPropertyName::new("j3GridDocker.OwnerGuard.", hwnd, token)
}

fn token_to_handle(token: usize) -> HANDLE {
    token as HANDLE
}

fn is_window(hwnd: HWND) -> bool {
    unsafe { IsWindow(hwnd) != 0 }
}

fn is_top_level_window(hwnd: HWND) -> bool {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    root == hwnd
}

fn display_state(hwnd: HWND) -> WindowDisplayState {
    if unsafe { IsWindowVisible(hwnd) == 0 } {
        WindowDisplayState::Hidden
    } else if unsafe { IsIconic(hwnd) != 0 } {
        WindowDisplayState::Minimized
    } else if unsafe { IsZoomed(hwnd) != 0 } {
        WindowDisplayState::Maximized
    } else {
        WindowDisplayState::Normal
    }
}

fn z_order_hint(hwnd: HWND) -> Option<ZOrderHint> {
    let previous = unsafe { GetWindow(hwnd, GW_HWNDPREV) };

    if previous.is_null() {
        None
    } else {
        Some(ZOrderHint::new(previous as isize))
    }
}

fn is_valid_z_order_insert_after(raw: HWND, insert_after: HWND) -> bool {
    !insert_after.is_null()
        && insert_after != raw
        && is_window(insert_after)
        && is_top_level_window(insert_after)
}

fn owner_window(hwnd: HWND) -> Option<WindowHandle> {
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };

    if owner.is_null() {
        None
    } else {
        WindowHandle::new(owner as isize).ok()
    }
}

fn show_command_for_activation(activation: ActivationPolicy) -> i32 {
    match activation {
        ActivationPolicy::NoActivate => SW_SHOWNOACTIVATE,
        ActivationPolicy::Activate => SW_SHOW,
    }
}

fn show_command_for_display_state(display_state: WindowDisplayState) -> i32 {
    match display_state {
        WindowDisplayState::Hidden => SW_HIDE,
        WindowDisplayState::Normal => SW_SHOWNOACTIVATE,
        WindowDisplayState::Minimized => SW_SHOWMINNOACTIVE,
        WindowDisplayState::Maximized => SW_SHOWMAXIMIZED,
    }
}

fn win32_rect_to_domain(hwnd: WindowHandle, rect: RECT) -> Result<Rect, WindowControlError> {
    let width = rect.right.checked_sub(rect.left).ok_or_else(|| {
        domain_error(
            WindowOperation::Snapshot,
            hwnd,
            DomainError::CoordinateOverflow,
        )
    })?;
    let height = rect.bottom.checked_sub(rect.top).ok_or_else(|| {
        domain_error(
            WindowOperation::Snapshot,
            hwnd,
            DomainError::CoordinateOverflow,
        )
    })?;

    Rect::new(rect.left, rect.top, width, height)
        .map_err(|error| domain_error(WindowOperation::Snapshot, hwnd, error))
}

fn invalid_window_error(
    operation: WindowOperation,
    hwnd: WindowHandle,
    detail: &'static str,
) -> WindowControlError {
    WindowControlError::new(
        operation,
        Some(hwnd),
        USER_INVALID_WINDOW_MESSAGE,
        Some(detail.to_owned()),
    )
}

fn domain_error(
    operation: WindowOperation,
    hwnd: WindowHandle,
    error: DomainError,
) -> WindowControlError {
    WindowControlError::new(
        operation,
        Some(hwnd),
        error.user_message(),
        Some(error.to_string()),
    )
}

fn win32_error(
    operation: WindowOperation,
    hwnd: Option<WindowHandle>,
    api: &'static str,
    last_error: u32,
    user_message: &'static str,
) -> WindowControlError {
    WindowControlError::from_win32(operation, hwnd, api, last_error, user_message)
}

fn clear_last_error() {
    unsafe {
        SetLastError(0);
    }
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io;
    use std::iter::once;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::app::App;
    use windows_sys::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetParent,
        RegisterClassW, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MINIMIZEBOX,
        WS_OVERLAPPEDWINDOW,
    };

    static NEXT_CLASS_ID: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn activation_policy_maps_to_documented_show_commands() {
        assert_eq!(
            show_command_for_activation(ActivationPolicy::NoActivate),
            SW_SHOWNOACTIVATE
        );
        assert_eq!(
            show_command_for_activation(ActivationPolicy::Activate),
            SW_SHOW
        );
    }

    #[test]
    fn display_state_maps_to_no_activate_restore_commands() {
        assert_eq!(
            show_command_for_display_state(WindowDisplayState::Normal),
            SW_SHOWNOACTIVATE
        );
        assert_eq!(
            show_command_for_display_state(WindowDisplayState::Hidden),
            SW_HIDE
        );
        assert_eq!(
            show_command_for_display_state(WindowDisplayState::Minimized),
            SW_SHOWMINNOACTIVE
        );
        assert_eq!(
            show_command_for_display_state(WindowDisplayState::Maximized),
            SW_SHOWMAXIMIZED
        );
    }

    #[test]
    fn window_property_names_match_existing_format() -> Result<(), Box<dyn Error>> {
        let hwnd = WindowHandle::new(-42)?;
        let token = usize::MAX;
        let expected_identity: Vec<u16> = format!(
            "j3GridDocker.WindowIdentity.{}.{}.{}",
            std::process::id(),
            hwnd.raw(),
            token
        )
        .encode_utf16()
        .chain(once(0))
        .collect();
        let expected_owner_guard: Vec<u16> = format!(
            "j3GridDocker.OwnerGuard.{}.{}.{}",
            std::process::id(),
            hwnd.raw(),
            token
        )
        .encode_utf16()
        .chain(once(0))
        .collect();

        assert_eq!(
            window_identity_property_name(hwnd, token).as_slice(),
            expected_identity.as_slice()
        );
        assert_eq!(
            owner_guard_property_name(hwnd, token).as_slice(),
            expected_owner_guard.as_slice()
        );

        Ok(())
    }

    #[test]
    fn win32_error_preserves_user_message_and_get_last_error() {
        let error = WindowControlError::from_win32(
            WindowOperation::SetPosition,
            None,
            "SetWindowPos",
            5,
            USER_WINDOW_MOVE_MESSAGE,
        );

        assert_eq!(error.user_message(), USER_WINDOW_MOVE_MESSAGE);
        assert_eq!(
            error.internal_detail(),
            Some("SetWindowPos failed with GetLastError=5")
        );
        assert_eq!(error.win32_api(), Some("SetWindowPos"));
        assert_eq!(error.last_error(), Some(5));
    }

    #[test]
    fn child_window_is_rejected_as_external_window() -> Result<(), Box<dyn Error>> {
        let parent = TestWindow::top_level("child_rejection_parent")?;
        let child = TestWindow::child(parent.hwnd, "child_rejection_child")?;
        let hwnd = window_handle(child.hwnd)?;
        let mut controller = Win32WindowController::new();

        assert!(!controller.is_valid_external_window(hwnd)?);

        let Err(error) = controller.snapshot(hwnd) else {
            return Err(test_error("child HWND unexpectedly produced a snapshot").into());
        };

        assert_eq!(error.operation(), WindowOperation::Snapshot);
        assert_eq!(error.user_message(), USER_INVALID_WINDOW_MESSAGE);
        assert!(
            error
                .internal_detail()
                .is_some_and(|detail| detail.contains("GA_ROOT"))
        );

        eprintln!(
            "win32-smoke child-rejected hwnd={} parent={:?} detail={:?}",
            hwnd.raw(),
            unsafe { GetParent(child.hwnd) },
            error.internal_detail()
        );

        Ok(())
    }

    #[test]
    fn excluded_owner_window_is_rejected_as_external_window() -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("owner_exclusion")?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        controller.exclude_owner_window(hwnd);

        assert!(!controller.is_valid_external_window(hwnd)?);

        let Err(error) = controller.snapshot(hwnd) else {
            return Err(test_error("owner HWND unexpectedly produced a snapshot").into());
        };

        assert_eq!(error.operation(), WindowOperation::Snapshot);
        assert_eq!(error.user_message(), USER_INVALID_WINDOW_MESSAGE);
        assert!(
            error
                .internal_detail()
                .is_some_and(|detail| detail.contains("owner window"))
        );

        eprintln!(
            "win32-smoke owner-excluded hwnd={} detail={:?}",
            hwnd.raw(),
            error.internal_detail()
        );

        Ok(())
    }

    #[test]
    fn controller_smoke_places_and_restores_real_top_level_window() -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("controller_smoke")?;
        let hwnd = window_handle(window.hwnd)?;
        let parent_before = unsafe { GetParent(window.hwnd) };
        let mut controller = Win32WindowController::new();

        assert!(parent_before.is_null());
        assert!(controller.is_valid_external_window(hwnd)?);

        let snapshot = controller.snapshot(hwnd)?;
        let original_rect = snapshot.rect();
        let original_ex_style = snapshot
            .ex_style()
            .ok_or_else(|| test_error("snapshot did not include ex-style"))?;
        let mutated_ex_style = original_ex_style ^ WS_EX_TOOLWINDOW;
        set_ex_style(window.hwnd, mutated_ex_style)?;

        let target = Rect::new(80, 90, 320, 220)?;
        controller.show(&snapshot, ActivationPolicy::NoActivate)?;
        assert!(unsafe { IsWindowVisible(window.hwnd) != 0 });
        controller.set_position(&snapshot, target)?;
        assert_eq!(window_rect(window.hwnd)?, target);

        controller.hide(&snapshot)?;
        assert!(unsafe { IsWindowVisible(window.hwnd) == 0 });
        controller.restore(&snapshot)?;

        let restored_rect = window_rect(window.hwnd)?;
        let restored_ex_style = current_ex_style(window.hwnd)?;
        let parent_after = unsafe { GetParent(window.hwnd) };

        assert_eq!(restored_rect, original_rect);
        assert_eq!(restored_ex_style, original_ex_style);
        assert_eq!(parent_after, parent_before);
        assert_eq!(restored_ex_style & WS_EX_TOPMOST, 0);
        assert!(unsafe { IsWindowVisible(window.hwnd) == 0 });

        eprintln!(
            "win32-smoke controller hwnd={} snapshot_rect={:?} target_rect={:?} restored_rect={:?} style={:?} ex_style=0x{original_ex_style:08x} parent_before={:?} parent_after={:?}",
            hwnd.raw(),
            original_rect,
            target,
            restored_rect,
            snapshot.style(),
            parent_before,
            parent_after
        );

        Ok(())
    }

    #[test]
    fn controller_positions_docked_window_above_owner_without_topmost() -> Result<(), Box<dyn Error>>
    {
        let owner = TestWindow::top_level("z_order_owner")?;
        let docked = TestWindow::top_level("z_order_docked")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let docked_hwnd = window_handle(docked.hwnd)?;
        let mut controller = Win32WindowController::new();
        let snapshot = controller.snapshot(docked_hwnd)?;

        unsafe {
            ShowWindow(owner.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(docked.hwnd, SW_SHOWNOACTIVATE);
        }
        move_to_top(docked.hwnd)?;
        move_to_top(owner.hwnd)?;
        assert!(is_z_order_above(owner.hwnd, docked.hwnd));

        controller.exclude_owner_window(owner_hwnd);
        let target = Rect::new(120, 130, 340, 210)?;
        controller.set_position(&snapshot, target)?;

        assert_eq!(window_rect(docked.hwnd)?, target);
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(is_z_order_above(docked.hwnd, owner.hwnd));
        assert_eq!(current_ex_style(docked.hwnd)? & WS_EX_TOPMOST, 0);

        move_to_top(owner.hwnd)?;
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(is_z_order_above(docked.hwnd, owner.hwnd));

        controller.restore(&snapshot)?;
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, null_mut());

        eprintln!(
            "win32-smoke z-order owner={} docked={} docked_owner_set=true docked_above_owner=true ex_style=0x{:08x}",
            owner_hwnd.raw(),
            docked_hwnd.raw(),
            current_ex_style(docked.hwnd)?
        );

        Ok(())
    }

    #[test]
    fn active_dock_sync_keeps_owner_above_intervening_windows() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("z_order_owner_group")?;
        let docked = TestWindow::top_level("z_order_docked_group")?;
        let intervening = TestWindow::top_level("z_order_intervening")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let docked_hwnd = window_handle(docked.hwnd)?;
        let mut controller = Win32WindowController::new();
        let snapshot = controller.snapshot(docked_hwnd)?;

        unsafe {
            ShowWindow(owner.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(docked.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(intervening.hwnd, SW_SHOWNOACTIVATE);
        }
        move_to_top(owner.hwnd)?;
        move_to_top(intervening.hwnd)?;
        assert!(is_z_order_above(intervening.hwnd, owner.hwnd));

        controller.exclude_owner_window(owner_hwnd);
        let target = Rect::new(120, 130, 340, 210)?;
        controller.set_position(&snapshot, target)?;

        assert_eq!(window_rect(docked.hwnd)?, target);
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(is_z_order_above(docked.hwnd, owner.hwnd));
        assert!(is_z_order_above(owner.hwnd, intervening.hwnd));
        assert_eq!(current_ex_style(docked.hwnd)? & WS_EX_TOPMOST, 0);

        eprintln!(
            "win32-smoke z-order-group owner={} docked={} intervening={} owner_above_intervening=true ex_style=0x{:08x}",
            owner_hwnd.raw(),
            docked_hwnd.raw(),
            intervening.hwnd as isize,
            current_ex_style(docked.hwnd)?
        );

        Ok(())
    }

    #[test]
    fn active_dock_sync_keeps_multiple_docks_above_repaired_owner() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("z_order_multi_owner")?;
        let first = TestWindow::top_level("z_order_multi_first")?;
        let second = TestWindow::top_level("z_order_multi_second")?;
        let intervening = TestWindow::top_level("z_order_multi_intervening")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let first_hwnd = window_handle(first.hwnd)?;
        let second_hwnd = window_handle(second.hwnd)?;
        let mut controller = Win32WindowController::new();
        let first_snapshot = controller.snapshot(first_hwnd)?;
        let second_snapshot = controller.snapshot(second_hwnd)?;

        unsafe {
            ShowWindow(owner.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(first.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(second.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(intervening.hwnd, SW_SHOWNOACTIVATE);
        }
        move_to_top(owner.hwnd)?;
        move_to_top(intervening.hwnd)?;
        assert!(is_z_order_above(intervening.hwnd, owner.hwnd));

        controller.exclude_owner_window(owner_hwnd);
        controller.set_position(&first_snapshot, Rect::new(120, 130, 340, 210)?)?;
        controller.set_position(&second_snapshot, Rect::new(480, 130, 340, 210)?)?;

        assert!(is_z_order_above(first.hwnd, owner.hwnd));
        assert!(is_z_order_above(second.hwnd, owner.hwnd));
        assert!(is_z_order_above(owner.hwnd, intervening.hwnd));
        assert_eq!(current_ex_style(first.hwnd)? & WS_EX_TOPMOST, 0);
        assert_eq!(current_ex_style(second.hwnd)? & WS_EX_TOPMOST, 0);

        Ok(())
    }

    #[test]
    fn active_dock_batch_sync_keeps_multiple_docks_above_repaired_owner()
    -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("z_order_batch_owner")?;
        let first = TestWindow::top_level("z_order_batch_first")?;
        let second = TestWindow::top_level("z_order_batch_second")?;
        let intervening = TestWindow::top_level("z_order_batch_intervening")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let first_hwnd = window_handle(first.hwnd)?;
        let second_hwnd = window_handle(second.hwnd)?;
        let mut controller = Win32WindowController::new();
        let first_snapshot = controller.snapshot(first_hwnd)?;
        let second_snapshot = controller.snapshot(second_hwnd)?;
        let first_target = Rect::new(120, 130, 340, 210)?;
        let second_target = Rect::new(480, 130, 340, 210)?;

        unsafe {
            ShowWindow(owner.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(first.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(second.hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(intervening.hwnd, SW_SHOWNOACTIVATE);
        }
        move_to_top(owner.hwnd)?;
        move_to_top(intervening.hwnd)?;
        assert!(is_z_order_above(intervening.hwnd, owner.hwnd));

        controller.exclude_owner_window(owner_hwnd);
        let positions = [
            WindowPositionRequest::new(&first_snapshot, first_target),
            WindowPositionRequest::new(&second_snapshot, second_target),
        ];
        let results = controller.set_positions_if_same_external_windows(&positions);

        assert!(matches!(
            results.as_slice(),
            [
                WindowPositionResult::Positioned,
                WindowPositionResult::Positioned
            ]
        ));
        assert_eq!(window_rect(first.hwnd)?, first_target);
        assert_eq!(window_rect(second.hwnd)?, second_target);
        assert_eq!(unsafe { GetWindow(first.hwnd, GW_OWNER) }, owner.hwnd);
        assert_eq!(unsafe { GetWindow(second.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(is_z_order_above(first.hwnd, owner.hwnd));
        assert!(is_z_order_above(second.hwnd, owner.hwnd));
        assert!(is_z_order_above(owner.hwnd, intervening.hwnd));
        assert_eq!(current_ex_style(first.hwnd)? & WS_EX_TOPMOST, 0);
        assert_eq!(current_ex_style(second.hwnd)? & WS_EX_TOPMOST, 0);

        Ok(())
    }

    #[test]
    fn active_dock_batch_completion_failure_marks_all_pending_failed_and_restores_previous_owners()
    -> Result<(), Box<dyn Error>> {
        let previous_owner = TestWindow::top_level("z_order_batch_failure_previous_owner")?;
        let active_owner = TestWindow::top_level("z_order_batch_failure_active_owner")?;
        let first = TestWindow::owned(previous_owner.hwnd, "z_order_batch_failure_first")?;
        let second = TestWindow::owned(previous_owner.hwnd, "z_order_batch_failure_second")?;
        let active_owner_hwnd = window_handle(active_owner.hwnd)?;
        let first_hwnd = window_handle(first.hwnd)?;
        let second_hwnd = window_handle(second.hwnd)?;
        let mut controller = Win32WindowController::new();
        let first_snapshot = controller.snapshot(first_hwnd)?;
        let second_snapshot = controller.snapshot(second_hwnd)?;
        let first_target = Rect::new(120, 130, 340, 210)?;
        let second_target = Rect::new(480, 130, 340, 210)?;

        controller.exclude_owner_window(active_owner_hwnd);
        controller.injected_owner_z_order_failure = Some(5);
        let positions = [
            WindowPositionRequest::new(&first_snapshot, first_target),
            WindowPositionRequest::new(&second_snapshot, second_target),
        ];

        let results = controller.set_positions_if_same_external_windows(&positions);

        match results.as_slice() {
            [
                WindowPositionResult::Failed(first_error),
                WindowPositionResult::Failed(second_error),
            ] => {
                assert_eq!(first_error.operation(), WindowOperation::SetPosition);
                assert_eq!(first_error.win32_api(), Some("SetWindowPos(owner z-order)"));
                assert_eq!(first_error.last_error(), Some(5));
                assert_eq!(second_error.operation(), WindowOperation::SetPosition);
                assert_eq!(
                    second_error.win32_api(),
                    Some("SetWindowPos(owner z-order)")
                );
                assert_eq!(second_error.last_error(), Some(5));
            }
            _ => {
                return Err(test_error(
                    "batch owner z-order failure did not fail every pending result",
                )
                .into());
            }
        }
        assert_eq!(
            unsafe { GetWindow(first.hwnd, GW_OWNER) },
            previous_owner.hwnd
        );
        assert_eq!(
            unsafe { GetWindow(second.hwnd, GW_OWNER) },
            previous_owner.hwnd
        );

        Ok(())
    }

    #[test]
    fn snapshot_identity_mismatch_is_rejected_before_positioning() -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("identity_mismatch")?;
        let hwnd = window_handle(window.hwnd)?;
        let original = window_rect(window.hwnd)?;
        let mut controller = Win32WindowController::new();
        let snapshot = controller.snapshot(hwnd)?;
        let identity = match snapshot.identity() {
            Some(identity) => identity,
            None => return Err(test_error("snapshot did not include window identity").into()),
        };
        let mismatched_thread_id = if identity.thread_id() == u32::MAX {
            identity.thread_id() - 1
        } else {
            identity.thread_id() + 1
        };
        let stale_snapshot = snapshot.clone().with_identity(WindowIdentity::new(
            mismatched_thread_id,
            identity.process_id(),
        ));
        let target = Rect::new(140, 150, 360, 240)?;

        let result = controller.set_position(&stale_snapshot, target);

        let Err(error) = result else {
            return Err(test_error("identity mismatch unexpectedly positioned window").into());
        };
        assert_eq!(error.operation(), WindowOperation::SetPosition);
        assert_eq!(error.user_message(), USER_INVALID_WINDOW_MESSAGE);
        assert!(
            error
                .internal_detail()
                .is_some_and(|detail| detail.contains("snapshot identity"))
        );
        assert_eq!(window_rect(window.hwnd)?, original);

        Ok(())
    }

    #[test]
    fn snapshot_cleans_identity_guard_when_rect_capture_fails_after_recording()
    -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("snapshot_identity_guard_cleanup")?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();
        let mut recorded_token = None;

        let result = controller.snapshot_with(
            hwnd,
            |controller, raw, hwnd| {
                let guard = match controller.snapshot_identity_guards.first().copied() {
                    Some(guard) => guard,
                    None => {
                        return Err(win32_error(
                            WindowOperation::Snapshot,
                            Some(hwnd),
                            "InjectedGetWindowRect",
                            5,
                            USER_WINDOW_ACCESS_MESSAGE,
                        ));
                    }
                };

                assert_eq!(guard.hwnd, hwnd);
                assert!(window_identity_property_matches(hwnd, raw, guard.token));
                recorded_token = Some(guard.token);

                Err(win32_error(
                    WindowOperation::Snapshot,
                    Some(hwnd),
                    "InjectedGetWindowRect",
                    5,
                    USER_WINDOW_ACCESS_MESSAGE,
                ))
            },
            Win32WindowController::window_style,
        );

        let Err(error) = result else {
            return Err(test_error("injected snapshot rect failure unexpectedly succeeded").into());
        };
        assert_eq!(error.operation(), WindowOperation::Snapshot);
        assert_eq!(error.win32_api(), Some("InjectedGetWindowRect"));
        assert_eq!(error.last_error(), Some(5));

        let Some(token) = recorded_token else {
            return Err(test_error("snapshot failure did not observe identity guard").into());
        };
        assert!(controller.snapshot_identity_guards.is_empty());
        assert!(!window_identity_property_matches(hwnd, window.hwnd, token));

        Ok(())
    }

    #[test]
    fn snapshot_fails_when_identity_guard_registration_fails() -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("snapshot_identity_guard_failure")?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();
        controller.injected_set_prop_failure = Some(SetPropFailure::Identity);
        let mut rect_called = false;

        let result = controller.snapshot_with(
            hwnd,
            |controller, raw, hwnd| {
                rect_called = true;
                controller.snapshot_rect(raw, hwnd)
            },
            Win32WindowController::window_style,
        );

        let Err(error) = result else {
            return Err(
                test_error("identity guard registration failure unexpectedly succeeded").into(),
            );
        };

        assert_eq!(error.operation(), WindowOperation::Snapshot);
        assert_eq!(error.win32_api(), Some("SetPropW(snapshot identity guard)"));
        assert_eq!(error.last_error(), Some(INJECTED_SET_PROP_ERROR));
        assert!(!rect_called);
        assert!(controller.snapshot_identity_guards.is_empty());

        Ok(())
    }

    #[test]
    fn snapshot_fails_when_owner_guard_registration_fails() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("snapshot_owner_guard_failure_owner")?;
        let docked = TestWindow::owned(owner.hwnd, "snapshot_owner_guard_failure_docked")?;
        let docked_hwnd = window_handle(docked.hwnd)?;
        let mut controller = Win32WindowController::new();
        controller.injected_set_prop_failure = Some(SetPropFailure::Owner);

        let result = controller.snapshot(docked_hwnd);

        let Err(error) = result else {
            return Err(
                test_error("owner guard registration failure unexpectedly succeeded").into(),
            );
        };

        assert_eq!(error.operation(), WindowOperation::Snapshot);
        assert_eq!(error.win32_api(), Some("SetPropW(snapshot owner guard)"));
        assert_eq!(error.last_error(), Some(INJECTED_SET_PROP_ERROR));
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(controller.snapshot_identity_guards.is_empty());
        assert!(controller.snapshot_owner_guards.is_empty());

        Ok(())
    }

    #[test]
    fn active_dock_set_position_failure_restores_previous_owner() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("owner_rollback_owner")?;
        let docked = TestWindow::top_level("owner_rollback_docked")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let docked_hwnd = window_handle(docked.hwnd)?;
        let mut controller = Win32WindowController::new();

        controller.exclude_owner_window(owner_hwnd);
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, null_mut());

        let target = Rect::new(120, 130, 340, 210)?;
        let result = controller.set_position_raw_with(
            docked.hwnd,
            docked_hwnd,
            target,
            WindowOperation::SetPosition,
            ZOrderPolicy::ActiveDock,
            |raw, _insert_after, _x, _y, _width, _height, _flags| {
                assert_eq!(unsafe { GetWindow(raw, GW_OWNER) }, owner.hwnd);
                unsafe {
                    SetLastError(5);
                }
                0
            },
        );

        let Err(error) = result else {
            return Err(test_error("forced SetWindowPos failure unexpectedly succeeded").into());
        };

        assert_eq!(error.operation(), WindowOperation::SetPosition);
        assert_eq!(error.win32_api(), Some("SetWindowPos"));
        assert_eq!(error.last_error(), Some(5));
        assert_eq!(unsafe { GetWindow(docked.hwnd, GW_OWNER) }, null_mut());

        Ok(())
    }

    #[test]
    fn active_dock_completion_failure_restores_previous_owner() -> Result<(), Box<dyn Error>> {
        let previous_owner = TestWindow::top_level("z_order_rollback_previous_owner")?;
        let active_owner = TestWindow::top_level("z_order_rollback_active_owner")?;
        let docked = TestWindow::owned(previous_owner.hwnd, "z_order_rollback_docked")?;
        let active_owner_hwnd = window_handle(active_owner.hwnd)?;
        let docked_hwnd = window_handle(docked.hwnd)?;
        let mut controller = Win32WindowController::new();

        controller.exclude_owner_window(active_owner_hwnd);
        assert_eq!(
            unsafe { GetWindow(docked.hwnd, GW_OWNER) },
            previous_owner.hwnd
        );

        let application =
            controller.z_order_application(docked.hwnd, docked_hwnd, ZOrderPolicy::ActiveDock)?;
        assert_eq!(
            unsafe { GetWindow(docked.hwnd, GW_OWNER) },
            active_owner.hwnd
        );

        let injected_error = win32_error(
            WindowOperation::SetPosition,
            Some(docked_hwnd),
            "SetWindowPos(owner z-order)",
            5,
            USER_WINDOW_MOVE_MESSAGE,
        );
        let result = controller.handle_z_order_completion_result(
            docked.hwnd,
            docked_hwnd,
            application,
            Err(injected_error),
        );

        let Err(error) = result else {
            return Err(
                test_error("injected z-order completion failure unexpectedly succeeded").into(),
            );
        };

        assert_eq!(error.operation(), WindowOperation::SetPosition);
        assert_eq!(error.win32_api(), Some("SetWindowPos(owner z-order)"));
        assert_eq!(error.last_error(), Some(5));
        assert_eq!(
            unsafe { GetWindow(docked.hwnd, GW_OWNER) },
            previous_owner.hwnd
        );

        Ok(())
    }

    #[test]
    fn restore_uses_snapshot_z_order_hint_and_falls_back_when_invalid() -> Result<(), Box<dyn Error>>
    {
        let target = TestWindow::top_level("restore_z_order_target")?;
        let insert_after = TestWindow::top_level("restore_z_order_insert_after")?;
        let hwnd = window_handle(target.hwnd)?;
        let rect = window_rect(target.hwnd)?;
        let snapshot = WindowSnapshot::new(hwnd, rect, WindowDisplayState::Normal)
            .with_z_order_hint(ZOrderHint::new(insert_after.hwnd as isize));
        let mut controller = Win32WindowController::new();

        controller.restore_with(
            &snapshot,
            |raw, actual_insert_after, x, y, width, height, flags| {
                assert_eq!(raw, target.hwnd);
                assert_eq!(actual_insert_after, insert_after.hwnd);
                assert_eq!(flags & SWP_NOZORDER, 0);
                assert_eq!(
                    (x, y, width, height),
                    (rect.left(), rect.top(), rect.width(), rect.height())
                );
                1
            },
        )?;

        let invalid_snapshot = WindowSnapshot::new(hwnd, rect, WindowDisplayState::Normal)
            .with_z_order_hint(ZOrderHint::new(1));
        controller.restore_with(
            &invalid_snapshot,
            |_raw, actual_insert_after, _x, _y, _width, _height, flags| {
                assert_eq!(actual_insert_after, null_mut());
                assert_ne!(flags & SWP_NOZORDER, 0);
                1
            },
        )?;

        Ok(())
    }

    #[test]
    fn restore_reapplies_verified_snapshot_owner() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("verified_owner")?;
        let window = TestWindow::owned(owner.hwnd, "verified_owner_window")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, owner.hwnd);

        let snapshot = controller.snapshot(hwnd)?;
        assert_eq!(snapshot.owner(), Some(owner_hwnd));

        controller.set_window_owner(window.hwnd, hwnd, None, WindowOperation::SetPosition)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());

        controller.restore(&snapshot)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, owner.hwnd);

        Ok(())
    }

    #[test]
    fn restore_detached_clears_owner_instead_of_reapplying_snapshot_owner()
    -> Result<(), Box<dyn Error>> {
        let original_owner = TestWindow::top_level("detached_original_owner")?;
        let dock_owner = TestWindow::top_level("detached_dock_owner")?;
        let window = TestWindow::owned(original_owner.hwnd, "detached_owned_window")?;
        let original_owner_hwnd = window_handle(original_owner.hwnd)?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        let snapshot = controller.snapshot(hwnd)?;
        assert_eq!(snapshot.owner(), Some(original_owner_hwnd));
        assert_eq!(
            unsafe { GetWindow(window.hwnd, GW_OWNER) },
            original_owner.hwnd
        );

        controller.set_window_owner(
            window.hwnd,
            hwnd,
            Some(dock_owner.hwnd),
            WindowOperation::SetPosition,
        )?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, dock_owner.hwnd);

        controller.restore_detached_with(
            &snapshot,
            |raw, insert_after, x, y, width, height, flags| {
                assert_eq!(unsafe { GetWindow(raw, GW_OWNER) }, null_mut());
                unsafe { SetWindowPos(raw, insert_after, x, y, width, height, flags) }
            },
        )?;

        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());
        assert_eq!(window_rect(window.hwnd)?, snapshot.rect());
        assert!(controller.snapshot_owner_guards.is_empty());

        Ok(())
    }

    #[test]
    fn restore_rolls_back_owner_and_position_when_position_restore_fails()
    -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("restore_guard_failure_owner")?;
        let window = TestWindow::owned(owner.hwnd, "restore_guard_failure_window")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        let snapshot = controller.snapshot(hwnd)?;
        assert_eq!(snapshot.owner(), Some(owner_hwnd));
        assert_eq!(controller.snapshot_owner_guards.len(), 1);

        controller.set_window_owner(window.hwnd, hwnd, None, WindowOperation::SetPosition)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());
        let docked_rect = Rect::new(210, 220, 260, 190)?;
        controller.set_position_raw(
            window.hwnd,
            hwnd,
            docked_rect,
            WindowOperation::SetPosition,
            ZOrderPolicy::Preserve,
        )?;
        assert_eq!(window_rect(window.hwnd)?, docked_rect);

        let result = controller.restore_with(
            &snapshot,
            |raw, _insert_after, x, y, width, height, _flags| {
                assert_eq!(unsafe { GetWindow(raw, GW_OWNER) }, owner.hwnd);
                let ok = unsafe {
                    SetWindowPos(
                        raw,
                        null_mut(),
                        x,
                        y,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                    )
                };
                assert_ne!(ok, 0, "partial restore SetWindowPos setup failed");
                unsafe {
                    SetLastError(5);
                }
                0
            },
        );

        let Err(error) = result else {
            return Err(
                test_error("forced restore SetWindowPos failure unexpectedly succeeded").into(),
            );
        };

        assert_eq!(error.operation(), WindowOperation::Restore);
        assert_eq!(error.win32_api(), Some("SetWindowPos"));
        assert_eq!(error.last_error(), Some(5));
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());
        assert_eq!(window_rect(window.hwnd)?, docked_rect);
        assert_eq!(controller.snapshot_owner_guards.len(), 1);
        assert_eq!(
            controller.validated_restore_owner(window.hwnd, hwnd, snapshot.owner()),
            Some(owner.hwnd)
        );

        controller.restore(&snapshot)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, owner.hwnd);
        assert!(controller.snapshot_owner_guards.is_empty());
        assert_eq!(
            controller.validated_restore_owner(window.hwnd, hwnd, snapshot.owner()),
            None
        );

        Ok(())
    }

    #[test]
    fn restore_rolls_back_owner_and_style_when_style_restore_fails() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("restore_style_failure_owner")?;
        let window = TestWindow::owned(owner.hwnd, "restore_style_failure_window")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        let snapshot = controller.snapshot(hwnd)?;
        assert_eq!(snapshot.owner(), Some(owner_hwnd));
        let original_style = snapshot
            .style()
            .ok_or_else(|| test_error("snapshot did not include style"))?;
        let docked_style = original_style ^ WS_MINIMIZEBOX;

        controller.set_window_owner(window.hwnd, hwnd, None, WindowOperation::SetPosition)?;
        controller.set_window_style(
            window.hwnd,
            hwnd,
            GWL_STYLE,
            docked_style,
            "SetWindowLongPtrW(GWL_STYLE)",
        )?;
        controller.apply_frame_changed(window.hwnd, hwnd)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());
        assert_eq!(current_style(window.hwnd)?, docked_style);

        controller.injected_set_window_style_failure = Some(GWL_EXSTYLE);
        let mut position_called = false;
        let result = controller.restore_with(
            &snapshot,
            |_raw, _insert_after, _x, _y, _width, _height, _flags| {
                position_called = true;
                1
            },
        );

        let Err(error) = result else {
            return Err(test_error("forced restore style failure unexpectedly succeeded").into());
        };

        assert_eq!(error.operation(), WindowOperation::Restore);
        assert_eq!(error.win32_api(), Some("SetWindowLongPtrW(GWL_EXSTYLE)"));
        assert_eq!(error.last_error(), Some(INJECTED_SET_WINDOW_STYLE_ERROR));
        assert!(!position_called);
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());
        assert_eq!(current_style(window.hwnd)?, docked_style);
        assert_eq!(controller.snapshot_owner_guards.len(), 1);
        assert_eq!(
            controller.validated_restore_owner(window.hwnd, hwnd, snapshot.owner()),
            Some(owner.hwnd)
        );

        controller.injected_set_window_style_failure = None;

        Ok(())
    }

    #[test]
    fn restore_skips_unverified_snapshot_owner() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("unverified_owner")?;
        let window = TestWindow::top_level("unverified_owner_window")?;
        let owner_hwnd = window_handle(owner.hwnd)?;
        let hwnd = window_handle(window.hwnd)?;
        let snapshot =
            WindowSnapshot::new(hwnd, window_rect(window.hwnd)?, WindowDisplayState::Normal)
                .with_owner(owner_hwnd);
        let mut controller = Win32WindowController::new();

        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());

        controller.restore(&snapshot)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());

        Ok(())
    }

    #[test]
    fn restore_skips_destroyed_snapshot_owner() -> Result<(), Box<dyn Error>> {
        let owner = TestWindow::top_level("destroyed_owner")?;
        let owner_raw = owner.hwnd;
        let window = TestWindow::owned(owner_raw, "destroyed_owner_window")?;
        let owner_hwnd = window_handle(owner_raw)?;
        let hwnd = window_handle(window.hwnd)?;
        let mut controller = Win32WindowController::new();

        let snapshot = controller.snapshot(hwnd)?;
        assert_eq!(snapshot.owner(), Some(owner_hwnd));

        controller.set_window_owner(window.hwnd, hwnd, None, WindowOperation::SetPosition)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());

        drop(owner);
        assert!(!is_window(owner_raw));
        assert!(is_window(window.hwnd));

        controller.restore(&snapshot)?;
        assert_eq!(unsafe { GetWindow(window.hwnd, GW_OWNER) }, null_mut());

        Ok(())
    }

    #[test]
    fn app_smoke_saves_snapshot_and_restores_real_top_level_window() -> Result<(), Box<dyn Error>> {
        let window = TestWindow::top_level("app_smoke")?;
        let hwnd = window_handle(window.hwnd)?;
        let original_rect = window_rect(window.hwnd)?;
        let original_parent = unsafe { GetParent(window.hwnd) };
        let original_ex_style = current_ex_style(window.hwnd)?;
        let mut app = App::new(Win32WindowController::new());
        let bounds = Rect::new(140, 150, 300, 180)?;
        let tab_id = app.add_tab("Smoke")?;
        let region_id = app.layout_for_tab(tab_id, bounds)?[0].region_id();

        app.place_window(tab_id, region_id, hwnd, bounds)?;
        assert_eq!(window_rect(window.hwnd)?, bounds);
        assert!(unsafe { IsWindowVisible(window.hwnd) != 0 });

        let status = app.unregister_placement(tab_id, region_id)?;
        let restored_rect = window_rect(window.hwnd)?;
        let restored_parent = unsafe { GetParent(window.hwnd) };
        let restored_owner = unsafe { GetWindow(window.hwnd, GW_OWNER) };
        let restored_ex_style = current_ex_style(window.hwnd)?;

        assert_eq!(status, crate::app::UndockStatus::Restored);
        assert_eq!(restored_rect, original_rect);
        assert_eq!(restored_parent, original_parent);
        assert_eq!(restored_owner, null_mut());
        assert_eq!(restored_ex_style, original_ex_style);
        assert_eq!(restored_ex_style & WS_EX_TOPMOST, 0);
        assert!(unsafe { IsWindowVisible(window.hwnd) == 0 });

        eprintln!(
            "win32-smoke app hwnd={} placed_rect={:?} restored_rect={:?} parent={:?} ex_style=0x{restored_ex_style:08x}",
            hwnd.raw(),
            bounds,
            restored_rect,
            restored_parent
        );

        Ok(())
    }

    #[test]
    fn app_shutdown_restores_visible_window_hidden_for_inactive_tab() -> Result<(), Box<dyn Error>>
    {
        let window = TestWindow::top_level("app_shutdown_inactive_visible")?;
        let hwnd = window_handle(window.hwnd)?;
        let mut app = App::new(Win32WindowController::new());
        let bounds = Rect::new(140, 150, 300, 180)?;
        let _active_tab = app.add_tab("Active")?;
        let inactive_tab = app.add_tab("Inactive")?;
        let region_id = app.layout_for_tab(inactive_tab, bounds)?[0].region_id();

        unsafe {
            ShowWindow(window.hwnd, SW_SHOWNOACTIVATE);
        }
        assert!(unsafe { IsWindowVisible(window.hwnd) != 0 });

        app.place_window(inactive_tab, region_id, hwnd, bounds)?;
        assert!(unsafe { IsWindowVisible(window.hwnd) == 0 });

        let report = app.shutdown();

        assert_eq!(report.attempted(), 1);
        assert_eq!(report.restored(), 1);
        assert!(report.failures().is_empty());
        assert!(unsafe { IsWindowVisible(window.hwnd) != 0 });

        eprintln!(
            "win32-smoke shutdown-inactive-visible hwnd={} restored_visible=true",
            hwnd.raw()
        );

        Ok(())
    }

    struct TestWindow {
        hwnd: HWND,
    }

    impl TestWindow {
        fn top_level(label: &str) -> Result<Self, Box<dyn Error>> {
            Self::create(label, 0, WS_OVERLAPPEDWINDOW, null_mut())
        }

        fn owned(owner: HWND, label: &str) -> Result<Self, Box<dyn Error>> {
            Self::create(label, 0, WS_OVERLAPPEDWINDOW, owner)
        }

        fn child(parent: HWND, label: &str) -> Result<Self, Box<dyn Error>> {
            Self::create(label, 0, WS_CHILD, parent)
        }

        fn create(
            label: &str,
            ex_style: u32,
            style: u32,
            parent: HWND,
        ) -> Result<Self, Box<dyn Error>> {
            let hinstance = module_handle()?;
            let class_name = unique_class_name(label);
            register_smoke_class(hinstance, &class_name)?;
            let title = wide_null(label);

            clear_last_error();
            let hwnd = unsafe {
                CreateWindowExW(
                    ex_style,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    style,
                    -32000,
                    -32000,
                    240,
                    160,
                    parent,
                    null_mut(),
                    hinstance,
                    null_mut(),
                )
            };

            if hwnd.is_null() {
                Err(test_error(format!(
                    "CreateWindowExW failed with GetLastError={}",
                    last_error()
                ))
                .into())
            } else {
                Ok(Self { hwnd })
            }
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            if !self.hwnd.is_null() && is_window(self.hwnd) {
                unsafe {
                    DestroyWindow(self.hwnd);
                }
            }
        }
    }

    unsafe extern "system" fn smoke_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn module_handle() -> Result<HINSTANCE, Box<dyn Error>> {
        let handle = unsafe { GetModuleHandleW(null()) };
        if handle.is_null() {
            Err(test_error(format!(
                "GetModuleHandleW failed with GetLastError={}",
                last_error()
            ))
            .into())
        } else {
            Ok(handle)
        }
    }

    fn register_smoke_class(
        hinstance: HINSTANCE,
        class_name: &[u16],
    ) -> Result<(), Box<dyn Error>> {
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(smoke_window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
        };

        clear_last_error();
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            Err(test_error(format!(
                "RegisterClassW failed with GetLastError={}",
                last_error()
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn unique_class_name(label: &str) -> Vec<u16> {
        let sequence = NEXT_CLASS_ID.fetch_add(1, Ordering::Relaxed);
        wide_null(&format!(
            "j3GridDocker.SmokeWindow.{}.{}.{}",
            std::process::id(),
            sequence,
            label
        ))
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(once(0)).collect()
    }

    fn window_handle(hwnd: HWND) -> Result<WindowHandle, DomainError> {
        WindowHandle::new(hwnd as isize)
    }

    fn window_rect(hwnd: HWND) -> Result<Rect, Box<dyn Error>> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        clear_last_error();
        let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
        if ok == 0 {
            return Err(test_error(format!(
                "GetWindowRect failed with GetLastError={}",
                last_error()
            ))
            .into());
        }

        Ok(win32_rect_to_domain(window_handle(hwnd)?, rect)?)
    }

    fn current_ex_style(hwnd: HWND) -> Result<u32, Box<dyn Error>> {
        get_window_long(hwnd, GWL_EXSTYLE, "GetWindowLongPtrW(GWL_EXSTYLE)")
    }

    fn current_style(hwnd: HWND) -> Result<u32, Box<dyn Error>> {
        get_window_long(hwnd, GWL_STYLE, "GetWindowLongPtrW(GWL_STYLE)")
    }

    fn set_ex_style(hwnd: HWND, ex_style: u32) -> Result<(), Box<dyn Error>> {
        clear_last_error();
        let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize) };
        let error = last_error();

        if previous == 0 && error != 0 {
            Err(test_error(format!(
                "SetWindowLongPtrW(GWL_EXSTYLE) failed with GetLastError={error}"
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn move_to_top(hwnd: HWND) -> Result<(), Box<dyn Error>> {
        clear_last_error();
        let ok = unsafe {
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        if ok == 0 {
            Err(test_error(format!(
                "SetWindowPos(HWND_TOP) failed with GetLastError={}",
                last_error()
            ))
            .into())
        } else {
            Ok(())
        }
    }

    fn is_z_order_above(above: HWND, below: HWND) -> bool {
        let mut current = unsafe { GetWindow(below, GW_HWNDPREV) };
        let mut hops = 0;

        while !current.is_null() && hops < 512 {
            if current == above {
                return true;
            }
            current = unsafe { GetWindow(current, GW_HWNDPREV) };
            hops += 1;
        }

        false
    }

    fn get_window_long(hwnd: HWND, index: i32, api: &'static str) -> Result<u32, Box<dyn Error>> {
        clear_last_error();
        let value = unsafe { GetWindowLongPtrW(hwnd, index) };
        let error = last_error();

        if value == 0 && error != 0 {
            Err(test_error(format!("{api} failed with GetLastError={error}")).into())
        } else {
            Ok(value as u32)
        }
    }

    fn test_error(message: impl Into<String>) -> io::Error {
        io::Error::other(message.into())
    }
}
