use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, POINT, SetLastError, WPARAM};
use windows_sys::Win32::UI::Controls::{
    ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASSW, TTDT_AUTOPOP,
    TTDT_INITIAL, TTDT_RESHOW, TTM_ADDTOOLW, TTM_DELTOOLW, TTM_RELAYEVENT, TTM_SETDELAYTIME,
    TTM_SETMAXTIPWIDTH, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
#[cfg(test)]
use windows_sys::Win32::UI::Controls::{TTHITTESTINFOW, TTM_GETTOOLCOUNT, TTM_HITTESTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DestroyWindow, GetCursorPos, HWND_TOPMOST, MSG, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SendMessageW, SetWindowPos, WS_EX_TOPMOST, WS_POPUP,
};

use crate::domain::TabId;

#[cfg(test)]
use super::ui::ClientPoint;
use super::ui::UiRect;
use super::{WideText, Win32StatusFailure};

const TAB_TOOLTIP_MAX_WIDTH: i32 = 360;
const TAB_TOOLTIP_AUTOPOP_MS: i32 = 15_000;
const TAB_TOOLTIP_INITIAL_MS: i32 = 500;
const TAB_TOOLTIP_RESHOW_MS: i32 = 100;

pub(super) fn text_from_titles(titles: impl IntoIterator<Item = String>) -> Option<String> {
    let mut tooltip = String::new();
    for title in titles {
        let title = title.trim();
        if title.is_empty() {
            continue;
        }

        if !tooltip.is_empty() {
            tooltip.push('\n');
        }
        tooltip.push_str(title);
    }

    (!tooltip.is_empty()).then_some(tooltip)
}

fn tool_info(owner: HWND, id: usize, rect: UiRect, text: &[u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: tool_info_size(),
        uFlags: 0,
        hwnd: owner,
        uId: id,
        rect: rect.to_rect(),
        hinst: null_mut(),
        lpszText: text.as_ptr() as *mut u16,
        lParam: 0,
        lpReserved: null_mut(),
    }
}

fn delete_info(owner: HWND, id: usize) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: tool_info_size(),
        hwnd: owner,
        uId: id,
        ..TTTOOLINFOW::default()
    }
}

fn tool_info_lparam(info: &mut TTTOOLINFOW) -> LPARAM {
    info as *mut TTTOOLINFOW as LPARAM
}

fn tool_info_size() -> u32 {
    // Some common-controls versions reject TTM_ADDTOOLW when cbSize includes lpReserved.
    (size_of::<TTTOOLINFOW>() - size_of::<*mut core::ffi::c_void>()) as u32
}

#[cfg(test)]
fn hit_test_info(owner: HWND, point: ClientPoint) -> TTHITTESTINFOW {
    TTHITTESTINFOW {
        hwnd: owner,
        pt: POINT {
            x: point.x,
            y: point.y,
        },
        ti: TTTOOLINFOW {
            cbSize: tool_info_size(),
            ..TTTOOLINFOW::default()
        },
    }
}

fn relay_msg(owner: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> MSG {
    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut point);
    }

    MSG {
        hwnd: owner,
        message,
        wParam: wparam,
        lParam: lparam,
        time: 0,
        pt: point,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabTooltipSpec {
    pub(super) tab_id: TabId,
    pub(super) rect: UiRect,
    pub(super) text: String,
}

#[derive(Debug, Default)]
pub(super) struct TabTooltip {
    hwnd: HWND,
    tools: Vec<TabTooltipTool>,
}

impl TabTooltip {
    pub(super) fn initialize(&mut self, owner: HWND) -> Result<(), Win32StatusFailure> {
        if !self.hwnd.is_null() {
            return Ok(());
        }

        let common_controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        let initialized = unsafe { InitCommonControlsEx(&common_controls) };
        if initialized == 0 {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "InitCommonControlsEx",
                last_error,
                "탭 tooltip 공용 컨트롤을 초기화할 수 없습니다.",
            ));
        }

        unsafe {
            SetLastError(0);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                owner,
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if hwnd.is_null() {
            let last_error = unsafe { GetLastError() };
            return Err(Win32StatusFailure::new(
                "CreateWindowExW",
                last_error,
                "탭 tooltip window를 생성할 수 없습니다.",
            ));
        }

        self.hwnd = hwnd;
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        self.configure();
        Ok(())
    }

    pub(super) fn sync(&mut self, owner: HWND, specs: Vec<TabTooltipSpec>) -> bool {
        if self.hwnd.is_null() {
            return false;
        }
        if self.matches_specs(&specs) {
            return true;
        }

        let expected_tool_count = specs.len();
        self.clear(owner);
        for (index, spec) in specs.into_iter().enumerate() {
            let Some(id) = index.checked_add(1) else {
                break;
            };
            if !self.add_tool(owner, id, spec) {
                break;
            }
        }

        self.tools.len() == expected_tool_count
    }

    pub(super) fn sync_tab(
        &mut self,
        owner: HWND,
        tab_id: TabId,
        spec: Option<TabTooltipSpec>,
    ) -> bool {
        if self.hwnd.is_null() {
            return false;
        }

        let Some(spec) = spec else {
            self.remove_tool_for_tab(owner, tab_id);
            return true;
        };
        if let Some(index) = self.tools.iter().position(|tool| tool.tab_id == tab_id) {
            if self.tools[index].rect == spec.rect && self.tools[index].text.text == spec.text {
                return true;
            }

            let id = self.tools[index].id;
            self.delete_tool(owner, id);
            self.tools.remove(index);
            return self.add_tool_at(owner, id, spec, index);
        }

        let Some(id) = self.next_tool_id() else {
            return false;
        };
        self.add_tool(owner, id, spec)
    }

    pub(super) fn relay_mouse_message(
        &self,
        owner: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) {
        if self.hwnd.is_null() || owner.is_null() || self.tools.is_empty() {
            return;
        }

        let msg = relay_msg(owner, message, wparam, lparam);
        unsafe {
            SendMessageW(self.hwnd, TTM_RELAYEVENT, 0, &msg as *const MSG as LPARAM);
        }
    }

    pub(super) fn destroy(&mut self, owner: HWND) {
        self.clear(owner);
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = null_mut();
        }
    }

    fn configure(&self) {
        if self.hwnd.is_null() {
            return;
        }

        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_SETMAXTIPWIDTH,
                0,
                TAB_TOOLTIP_MAX_WIDTH as LPARAM,
            );
            SendMessageW(
                self.hwnd,
                TTM_SETDELAYTIME,
                TTDT_AUTOPOP as WPARAM,
                TAB_TOOLTIP_AUTOPOP_MS as LPARAM,
            );
            SendMessageW(
                self.hwnd,
                TTM_SETDELAYTIME,
                TTDT_INITIAL as WPARAM,
                TAB_TOOLTIP_INITIAL_MS as LPARAM,
            );
            SendMessageW(
                self.hwnd,
                TTM_SETDELAYTIME,
                TTDT_RESHOW as WPARAM,
                TAB_TOOLTIP_RESHOW_MS as LPARAM,
            );
        }
    }

    fn matches_specs(&self, specs: &[TabTooltipSpec]) -> bool {
        self.tools.len() == specs.len()
            && self.tools.iter().zip(specs).all(|(tool, spec)| {
                tool.tab_id == spec.tab_id && tool.rect == spec.rect && tool.text.text == spec.text
            })
    }

    #[cfg(test)]
    fn native_tool_count(&self) -> usize {
        if self.hwnd.is_null() {
            return 0;
        }

        let count = unsafe { SendMessageW(self.hwnd, TTM_GETTOOLCOUNT, 0, 0) };
        usize::try_from(count).unwrap_or(0)
    }

    #[cfg(test)]
    fn hit_test_tool_id(&self, owner: HWND, point: ClientPoint) -> Option<usize> {
        if self.hwnd.is_null() {
            return None;
        }

        let mut hit = hit_test_info(owner, point);
        let result =
            unsafe { SendMessageW(self.hwnd, TTM_HITTESTW, 0, &mut hit as *mut _ as LPARAM) };
        if result == 0 { None } else { Some(hit.ti.uId) }
    }

    fn add_tool(&mut self, owner: HWND, id: usize, spec: TabTooltipSpec) -> bool {
        self.add_tool_at(owner, id, spec, self.tools.len())
    }

    fn add_tool_at(&mut self, owner: HWND, id: usize, spec: TabTooltipSpec, index: usize) -> bool {
        let text = WideText::new(spec.text);
        let mut info = tool_info(owner, id, spec.rect, text.wide());
        let added =
            unsafe { SendMessageW(self.hwnd, TTM_ADDTOOLW, 0, tool_info_lparam(&mut info)) };
        if added == 0 {
            return false;
        }

        let tool = TabTooltipTool {
            id,
            tab_id: spec.tab_id,
            rect: spec.rect,
            text,
        };
        if index >= self.tools.len() {
            self.tools.push(tool);
        } else {
            self.tools.insert(index, tool);
        }
        true
    }

    fn clear(&mut self, owner: HWND) {
        if self.hwnd.is_null() {
            self.tools.clear();
            return;
        }

        for tool in &self.tools {
            self.delete_tool(owner, tool.id);
        }
        self.tools.clear();
    }

    fn remove_tool_for_tab(&mut self, owner: HWND, tab_id: TabId) {
        let Some(index) = self.tools.iter().position(|tool| tool.tab_id == tab_id) else {
            return;
        };
        let id = self.tools[index].id;
        self.delete_tool(owner, id);
        self.tools.remove(index);
    }

    fn delete_tool(&self, owner: HWND, id: usize) {
        let mut info = delete_info(owner, id);
        unsafe {
            SendMessageW(self.hwnd, TTM_DELTOOLW, 0, tool_info_lparam(&mut info));
        }
    }

    fn next_tool_id(&self) -> Option<usize> {
        self.tools
            .iter()
            .map(|tool| tool.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
    }
}

#[derive(Debug)]
struct TabTooltipTool {
    id: usize,
    tab_id: TabId,
    rect: UiRect,
    text: WideText,
}

#[cfg(test)]
mod tests {
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW,
        WM_MOUSEMOVE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    use super::super::{module_handle, wide_null};
    use super::*;

    fn test_hwnd(value: isize) -> HWND {
        value as HWND
    }

    struct TooltipOwnerWindow {
        hwnd: HWND,
    }

    impl TooltipOwnerWindow {
        fn new() -> Self {
            register_tooltip_owner_test_class();
            let class = wide_null("j3GridDocker.TooltipOwnerTest");
            let title = wide_null("tooltip-owner");
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    40,
                    40,
                    240,
                    120,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            assert!(!hwnd.is_null(), "test tooltip owner window was not created");
            Self { hwnd }
        }
    }

    unsafe extern "system" fn tooltip_owner_test_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn register_tooltip_owner_test_class() {
        let class_name = wide_null("j3GridDocker.TooltipOwnerTest");
        let hinstance = module_handle().expect("test module handle should be available");
        let wndclass = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tooltip_owner_test_proc),
            hInstance: hinstance,
            lpszClassName: class_name.as_ptr(),
            ..WNDCLASSW::default()
        };

        unsafe {
            SetLastError(0);
        }
        let atom = unsafe { RegisterClassW(&wndclass) };
        if atom == 0 {
            let last_error = unsafe { GetLastError() };
            assert_eq!(last_error, ERROR_CLASS_ALREADY_EXISTS);
        }
    }

    impl Drop for TooltipOwnerWindow {
        fn drop(&mut self) {
            if !self.hwnd.is_null() {
                unsafe {
                    DestroyWindow(self.hwnd);
                }
                self.hwnd = null_mut();
            }
        }
    }

    fn assert_rect_eq(actual: RECT, expected: RECT) {
        assert_eq!(actual.left, expected.left);
        assert_eq!(actual.top, expected.top);
        assert_eq!(actual.right, expected.right);
        assert_eq!(actual.bottom, expected.bottom);
    }

    #[test]
    fn text_from_titles_joins_window_titles_by_line() {
        let text = text_from_titles([" Editor ".to_owned(), "".to_owned(), "Terminal".to_owned()]);

        assert_eq!(text.as_deref(), Some("Editor\nTerminal"));
    }

    #[test]
    fn text_from_titles_returns_none_without_titles() {
        assert_eq!(text_from_titles(Vec::<String>::new()), None);
        assert_eq!(text_from_titles(["  ".to_owned()]), None);
    }

    #[test]
    fn tool_info_uses_compatible_size_and_manual_relay_flags() {
        let rect = UiRect::new(10, 4, 142, 32);
        let text = WideText::new("Editor");
        let info = tool_info(test_hwnd(11), 7, rect, text.wide());

        assert_eq!(info.cbSize, tool_info_size());
        assert_eq!(info.uFlags, 0);
        assert_eq!(info.hwnd, test_hwnd(11));
        assert_eq!(info.uId, 7);
        assert_rect_eq(info.rect, rect.to_rect());
        assert_eq!(info.lpszText, text.wide().as_ptr() as *mut u16);
    }

    #[test]
    fn relay_msg_preserves_mouse_message_identity() {
        let msg = relay_msg(test_hwnd(13), WM_MOUSEMOVE, 1, 0x0012_0034);

        assert_eq!(msg.hwnd, test_hwnd(13));
        assert_eq!(msg.message, WM_MOUSEMOVE);
        assert_eq!(msg.wParam, 1);
        assert_eq!(msg.lParam, 0x0012_0034);
    }

    #[test]
    fn tab_tooltip_registers_native_rect_tool_and_hit_tests_client_point() {
        let owner = TooltipOwnerWindow::new();
        let mut tooltip = TabTooltip::default();
        tooltip
            .initialize(owner.hwnd)
            .expect("tooltip window should initialize for a real owner");
        tooltip.sync(
            owner.hwnd,
            vec![TabTooltipSpec {
                tab_id: TabId::new(1),
                rect: UiRect::new(10, 10, 100, 32),
                text: "Tooltip Target Window".to_owned(),
            }],
        );

        assert_eq!(tooltip.tools.len(), 1);
        assert_eq!(tooltip.native_tool_count(), 1);
        assert_eq!(
            tooltip.hit_test_tool_id(owner.hwnd, ClientPoint { x: 20, y: 20 }),
            Some(1)
        );
        assert_eq!(
            tooltip.hit_test_tool_id(owner.hwnd, ClientPoint { x: 120, y: 20 }),
            None
        );

        tooltip.destroy(owner.hwnd);
    }

    #[test]
    fn tab_tooltip_sync_tab_updates_one_native_tool() {
        let owner = TooltipOwnerWindow::new();
        let mut tooltip = TabTooltip::default();
        tooltip
            .initialize(owner.hwnd)
            .expect("tooltip window should initialize for a real owner");
        assert!(tooltip.sync(
            owner.hwnd,
            vec![
                TabTooltipSpec {
                    tab_id: TabId::new(1),
                    rect: UiRect::new(10, 10, 100, 32),
                    text: "First Window".to_owned(),
                },
                TabTooltipSpec {
                    tab_id: TabId::new(2),
                    rect: UiRect::new(110, 10, 200, 32),
                    text: "Second Window".to_owned(),
                },
            ],
        ));
        let first_id = tooltip.tools[0].id;
        let second_id = tooltip.tools[1].id;

        assert!(tooltip.sync_tab(
            owner.hwnd,
            TabId::new(2),
            Some(TabTooltipSpec {
                tab_id: TabId::new(2),
                rect: UiRect::new(110, 10, 200, 32),
                text: "Second Window Renamed".to_owned(),
            }),
        ));

        assert_eq!(tooltip.native_tool_count(), 2);
        assert_eq!(tooltip.tools[0].id, first_id);
        assert_eq!(tooltip.tools[0].text.text, "First Window");
        assert_eq!(tooltip.tools[1].id, second_id);
        assert_eq!(tooltip.tools[1].text.text, "Second Window Renamed");

        tooltip.destroy(owner.hwnd);
    }
}
