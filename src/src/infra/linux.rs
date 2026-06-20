use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask,
    GetGeometryReply, KeyButMask, MapState, PropMode, StackMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as X11WrapperConnectionExt;

use crate::app::{ActivationPolicy, WindowControlError, WindowController, WindowOperation};
use crate::domain::{
    ExternalProgramSpec, Rect, WindowDisplayState, WindowHandle, WindowIdentity, WindowSnapshot,
    ZOrderHint,
};

const USER_X11_UNAVAILABLE_MESSAGE: &str =
    "Linux 외부 창 제어에는 X11 세션이 필요합니다. Wayland 세션에서는 지원되지 않습니다.";
const USER_INVALID_WINDOW_MESSAGE: &str = "유효하지 않은 외부 윈도우입니다.";
const USER_WINDOW_ACCESS_MESSAGE: &str = "외부 윈도우 상태를 조회할 수 없습니다.";
const USER_PROGRAM_ACCESS_MESSAGE: &str = "외부 프로그램 정보를 조회할 수 없습니다.";
const USER_WINDOW_MOVE_MESSAGE: &str = "외부 윈도우 위치를 변경할 수 없습니다.";
const USER_WINDOW_RESTORE_MESSAGE: &str = "외부 윈도우를 복원할 수 없습니다.";
const USER_SPLITTER_OVERLAY_MESSAGE: &str = "splitter overlay를 갱신할 수 없습니다.";
const MAX_CLIENT_WINDOW_SEARCH_DEPTH: usize = 8;
const WM_STATE_ICONIC: u32 = 3;
const NET_WM_STATE_REMOVE: u32 = 0;
const NET_WM_STATE_ADD: u32 = 1;
const NET_WM_STATE_SOURCE_APPLICATION: u32 = 1;
const LINUX_Z_ORDER_HINT_WAS_ABOVE: isize = 1;
const SPLITTER_OVERLAY_BACKGROUND_PIXEL: u32 = 0x0090_9090;
const MAX_NET_WM_STATE_ATOMS: u32 = 64;
const MAX_CLIENT_LIST_STACKING_WINDOWS: u32 = 4096;
static NEXT_WINDOW_IDENTITY_TOKEN: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug)]
pub struct LinuxWindowController {
    connection: Option<RustConnection>,
    screen_num: usize,
    snapshot_identity_guards: Vec<LinuxSnapshotIdentityGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxSnapshotIdentityGuard {
    hwnd: WindowHandle,
    token: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxPointerState {
    hwnd: Option<WindowHandle>,
    root_x: i32,
    root_y: i32,
    left_button_down: bool,
    control_down: bool,
}

impl LinuxPointerState {
    pub const fn hwnd(self) -> Option<WindowHandle> {
        self.hwnd
    }

    pub const fn root_x(self) -> i32 {
        self.root_x
    }

    pub const fn root_y(self) -> i32 {
        self.root_y
    }

    pub const fn left_button_down(self) -> bool {
        self.left_button_down
    }

    pub const fn control_down(self) -> bool {
        self.control_down
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxOverlayWindow {
    raw: Window,
}

impl LinuxWindowController {
    pub fn new() -> Self {
        match RustConnection::connect(None) {
            Ok((connection, screen_num)) => Self {
                connection: Some(connection),
                screen_num,
                snapshot_identity_guards: Vec::new(),
            },
            Err(error) => {
                eprintln!("X11 connection unavailable: {error}");
                Self {
                    connection: None,
                    screen_num: 0,
                    snapshot_identity_guards: Vec::new(),
                }
            }
        }
    }

    pub fn x11_available(&self) -> bool {
        self.connection.is_some()
    }

    pub fn pointer_state(&self) -> Result<LinuxPointerState, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let root = self.root_window(connection);
        let reply = connection
            .query_pointer(root)
            .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryPointer", error))?
            .reply()
            .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryPointer", error))?;
        let hwnd = self.client_window_for_raw(reply.child)?;

        Ok(LinuxPointerState {
            hwnd,
            root_x: i32::from(reply.root_x),
            root_y: i32::from(reply.root_y),
            left_button_down: reply.mask.contains(KeyButMask::BUTTON1),
            control_down: reply.mask.contains(KeyButMask::CONTROL),
        })
    }

    pub fn active_window(&self) -> Result<Option<WindowHandle>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let root = self.root_window(connection);
        if let Some(raw) = self.net_active_window(root)? {
            return self.client_window_for_raw(raw);
        }

        let focus = connection
            .get_input_focus()
            .map_err(|error| x11_error(WindowOperation::Validate, None, "GetInputFocus", error))?
            .reply()
            .map_err(|error| x11_error(WindowOperation::Validate, None, "GetInputFocus", error))?
            .focus;
        self.client_window_for_raw(focus)
    }

    pub fn create_splitter_overlay_window(
        &mut self,
    ) -> Result<LinuxOverlayWindow, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let screen = &connection.setup().roots[self.screen_num];
        let raw = connection
            .generate_id()
            .map_err(|error| splitter_overlay_error("GenerateId", error))?;
        let aux = CreateWindowAux::new()
            .background_pixel(SPLITTER_OVERLAY_BACKGROUND_PIXEL)
            .override_redirect(1u32)
            .event_mask(
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            );
        connection
            .create_window(
                screen.root_depth,
                raw,
                screen.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &aux,
            )
            .map_err(|error| splitter_overlay_error("CreateWindow", error))?
            .check()
            .map_err(|error| splitter_overlay_error("CreateWindow", error))?;
        self.flush(WindowOperation::Validate, None, "Flush")?;
        Ok(LinuxOverlayWindow { raw })
    }

    pub fn set_splitter_overlay_rect(
        &mut self,
        window: LinuxOverlayWindow,
        rect: Rect,
    ) -> Result<(), WindowControlError> {
        let connection = self.connection(WindowOperation::SetPosition, None)?;
        let aux = ConfigureWindowAux::new()
            .x(rect.left())
            .y(rect.top())
            .width(rect.width() as u32)
            .height(rect.height() as u32)
            .stack_mode(StackMode::ABOVE);
        connection
            .configure_window(window.raw, &aux)
            .map_err(|error| splitter_overlay_error("ConfigureWindow", error))?
            .check()
            .map_err(|error| splitter_overlay_error("ConfigureWindow", error))?;
        connection
            .map_window(window.raw)
            .map_err(|error| splitter_overlay_error("MapWindow", error))?
            .check()
            .map_err(|error| splitter_overlay_error("MapWindow", error))?;
        self.flush(WindowOperation::SetPosition, None, "Flush")
    }

    pub fn hide_splitter_overlay_window(
        &mut self,
        window: LinuxOverlayWindow,
    ) -> Result<(), WindowControlError> {
        let connection = self.connection(WindowOperation::Hide, None)?;
        connection
            .unmap_window(window.raw)
            .map_err(|error| splitter_overlay_error("UnmapWindow", error))?
            .check()
            .map_err(|error| splitter_overlay_error("UnmapWindow", error))?;
        self.flush(WindowOperation::Hide, None, "Flush")
    }

    pub fn destroy_splitter_overlay_window(
        &mut self,
        window: LinuxOverlayWindow,
    ) -> Result<(), WindowControlError> {
        let connection = self.connection(WindowOperation::Restore, None)?;
        connection
            .destroy_window(window.raw)
            .map_err(|error| splitter_overlay_error("DestroyWindow", error))?
            .check()
            .map_err(|error| splitter_overlay_error("DestroyWindow", error))?;
        self.flush(WindowOperation::Restore, None, "Flush")
    }

    pub fn current_rect(&self, hwnd: WindowHandle) -> Result<Rect, WindowControlError> {
        let raw = self.valid_raw_window(WindowOperation::Validate, hwnd)?;
        self.window_rect(WindowOperation::Validate, hwnd, raw)
    }

    pub fn current_rect_for_xid(&self, xid: u64) -> Result<Rect, WindowControlError> {
        let hwnd = WindowHandle::new(xid as isize).map_err(|error| {
            WindowControlError::new(
                WindowOperation::Validate,
                None,
                USER_INVALID_WINDOW_MESSAGE,
                Some(error.to_string()),
            )
        })?;
        self.current_rect(hwnd)
    }

    pub fn set_rect_for_xid(&mut self, xid: u64, rect: Rect) -> Result<(), WindowControlError> {
        let hwnd = WindowHandle::new(xid as isize).map_err(|error| {
            WindowControlError::new(
                WindowOperation::SetPosition,
                None,
                USER_INVALID_WINDOW_MESSAGE,
                Some(error.to_string()),
            )
        })?;
        let raw = self.valid_raw_window(WindowOperation::SetPosition, hwnd)?;
        self.configure_window_rect(WindowOperation::SetPosition, hwnd, raw, rect)
    }

    pub fn window_title(&self, hwnd: WindowHandle) -> Result<Option<String>, WindowControlError> {
        let raw = self.valid_raw_window(WindowOperation::Snapshot, hwnd)?;
        self.window_title_for_raw(hwnd, raw)
    }

    pub fn top_level_windows_for_processes<I>(
        &self,
        process_ids: I,
    ) -> Result<HashMap<u32, WindowHandle>, WindowControlError>
    where
        I: IntoIterator<Item = u32>,
    {
        let targets = process_ids
            .into_iter()
            .filter(|process_id| *process_id != 0)
            .collect::<HashSet<_>>();
        if targets.is_empty() {
            return Ok(HashMap::new());
        }

        let connection = self.connection(WindowOperation::Validate, None)?;
        let root = self.root_window(connection);
        let mut candidates = self.client_list_stacking(root)?;
        if candidates.is_empty() {
            candidates = connection
                .query_tree(root)
                .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryTree", error))?
                .reply()
                .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryTree", error))?
                .children;
        }

        let mut matches = HashMap::new();
        for raw in candidates {
            let Some((process_id, hwnd)) = self.visible_window_for_process(raw, &targets)? else {
                continue;
            };
            matches.insert(process_id, hwnd);
        }
        Ok(matches)
    }

    fn connection(
        &self,
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
    ) -> Result<&RustConnection, WindowControlError> {
        self.connection.as_ref().ok_or_else(|| {
            WindowControlError::new(
                operation,
                hwnd,
                USER_X11_UNAVAILABLE_MESSAGE,
                Some(String::from(
                    "No X11 connection is available. GTK4 may be running on Wayland.",
                )),
            )
        })
    }

    fn root_window(&self, connection: &RustConnection) -> Window {
        connection.setup().roots[self.screen_num].root
    }

    fn raw_window(hwnd: WindowHandle) -> Result<Window, WindowControlError> {
        u32::try_from(hwnd.raw()).map_err(|error| {
            WindowControlError::new(
                WindowOperation::Validate,
                Some(hwnd),
                USER_INVALID_WINDOW_MESSAGE,
                Some(format!(
                    "window handle cannot be converted to X11 Window: {error}"
                )),
            )
        })
    }

    fn valid_raw_window(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
    ) -> Result<Window, WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let raw = Self::raw_window(hwnd)?;
        if raw == self.root_window(connection) {
            return Err(WindowControlError::new(
                operation,
                Some(hwnd),
                USER_INVALID_WINDOW_MESSAGE,
                Some(String::from("X11 root window cannot be docked.")),
            ));
        }

        connection
            .get_window_attributes(raw)
            .map_err(|error| x11_error(operation, Some(hwnd), "GetWindowAttributes", error))?
            .reply()
            .map_err(|error| {
                if reply_error_is_bad_window(&error) {
                    WindowControlError::new(
                        operation,
                        Some(hwnd),
                        USER_INVALID_WINDOW_MESSAGE,
                        Some(format!("GetWindowAttributes returned BadWindow: {error}")),
                    )
                } else {
                    x11_error(operation, Some(hwnd), "GetWindowAttributes", error)
                }
            })?;

        Ok(raw)
    }

    fn net_active_window(&self, root: Window) -> Result<Option<Window>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let active_window =
            self.atom_for(WindowOperation::Validate, None, b"_NET_ACTIVE_WINDOW")?;
        let reply = connection
            .get_property(false, root, active_window, AtomEnum::WINDOW, 0, 1)
            .map_err(|error| {
                x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetProperty(_NET_ACTIVE_WINDOW)",
                    error,
                )
            })?
            .reply()
            .map_err(|error| {
                x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetProperty(_NET_ACTIVE_WINDOW)",
                    error,
                )
            })?;
        let raw = reply.value32().and_then(|mut values| values.next());
        Ok(raw.and_then(|raw| x11_window_candidate(raw, root)))
    }

    fn client_window_for_raw(
        &self,
        raw: Window,
    ) -> Result<Option<WindowHandle>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let root = self.root_window(connection);
        let Some(raw) = x11_window_candidate(raw, root) else {
            return Ok(None);
        };

        let wm_state = self.atom_for(WindowOperation::Validate, None, b"WM_STATE")?;
        let target = if let Some(descendant) =
            self.find_descendant_with_property(raw, wm_state, MAX_CLIENT_WINDOW_SEARCH_DEPTH)?
        {
            descendant
        } else if self.window_has_property(raw, wm_state)? {
            raw
        } else {
            self.find_ancestor_with_property(raw, root, wm_state, MAX_CLIENT_WINDOW_SEARCH_DEPTH)?
                .unwrap_or(raw)
        };
        WindowHandle::new(target as isize)
            .map(Some)
            .map_err(|error| {
                WindowControlError::new(
                    WindowOperation::Validate,
                    None,
                    USER_INVALID_WINDOW_MESSAGE,
                    Some(format!("X11 client window handle is invalid: {error}")),
                )
            })
    }

    fn find_ancestor_with_property(
        &self,
        raw: Window,
        root: Window,
        property: u32,
        max_depth: usize,
    ) -> Result<Option<Window>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let mut current = raw;
        for _ in 0..max_depth {
            let parent = match connection
                .query_tree(current)
                .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryTree", error))?
                .reply()
            {
                Ok(reply) => reply.parent,
                Err(error) if reply_error_is_bad_window(&error) => return Ok(None),
                Err(error) => {
                    return Err(x11_error(
                        WindowOperation::Validate,
                        None,
                        "QueryTree",
                        error,
                    ));
                }
            };
            let Some(parent) = x11_window_candidate(parent, root) else {
                return Ok(None);
            };
            if self.window_has_property(parent, property)? {
                return Ok(Some(parent));
            }
            current = parent;
        }
        Ok(None)
    }

    fn find_descendant_with_property(
        &self,
        raw: Window,
        property: u32,
        max_depth: usize,
    ) -> Result<Option<Window>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let mut stack = vec![(raw, 0usize)];
        while let Some((window, depth)) = stack.pop() {
            if depth > 0 && self.window_has_property(window, property)? {
                return Ok(Some(window));
            }
            if depth >= max_depth {
                continue;
            }

            let children = match connection
                .query_tree(window)
                .map_err(|error| x11_error(WindowOperation::Validate, None, "QueryTree", error))?
                .reply()
            {
                Ok(reply) => reply.children,
                Err(error) if reply_error_is_bad_window(&error) => continue,
                Err(error) => {
                    return Err(x11_error(
                        WindowOperation::Validate,
                        None,
                        "QueryTree",
                        error,
                    ));
                }
            };
            stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
        }

        Ok(None)
    }

    fn window_has_property(&self, raw: Window, property: u32) -> Result<bool, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let reply = match connection
            .get_property(false, raw, property, AtomEnum::ANY, 0, 0)
            .map_err(|error| x11_error(WindowOperation::Validate, None, "GetProperty", error))?
            .reply()
        {
            Ok(reply) => reply,
            Err(error) if reply_error_is_bad_window(&error) => return Ok(false),
            Err(error) => {
                return Err(x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetProperty",
                    error,
                ));
            }
        };
        Ok(reply.type_ != AtomEnum::NONE.into())
    }

    fn configure_window_rect(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        rect: Rect,
    ) -> Result<(), WindowControlError> {
        let aux = ConfigureWindowAux::new()
            .x(rect.left())
            .y(rect.top())
            .width(rect.width() as u32)
            .height(rect.height() as u32)
            .stack_mode(StackMode::ABOVE);
        self.connection(operation, Some(hwnd))?
            .configure_window(raw, &aux)
            .map_err(|error| x11_error(operation, Some(hwnd), "ConfigureWindow", error))?
            .check()
            .map_err(|error| x11_error(operation, Some(hwnd), "ConfigureWindow", error))?;
        self.flush(operation, Some(hwnd), "Flush")
    }

    fn map_raw(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<(), WindowControlError> {
        self.connection(operation, Some(hwnd))?
            .map_window(raw)
            .map_err(|error| x11_error(operation, Some(hwnd), "MapWindow", error))?
            .check()
            .map_err(|error| x11_error(operation, Some(hwnd), "MapWindow", error))?;
        self.flush(operation, Some(hwnd), "Flush")
    }

    fn unmap_raw(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<(), WindowControlError> {
        self.connection(operation, Some(hwnd))?
            .unmap_window(raw)
            .map_err(|error| x11_error(operation, Some(hwnd), "UnmapWindow", error))?
            .check()
            .map_err(|error| x11_error(operation, Some(hwnd), "UnmapWindow", error))?;
        self.flush(operation, Some(hwnd), "Flush")
    }

    fn set_net_wm_maximized(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        maximized: bool,
    ) -> Result<(), WindowControlError> {
        let action = if maximized {
            NET_WM_STATE_ADD
        } else {
            NET_WM_STATE_REMOVE
        };
        let horz = self.atom_for(operation, Some(hwnd), b"_NET_WM_STATE_MAXIMIZED_HORZ")?;
        let vert = self.atom_for(operation, Some(hwnd), b"_NET_WM_STATE_MAXIMIZED_VERT")?;
        self.send_client_message_to_root(
            operation,
            hwnd,
            raw,
            b"_NET_WM_STATE",
            [action, horz, vert, NET_WM_STATE_SOURCE_APPLICATION, 0],
            "SendEvent(_NET_WM_STATE)",
        )
    }

    fn set_net_wm_state_atom(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        atom_name: &[u8],
        enabled: bool,
        api: &'static str,
    ) -> Result<(), WindowControlError> {
        let action = if enabled {
            NET_WM_STATE_ADD
        } else {
            NET_WM_STATE_REMOVE
        };
        let atom = self.atom_for(operation, Some(hwnd), atom_name)?;
        self.send_client_message_to_root(
            operation,
            hwnd,
            raw,
            b"_NET_WM_STATE",
            [action, atom, 0, NET_WM_STATE_SOURCE_APPLICATION, 0],
            api,
        )
    }

    fn set_net_wm_above(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        above: bool,
    ) -> Result<(), WindowControlError> {
        self.set_net_wm_state_atom(
            operation,
            hwnd,
            raw,
            b"_NET_WM_STATE_ABOVE",
            above,
            "SendEvent(_NET_WM_STATE_ABOVE)",
        )
    }

    fn request_iconic_state(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<(), WindowControlError> {
        self.send_client_message_to_root(
            operation,
            hwnd,
            raw,
            b"WM_CHANGE_STATE",
            [WM_STATE_ICONIC, 0, 0, 0, 0],
            "SendEvent(WM_CHANGE_STATE)",
        )
    }

    fn send_client_message_to_root(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        message_type: &[u8],
        data: [u32; 5],
        api: &'static str,
    ) -> Result<(), WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let event_type = self.atom_for(operation, Some(hwnd), message_type)?;
        let event = ClientMessageEvent::new(32, raw, event_type, data);
        connection
            .send_event(
                false,
                self.root_window(connection),
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .map_err(|error| x11_error(operation, Some(hwnd), api, error))?
            .check()
            .map_err(|error| x11_error(operation, Some(hwnd), api, error))?;
        self.flush(operation, Some(hwnd), "Flush")
    }

    fn window_rect(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<Rect, WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let geometry = connection
            .get_geometry(raw)
            .map_err(|error| x11_error(operation, Some(hwnd), "GetGeometry", error))?
            .reply()
            .map_err(|error| x11_error(operation, Some(hwnd), "GetGeometry", error))?;
        let translated = connection
            .translate_coordinates(raw, self.root_window(connection), 0, 0)
            .map_err(|error| x11_error(operation, Some(hwnd), "TranslateCoordinates", error))?
            .reply()
            .map_err(|error| x11_error(operation, Some(hwnd), "TranslateCoordinates", error))?;

        rect_from_geometry(
            &geometry,
            i32::from(translated.dst_x),
            i32::from(translated.dst_y),
        )
        .map_err(|error| {
            WindowControlError::new(
                operation,
                Some(hwnd),
                USER_WINDOW_ACCESS_MESSAGE,
                Some(format!("invalid X11 window geometry: {error}")),
            )
        })
    }

    fn display_state(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<WindowDisplayState, WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let attributes = connection
            .get_window_attributes(raw)
            .map_err(|error| x11_error(operation, Some(hwnd), "GetWindowAttributes", error))?
            .reply()
            .map_err(|error| x11_error(operation, Some(hwnd), "GetWindowAttributes", error))?;
        let wm_state = self.wm_state(operation, hwnd, raw)?;
        let maximized_horz = self.window_has_atom_property(
            operation,
            hwnd,
            raw,
            b"_NET_WM_STATE",
            b"_NET_WM_STATE_MAXIMIZED_HORZ",
            "GetProperty(_NET_WM_STATE)",
        )?;
        let maximized_vert = self.window_has_atom_property(
            operation,
            hwnd,
            raw,
            b"_NET_WM_STATE",
            b"_NET_WM_STATE_MAXIMIZED_VERT",
            "GetProperty(_NET_WM_STATE)",
        )?;

        Ok(display_state_from_x11_parts(
            wm_state,
            attributes.map_state,
            maximized_horz,
            maximized_vert,
        ))
    }

    fn flush(
        &self,
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
        api: &'static str,
    ) -> Result<(), WindowControlError> {
        self.connection(operation, hwnd)?
            .flush()
            .map_err(|error| x11_error(operation, hwnd, api, error))
    }

    fn atom_for(
        &self,
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
        name: &[u8],
    ) -> Result<u32, WindowControlError> {
        let connection = self.connection(operation, hwnd)?;
        Ok(connection
            .intern_atom(false, name)
            .map_err(|error| x11_error(operation, hwnd, "InternAtom", error))?
            .reply()
            .map_err(|error| x11_error(operation, hwnd, "InternAtom", error))?
            .atom)
    }

    fn wm_state(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<Option<u32>, WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let wm_state = self.atom_for(operation, Some(hwnd), b"WM_STATE")?;
        let reply = connection
            .get_property(false, raw, wm_state, AtomEnum::ANY, 0, 2)
            .map_err(|error| x11_error(operation, Some(hwnd), "GetProperty(WM_STATE)", error))?
            .reply()
            .map_err(|error| x11_error(operation, Some(hwnd), "GetProperty(WM_STATE)", error))?;
        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    fn window_has_atom_property(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        property_name: &[u8],
        atom_name: &[u8],
        api: &'static str,
    ) -> Result<bool, WindowControlError> {
        let connection = self.connection(operation, Some(hwnd))?;
        let property = self.atom_for(operation, Some(hwnd), property_name)?;
        let target = self.atom_for(operation, Some(hwnd), atom_name)?;
        let reply = connection
            .get_property(
                false,
                raw,
                property,
                AtomEnum::ATOM,
                0,
                MAX_NET_WM_STATE_ATOMS,
            )
            .map_err(|error| x11_error(operation, Some(hwnd), api, error))?
            .reply()
            .map_err(|error| x11_error(operation, Some(hwnd), api, error))?;
        Ok(reply
            .value32()
            .is_some_and(|mut atoms| atoms.any(|atom| atom == target)))
    }

    fn process_id_for_raw_window(
        &self,
        operation: WindowOperation,
        hwnd: Option<WindowHandle>,
        raw: Window,
    ) -> Result<Option<u32>, WindowControlError> {
        let connection = self.connection(operation, hwnd)?;
        let atom = self.atom_for(operation, hwnd, b"_NET_WM_PID")?;
        let reply = connection
            .get_property(false, raw, atom, AtomEnum::CARDINAL, 0, 1)
            .map_err(|error| x11_error(operation, hwnd, "GetProperty(_NET_WM_PID)", error))?
            .reply()
            .map_err(|error| x11_error(operation, hwnd, "GetProperty(_NET_WM_PID)", error))?;

        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    fn window_identity(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<WindowIdentity, WindowControlError> {
        let process_id = self
            .process_id_for_raw_window(operation, Some(hwnd), raw)?
            .unwrap_or(0);
        Ok(WindowIdentity::new(0, process_id))
    }

    fn same_snapshot_window_raw(
        &self,
        operation: WindowOperation,
        snapshot: &WindowSnapshot,
    ) -> Result<Option<Window>, WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = match self.valid_raw_window(operation, hwnd) {
            Ok(raw) => raw,
            Err(error) if error.user_message() == USER_INVALID_WINDOW_MESSAGE => return Ok(None),
            Err(error) => return Err(error),
        };

        let Some(expected) = snapshot.identity() else {
            return Ok(Some(raw));
        };
        let actual = self.window_identity(operation, hwnd, raw)?;
        if self.window_identity_matches(operation, hwnd, raw, expected, actual)? {
            Ok(Some(raw))
        } else {
            Ok(None)
        }
    }

    fn ensure_snapshot_raw_window(
        &self,
        operation: WindowOperation,
        snapshot: &WindowSnapshot,
    ) -> Result<Window, WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.valid_raw_window(operation, hwnd)?;

        if let Some(expected) = snapshot.identity() {
            let actual = self.window_identity(operation, hwnd, raw)?;
            if !self.window_identity_matches(operation, hwnd, raw, expected, actual)? {
                return Err(WindowControlError::new(
                    operation,
                    Some(hwnd),
                    USER_INVALID_WINDOW_MESSAGE,
                    Some(String::from(
                        "X11 window identity does not match the captured snapshot identity.",
                    )),
                ));
            }
        }

        Ok(raw)
    }

    fn window_identity_matches(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        expected: WindowIdentity,
        actual: WindowIdentity,
    ) -> Result<bool, WindowControlError> {
        if expected.thread_id() != actual.thread_id()
            || expected.process_id() != actual.process_id()
        {
            return Ok(false);
        }

        match expected.validation_token() {
            Some(token) => self.window_identity_property_matches(operation, hwnd, raw, token),
            None => Ok(true),
        }
    }

    fn record_snapshot_identity_guard(
        &mut self,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<usize, WindowControlError> {
        self.clear_snapshot_identity_guard(hwnd);

        let token = next_window_identity_token();
        self.set_window_identity_property(hwnd, raw, token)?;
        self.snapshot_identity_guards
            .push(LinuxSnapshotIdentityGuard { hwnd, token });
        Ok(token)
    }

    fn clear_snapshot_identity_guard(&mut self, hwnd: WindowHandle) {
        if let Some(index) = self
            .snapshot_identity_guards
            .iter()
            .position(|guard| guard.hwnd == hwnd)
        {
            let guard = self.snapshot_identity_guards.swap_remove(index);
            self.remove_window_identity_property(guard.hwnd, guard.token);
        }
    }

    fn set_window_identity_property(
        &self,
        hwnd: WindowHandle,
        raw: Window,
        token: usize,
    ) -> Result<(), WindowControlError> {
        let operation = WindowOperation::Snapshot;
        let property = self.window_identity_property_atom(operation, hwnd, token)?;
        let words = token_to_x11_words(token);
        self.connection(operation, Some(hwnd))?
            .change_property32(PropMode::REPLACE, raw, property, AtomEnum::CARDINAL, &words)
            .map_err(|error| {
                x11_error(
                    operation,
                    Some(hwnd),
                    "ChangeProperty(snapshot identity guard)",
                    error,
                )
            })?
            .check()
            .map_err(|error| {
                x11_error(
                    operation,
                    Some(hwnd),
                    "ChangeProperty(snapshot identity guard)",
                    error,
                )
            })?;
        self.flush(operation, Some(hwnd), "Flush")
    }

    fn window_identity_property_matches(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        raw: Window,
        token: usize,
    ) -> Result<bool, WindowControlError> {
        let property = self.window_identity_property_atom(operation, hwnd, token)?;
        let reply = self
            .connection(operation, Some(hwnd))?
            .get_property(false, raw, property, AtomEnum::CARDINAL, 0, 2)
            .map_err(|error| {
                x11_error(
                    operation,
                    Some(hwnd),
                    "GetProperty(snapshot identity guard)",
                    error,
                )
            })?
            .reply()
            .map_err(|error| {
                x11_error(
                    operation,
                    Some(hwnd),
                    "GetProperty(snapshot identity guard)",
                    error,
                )
            })?;
        let Some(values) = reply.value32() else {
            return Ok(false);
        };
        let actual = values.collect::<Vec<_>>();
        Ok(actual.as_slice() == token_to_x11_words(token))
    }

    fn remove_window_identity_property(&self, hwnd: WindowHandle, token: usize) {
        let Ok(raw) = Self::raw_window(hwnd) else {
            return;
        };
        let Ok(property) =
            self.window_identity_property_atom(WindowOperation::Validate, hwnd, token)
        else {
            return;
        };
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        if let Ok(cookie) = connection.delete_property(raw, property) {
            let _ = cookie.check();
        }
        let _ = connection.flush();
    }

    fn window_identity_property_atom(
        &self,
        operation: WindowOperation,
        hwnd: WindowHandle,
        token: usize,
    ) -> Result<u32, WindowControlError> {
        self.atom_for(
            operation,
            Some(hwnd),
            window_identity_property_name(hwnd, token).as_bytes(),
        )
    }

    fn process_id_for_window(&self, raw: Window) -> Result<Option<u32>, WindowControlError> {
        self.process_id_for_raw_window(WindowOperation::InspectProgram, None, raw)
    }

    fn client_list_stacking(&self, root: Window) -> Result<Vec<Window>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let atom = self.atom_for(
            WindowOperation::Validate,
            None,
            b"_NET_CLIENT_LIST_STACKING",
        )?;
        let reply = connection
            .get_property(
                false,
                root,
                atom,
                AtomEnum::WINDOW,
                0,
                MAX_CLIENT_LIST_STACKING_WINDOWS,
            )
            .map_err(|error| {
                x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetProperty(_NET_CLIENT_LIST_STACKING)",
                    error,
                )
            })?
            .reply()
            .map_err(|error| {
                x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetProperty(_NET_CLIENT_LIST_STACKING)",
                    error,
                )
            })?;
        Ok(reply
            .value32()
            .map(|values| {
                values
                    .take(MAX_CLIENT_LIST_STACKING_WINDOWS as usize)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default())
    }

    fn visible_window_for_process(
        &self,
        raw: Window,
        process_ids: &HashSet<u32>,
    ) -> Result<Option<(u32, WindowHandle)>, WindowControlError> {
        let connection = self.connection(WindowOperation::Validate, None)?;
        let attributes = match connection
            .get_window_attributes(raw)
            .map_err(|error| {
                x11_error(
                    WindowOperation::Validate,
                    None,
                    "GetWindowAttributes",
                    error,
                )
            })?
            .reply()
        {
            Ok(attributes) => attributes,
            Err(_) => return Ok(None),
        };
        if attributes.map_state != MapState::VIEWABLE {
            return Ok(None);
        }

        let Some(process_id) = self.process_id_for_window(raw)? else {
            return Ok(None);
        };
        if !process_ids.contains(&process_id) {
            return Ok(None);
        }
        let Some(hwnd) = WindowHandle::new(raw as isize).ok() else {
            return Ok(None);
        };
        Ok(Some((process_id, hwnd)))
    }

    fn window_title_for_raw(
        &self,
        hwnd: WindowHandle,
        raw: Window,
    ) -> Result<Option<String>, WindowControlError> {
        let net_wm_name = self.atom_for(WindowOperation::Snapshot, Some(hwnd), b"_NET_WM_NAME")?;
        let utf8_string = self.atom_for(WindowOperation::Snapshot, Some(hwnd), b"UTF8_STRING")?;
        if let Some(title) = self.string_property(
            hwnd,
            raw,
            net_wm_name,
            utf8_string,
            "GetProperty(_NET_WM_NAME)",
        )? {
            return Ok(Some(title));
        }

        self.string_property(
            hwnd,
            raw,
            AtomEnum::WM_NAME.into(),
            AtomEnum::STRING.into(),
            "GetProperty(WM_NAME)",
        )
    }

    fn string_property(
        &self,
        hwnd: WindowHandle,
        raw: Window,
        property: u32,
        property_type: u32,
        api: &'static str,
    ) -> Result<Option<String>, WindowControlError> {
        let connection = self.connection(WindowOperation::Snapshot, Some(hwnd))?;
        let reply = connection
            .get_property(false, raw, property, property_type, 0, 4096)
            .map_err(|error| x11_error(WindowOperation::Snapshot, Some(hwnd), api, error))?
            .reply()
            .map_err(|error| x11_error(WindowOperation::Snapshot, Some(hwnd), api, error))?;
        if reply.value.is_empty() {
            return Ok(None);
        }
        let title = String::from_utf8_lossy(&reply.value)
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        if title.is_empty() {
            Ok(None)
        } else {
            Ok(Some(title))
        }
    }
}

impl Default for LinuxWindowController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LinuxWindowController {
    fn drop(&mut self) {
        let guards = std::mem::take(&mut self.snapshot_identity_guards);
        for guard in guards {
            self.remove_window_identity_property(guard.hwnd, guard.token);
        }
    }
}

impl WindowController for LinuxWindowController {
    fn is_valid_external_window(&mut self, hwnd: WindowHandle) -> Result<bool, WindowControlError> {
        match self.valid_raw_window(WindowOperation::Validate, hwnd) {
            Ok(_) => Ok(true),
            Err(error) if error.user_message() == USER_INVALID_WINDOW_MESSAGE => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn is_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
    ) -> Result<bool, WindowControlError> {
        self.same_snapshot_window_raw(WindowOperation::Validate, snapshot)
            .map(|raw| raw.is_some())
    }

    fn snapshot(&mut self, hwnd: WindowHandle) -> Result<WindowSnapshot, WindowControlError> {
        let raw = self.valid_raw_window(WindowOperation::Snapshot, hwnd)?;
        let mut identity = self.window_identity(WindowOperation::Snapshot, hwnd, raw)?;
        let token = self.record_snapshot_identity_guard(hwnd, raw)?;
        identity = identity.with_validation_token(token);

        let result = (|| {
            let rect = self.window_rect(WindowOperation::Snapshot, hwnd, raw)?;
            let display_state = self.display_state(WindowOperation::Snapshot, hwnd, raw)?;
            let mut snapshot =
                WindowSnapshot::new(hwnd, rect, display_state).with_identity(identity);
            if self.window_has_atom_property(
                WindowOperation::Snapshot,
                hwnd,
                raw,
                b"_NET_WM_STATE",
                b"_NET_WM_STATE_ABOVE",
                "GetProperty(_NET_WM_STATE)",
            )? {
                snapshot =
                    snapshot.with_z_order_hint(ZOrderHint::new(LINUX_Z_ORDER_HINT_WAS_ABOVE));
            }
            Ok(snapshot)
        })();

        if result.is_err() {
            self.clear_snapshot_identity_guard(hwnd);
        }

        result
    }

    fn hide(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_snapshot_raw_window(WindowOperation::Hide, snapshot)?;
        self.unmap_raw(WindowOperation::Hide, hwnd, raw)
    }

    fn show(
        &mut self,
        snapshot: &WindowSnapshot,
        _activation: ActivationPolicy,
    ) -> Result<(), WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_snapshot_raw_window(WindowOperation::Show, snapshot)?;
        self.map_raw(WindowOperation::Show, hwnd, raw)?;
        self.set_net_wm_above(WindowOperation::Show, hwnd, raw, true)
    }

    fn set_position(
        &mut self,
        snapshot: &WindowSnapshot,
        rect: Rect,
    ) -> Result<(), WindowControlError> {
        let hwnd = snapshot.hwnd();
        let raw = self.ensure_snapshot_raw_window(WindowOperation::SetPosition, snapshot)?;
        self.set_net_wm_above(WindowOperation::SetPosition, hwnd, raw, true)?;
        self.configure_window_rect(WindowOperation::SetPosition, hwnd, raw, rect)
    }

    fn restore(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        let hwnd = snapshot.hwnd();
        let result = (|| {
            let raw = self.ensure_snapshot_raw_window(WindowOperation::Restore, snapshot)?;
            self.set_net_wm_maximized(WindowOperation::Restore, hwnd, raw, false)?;
            self.configure_window_rect(WindowOperation::Restore, hwnd, raw, snapshot.rect())?;
            self.set_net_wm_above(
                WindowOperation::Restore,
                hwnd,
                raw,
                linux_snapshot_was_above(snapshot),
            )?;

            match snapshot.display_state() {
                WindowDisplayState::Hidden => self.unmap_raw(WindowOperation::Restore, hwnd, raw),
                WindowDisplayState::Normal => self.map_raw(WindowOperation::Restore, hwnd, raw),
                WindowDisplayState::Minimized => {
                    self.map_raw(WindowOperation::Restore, hwnd, raw)?;
                    self.request_iconic_state(WindowOperation::Restore, hwnd, raw)
                }
                WindowDisplayState::Maximized => {
                    self.map_raw(WindowOperation::Restore, hwnd, raw)?;
                    self.set_net_wm_maximized(WindowOperation::Restore, hwnd, raw, true)
                }
            }
        })();

        if let Err(error) = result {
            return Err(WindowControlError::new(
                WindowOperation::Restore,
                Some(hwnd),
                USER_WINDOW_RESTORE_MESSAGE,
                Some(error.to_string()),
            ));
        }

        self.clear_snapshot_identity_guard(hwnd);
        Ok(())
    }

    fn restore_detached(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        self.restore(snapshot)
    }

    fn program_spec_for_snapshot(
        &mut self,
        snapshot: &WindowSnapshot,
        title: Option<String>,
    ) -> Result<ExternalProgramSpec, WindowControlError> {
        let raw = self.ensure_snapshot_raw_window(WindowOperation::InspectProgram, snapshot)?;
        let title = match title {
            Some(title) => Some(title),
            None => self
                .window_title_for_raw(snapshot.hwnd(), raw)
                .ok()
                .flatten(),
        };
        let process_id = self.process_id_for_window(raw)?.ok_or_else(|| {
            WindowControlError::new(
                WindowOperation::InspectProgram,
                Some(snapshot.hwnd()),
                USER_PROGRAM_ACCESS_MESSAGE,
                Some(String::from("X11 window has no _NET_WM_PID property.")),
            )
        })?;
        let executable_path =
            fs::read_link(format!("/proc/{process_id}/exe")).map_err(|error| {
                WindowControlError::new(
                    WindowOperation::InspectProgram,
                    Some(snapshot.hwnd()),
                    USER_PROGRAM_ACCESS_MESSAGE,
                    Some(format!("could not resolve /proc/{process_id}/exe: {error}")),
                )
            })?;

        ExternalProgramSpec::new_with_unix_executable_path_bytes(
            executable_path.as_os_str().as_bytes().to_vec(),
            title,
        )
        .map_err(|error| {
            WindowControlError::new(
                WindowOperation::InspectProgram,
                Some(snapshot.hwnd()),
                USER_PROGRAM_ACCESS_MESSAGE,
                Some(format!("invalid external program spec: {error}")),
            )
        })
    }
}

fn rect_from_geometry(geometry: &GetGeometryReply, left: i32, top: i32) -> Result<Rect, String> {
    Rect::new(
        left,
        top,
        i32::from(geometry.width),
        i32::from(geometry.height),
    )
    .map_err(|error| error.to_string())
}

fn display_state_from_x11_parts(
    wm_state: Option<u32>,
    map_state: MapState,
    maximized_horz: bool,
    maximized_vert: bool,
) -> WindowDisplayState {
    if wm_state == Some(WM_STATE_ICONIC) {
        WindowDisplayState::Minimized
    } else if map_state == MapState::UNMAPPED {
        WindowDisplayState::Hidden
    } else if maximized_horz && maximized_vert {
        WindowDisplayState::Maximized
    } else {
        WindowDisplayState::Normal
    }
}

fn linux_snapshot_was_above(snapshot: &WindowSnapshot) -> bool {
    snapshot
        .z_order_hint()
        .is_some_and(|hint| hint.value() == LINUX_Z_ORDER_HINT_WAS_ABOVE)
}

fn next_window_identity_token() -> usize {
    NEXT_WINDOW_IDENTITY_TOKEN
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            let next = current.wrapping_add(1);
            Some(if next == 0 { 1 } else { next })
        })
        .unwrap_or(1)
}

fn token_to_x11_words(token: usize) -> [u32; 2] {
    let token = token as u64;
    [token as u32, (token >> 32) as u32]
}

fn window_identity_property_name(hwnd: WindowHandle, token: usize) -> String {
    format!("j3GridDocker.WindowIdentity.{}.{}", hwnd.raw(), token)
}

fn x11_window_candidate(raw: Window, root: Window) -> Option<Window> {
    if raw == 0 || raw == root {
        None
    } else {
        Some(raw)
    }
}

fn x11_error(
    operation: WindowOperation,
    hwnd: Option<WindowHandle>,
    api: &'static str,
    error: impl std::fmt::Display,
) -> WindowControlError {
    let user_message = match operation {
        WindowOperation::Validate | WindowOperation::Snapshot => USER_WINDOW_ACCESS_MESSAGE,
        WindowOperation::InspectProgram => USER_PROGRAM_ACCESS_MESSAGE,
        WindowOperation::Hide | WindowOperation::Show | WindowOperation::SetPosition => {
            USER_WINDOW_MOVE_MESSAGE
        }
        WindowOperation::Restore => USER_WINDOW_RESTORE_MESSAGE,
    };
    WindowControlError::new(
        operation,
        hwnd,
        user_message,
        Some(format!("{api} failed: {error}")),
    )
}

fn splitter_overlay_error(api: &'static str, error: impl std::fmt::Display) -> WindowControlError {
    WindowControlError::new(
        WindowOperation::Validate,
        None,
        USER_SPLITTER_OVERLAY_MESSAGE,
        Some(format!("{api} failed: {error}")),
    )
}

fn reply_error_is_bad_window(error: &ReplyError) -> bool {
    matches!(
        error,
        ReplyError::X11Error(error) if error.error_kind == ErrorKind::Window
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::x11_utils::X11Error;

    #[test]
    fn display_state_from_x11_parts_matches_windows_priority() {
        assert_eq!(
            display_state_from_x11_parts(Some(WM_STATE_ICONIC), MapState::UNMAPPED, true, true),
            WindowDisplayState::Minimized
        );
        assert_eq!(
            display_state_from_x11_parts(None, MapState::UNMAPPED, true, true),
            WindowDisplayState::Hidden
        );
        assert_eq!(
            display_state_from_x11_parts(None, MapState::VIEWABLE, true, true),
            WindowDisplayState::Maximized
        );
        assert_eq!(
            display_state_from_x11_parts(None, MapState::VIEWABLE, true, false),
            WindowDisplayState::Normal
        );
    }

    #[test]
    fn linux_snapshot_was_above_uses_linux_z_order_hint() -> Result<(), Box<dyn std::error::Error>>
    {
        let hwnd = WindowHandle::new(100)?;
        let rect = Rect::new(0, 0, 320, 240)?;
        let snapshot = WindowSnapshot::new(hwnd, rect, WindowDisplayState::Normal);

        assert!(!linux_snapshot_was_above(&snapshot));
        assert!(linux_snapshot_was_above(&snapshot.with_z_order_hint(
            ZOrderHint::new(LINUX_Z_ORDER_HINT_WAS_ABOVE)
        )));
        let snapshot = WindowSnapshot::new(hwnd, rect, WindowDisplayState::Normal);
        assert!(!linux_snapshot_was_above(
            &snapshot.with_z_order_hint(ZOrderHint::new(99))
        ));

        Ok(())
    }

    #[test]
    fn identity_guard_property_name_and_token_words_are_stable()
    -> Result<(), Box<dyn std::error::Error>> {
        let hwnd = WindowHandle::new(0x0123_4567)?;
        let token = 0x1234_5678_9abc_def0usize;

        assert_eq!(
            window_identity_property_name(hwnd, token),
            "j3GridDocker.WindowIdentity.19088743.1311768467463790320"
        );
        assert_eq!(token_to_x11_words(token), [0x9abc_def0, 0x1234_5678]);

        Ok(())
    }

    #[test]
    fn x11_window_candidate_rejects_empty_and_root_windows() {
        assert_eq!(x11_window_candidate(0, 100), None);
        assert_eq!(x11_window_candidate(100, 100), None);
        assert_eq!(x11_window_candidate(101, 100), Some(101));
    }

    #[test]
    fn reply_error_is_bad_window_matches_x11_window_errors() {
        let error = ReplyError::X11Error(x11_error_with_kind(ErrorKind::Window));

        assert!(reply_error_is_bad_window(&error));
    }

    #[test]
    fn reply_error_is_bad_window_rejects_other_x11_errors() {
        let error = ReplyError::X11Error(x11_error_with_kind(ErrorKind::Access));

        assert!(!reply_error_is_bad_window(&error));
    }

    fn x11_error_with_kind(error_kind: ErrorKind) -> X11Error {
        X11Error {
            error_kind,
            error_code: 0,
            sequence: 0,
            bad_value: 0,
            minor_opcode: 0,
            major_opcode: 0,
            extension_name: None,
            request_name: None,
        }
    }
}
