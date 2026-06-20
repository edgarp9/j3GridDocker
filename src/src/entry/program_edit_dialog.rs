use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SetLastError,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, HDC, SetBkMode, TRANSPARENT,
    UpdateWindow,
};
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL,
    GWLP_USERDATA, GetClientRect, GetMessageW, GetScrollInfo, GetWindowLongPtrW, IsWindow,
    LoadCursorW, MB_ICONINFORMATION, MB_OK, MINMAXINFO, MSG, MessageBoxW, PostQuitMessage,
    RegisterClassW, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION,
    SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_GETMINMAXINFO, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WM_SIZE,
    WM_VSCROLL, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_DLGMODALFRAME, WS_EX_WINDOWEDGE,
    WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

use crate::domain::{
    DomainError, ExternalProgramSpec, UiLanguage, normalize_tab_preset_name,
    validate_tab_preset_name,
};

use super::{
    EntryError, ProgramArgumentsParseError, centered_window_position, cleanup_text_input_dialog,
    control_id, low_word, module_handle, parse_program_arguments, read_window_text,
    tab_preset_program_label, ui_text, wide_null,
};

const PROGRAM_EDIT_DIALOG_CLASS_NAME: &str = "j3GridDocker.ProgramEditDialog";
const PROGRAM_EDIT_DIALOG_OK_ID: u16 = 1;
const PROGRAM_EDIT_DIALOG_CANCEL_ID: u16 = 2;
const PROGRAM_EDIT_DIALOG_CLIENT_WIDTH: i32 = 640;
const PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT: i32 = 590;
const PROGRAM_EDIT_DIALOG_MIN_CLIENT_WIDTH: i32 = 560;
const PROGRAM_EDIT_DIALOG_MIN_CLIENT_HEIGHT: i32 = 360;
const PROGRAM_EDIT_DIALOG_VIEWPORT_X: i32 = 0;
const PROGRAM_EDIT_DIALOG_VIEWPORT_Y: i32 = 116;
const PROGRAM_EDIT_DIALOG_VIEWPORT_WIDTH: i32 = 606;
const PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT: i32 = 388;
const PROGRAM_EDIT_DIALOG_MIN_VIEWPORT_HEIGHT: i32 = 96;
const PROGRAM_EDIT_DIALOG_ROW_HEIGHT: i32 = 104;
const PROGRAM_EDIT_DIALOG_ROW_TOP: i32 = 8;
const PROGRAM_EDIT_DIALOG_SCROLL_LINE: i32 = 28;
const PROGRAM_EDIT_DIALOG_TEXT_X: i32 = 14;
const PROGRAM_EDIT_DIALOG_TEXT_RIGHT_MARGIN: i32 = 48;
const PROGRAM_EDIT_DIALOG_PRESET_NAME_EDIT_X: i32 = 150;
const PROGRAM_EDIT_DIALOG_PRESET_NAME_EDIT_RIGHT_MARGIN: i32 = 60;
const PROGRAM_EDIT_DIALOG_VIEWPORT_RIGHT_MARGIN: i32 = 34;
const PROGRAM_EDIT_DIALOG_CONTENT_BUTTON_GAP: i32 = 26;
const PROGRAM_EDIT_DIALOG_BUTTON_WIDTH: i32 = 78;
const PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT: i32 = 28;
const PROGRAM_EDIT_DIALOG_BUTTON_GAP: i32 = 8;
const PROGRAM_EDIT_DIALOG_BUTTON_RIGHT_MARGIN: i32 = 46;
const PROGRAM_EDIT_DIALOG_BUTTON_BOTTOM_MARGIN: i32 = 32;
const PROGRAM_EDIT_DIALOG_ROW_LABEL_X: i32 = 24;
const PROGRAM_EDIT_DIALOG_ROW_EDIT_X: i32 = 158;
const PROGRAM_EDIT_DIALOG_ROW_EDIT_RIGHT_MARGIN: i32 = 26;
const PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH: i32 = 160;
const PROGRAM_EDIT_DIALOG_ROW_OFFSCREEN_Y: i32 = -(PROGRAM_EDIT_DIALOG_ROW_HEIGHT * 2);
const PROGRAM_EDIT_DIALOG_STYLE: u32 =
    WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_VISIBLE | WS_VSCROLL;
const PROGRAM_EDIT_DIALOG_EX_STYLE: u32 = WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE;

pub(super) struct TabPresetEditDialogResult {
    name: String,
    programs: Vec<ExternalProgramSpec>,
}

impl TabPresetEditDialogResult {
    pub(super) fn into_parts(self) -> (String, Vec<ExternalProgramSpec>) {
        (self.name, self.programs)
    }
}

#[derive(Debug, Clone)]
struct ProgramEditDialogRow {
    title: Option<String>,
    header: Vec<u16>,
    executable_initial: Vec<u16>,
    arguments_initial: Vec<u16>,
    header_hwnd: HWND,
    executable_label_hwnd: HWND,
    executable_edit_hwnd: HWND,
    arguments_label_hwnd: HWND,
    arguments_edit_hwnd: HWND,
}

impl ProgramEditDialogRow {
    fn new(
        index: usize,
        total: usize,
        program: &ExternalProgramSpec,
        language: UiLanguage,
    ) -> Self {
        let label = tab_preset_program_label(program);
        let header = if language == UiLanguage::English {
            format!("Program {index}/{total}: {label}")
        } else {
            format!("프로그램 {index}/{total}: {label}")
        };
        Self {
            title: program.title().map(str::to_owned),
            header: wide_null(&header),
            executable_initial: wide_null(program.executable_path()),
            arguments_initial: wide_null(&super::format_program_arguments(program.arguments())),
            header_hwnd: null_mut(),
            executable_label_hwnd: null_mut(),
            executable_edit_hwnd: null_mut(),
            arguments_label_hwnd: null_mut(),
            arguments_edit_hwnd: null_mut(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProgramEditDialogRowRange {
    start: usize,
    end: usize,
}

impl ProgramEditDialogRowRange {
    fn empty(at: usize) -> Self {
        Self { start: at, end: at }
    }

    fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProgramEditDialogRowLayoutCache {
    visible_range: ProgramEditDialogRowRange,
    scroll_pos: i32,
    content_width: i32,
}

struct ProgramEditDialogState {
    language: UiLanguage,
    title: Vec<u16>,
    instruction: Vec<u16>,
    preset_name_label: Vec<u16>,
    preset_name_initial: Vec<u16>,
    executable_label: Vec<u16>,
    arguments_label: Vec<u16>,
    ok_label: Vec<u16>,
    cancel_label: Vec<u16>,
    rows: Vec<ProgramEditDialogRow>,
    title_hwnd: HWND,
    instruction_hwnd: HWND,
    preset_name_label_hwnd: HWND,
    preset_name_edit_hwnd: HWND,
    content_hwnd: HWND,
    ok_hwnd: HWND,
    cancel_hwnd: HWND,
    result: Option<TabPresetEditDialogResult>,
    read_error: Option<u32>,
    viewport_height: i32,
    scroll_pos: i32,
    row_layout: Option<ProgramEditDialogRowLayoutCache>,
    done: bool,
}

impl ProgramEditDialogState {
    fn new(
        language: UiLanguage,
        preset_name: &str,
        programs: &[ExternalProgramSpec],
        ok_label: &str,
        cancel_label: &str,
    ) -> Self {
        let total = programs.len();
        let title = ui_text(language, "Tab preset edit", "탭 프리셋 편집");
        let instruction = if total == 0 {
            ui_text(
                language,
                "Edit the preset name.",
                "프리셋 이름을 편집합니다.",
            )
            .to_owned()
        } else if language == UiLanguage::English {
            format!("Edit executable paths and arguments for {total} docked program(s).")
        } else {
            format!("도킹된 프로그램 {total}개의 실행 파일과 인수를 편집합니다.")
        };
        Self {
            language,
            title: wide_null(title),
            instruction: wide_null(&instruction),
            preset_name_label: wide_null(ui_text(language, "Preset name", "프리셋 이름")),
            preset_name_initial: wide_null(preset_name),
            executable_label: wide_null(ui_text(language, "Executable path", "실행 파일 경로")),
            arguments_label: wide_null(ui_text(language, "Arguments", "실행 인수")),
            ok_label: wide_null(ok_label),
            cancel_label: wide_null(cancel_label),
            rows: programs
                .iter()
                .enumerate()
                .map(|(index, program)| {
                    ProgramEditDialogRow::new(index + 1, total, program, language)
                })
                .collect(),
            title_hwnd: null_mut(),
            instruction_hwnd: null_mut(),
            preset_name_label_hwnd: null_mut(),
            preset_name_edit_hwnd: null_mut(),
            content_hwnd: null_mut(),
            ok_hwnd: null_mut(),
            cancel_hwnd: null_mut(),
            result: None,
            read_error: None,
            viewport_height: PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT,
            scroll_pos: 0,
            row_layout: None,
            done: false,
        }
    }

    fn content_height(&self) -> i32 {
        program_edit_dialog_content_height(self.rows.len())
    }
}

unsafe extern "system" fn program_edit_dialog_proc(
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

        let state = unsafe { (*create).lpCreateParams as *mut ProgramEditDialogState };
        if state.is_null() {
            return 0;
        }

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        return 1;
    }

    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ProgramEditDialogState };
    match message {
        WM_GETMINMAXINFO => {
            set_program_edit_dialog_min_track_size(lparam);
            0
        }
        WM_CREATE => {
            if state.is_null() {
                return -1;
            }

            if create_program_edit_dialog_controls(hwnd, state) {
                0
            } else {
                -1
            }
        }
        WM_COMMAND => {
            if !state.is_null() {
                match low_word(wparam) {
                    PROGRAM_EDIT_DIALOG_OK_ID => accept_program_edit_dialog(hwnd, state),
                    PROGRAM_EDIT_DIALOG_CANCEL_ID => cancel_program_edit_dialog(hwnd, state),
                    _ => {}
                }
            }
            0
        }
        WM_SIZE => {
            if !state.is_null() {
                resize_program_edit_dialog(hwnd, state);
            }
            0
        }
        WM_VSCROLL => {
            if !state.is_null() {
                handle_program_edit_dialog_scroll(hwnd, state, low_word(wparam));
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
                cancel_program_edit_dialog(hwnd, state);
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

fn create_program_edit_dialog_controls(hwnd: HWND, state: *mut ProgramEditDialogState) -> bool {
    set_dialog_font(hwnd);
    let static_class = wide_null("STATIC");
    let edit_class = wide_null("EDIT");
    let button_class = wide_null("BUTTON");

    let title = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).title.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            14,
            12,
            578,
            22,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if title.is_null() {
        return false;
    }
    set_dialog_font(title);
    unsafe {
        (*state).title_hwnd = title;
    }

    let instruction = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).instruction.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            14,
            42,
            578,
            20,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if instruction.is_null() {
        return false;
    }
    set_dialog_font(instruction);
    unsafe {
        (*state).instruction_hwnd = instruction;
    }

    let preset_name_label = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).preset_name_label.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            14,
            74,
            118,
            18,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if preset_name_label.is_null() {
        return false;
    }
    set_dialog_font(preset_name_label);
    unsafe {
        (*state).preset_name_label_hwnd = preset_name_label;
    }

    let preset_name_edit = unsafe {
        CreateWindowExW(
            0,
            edit_class.as_ptr(),
            (*state).preset_name_initial.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            150,
            70,
            430,
            24,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if preset_name_edit.is_null() {
        return false;
    }
    set_dialog_font(preset_name_edit);
    unsafe {
        (*state).preset_name_edit_hwnd = preset_name_edit;
    }

    let content = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            PROGRAM_EDIT_DIALOG_VIEWPORT_X,
            PROGRAM_EDIT_DIALOG_VIEWPORT_Y,
            PROGRAM_EDIT_DIALOG_VIEWPORT_WIDTH,
            PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT,
            hwnd,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if content.is_null() {
        return false;
    }
    set_dialog_font(content);
    unsafe {
        (*state).content_hwnd = content;
    }

    let mut first_edit: HWND = null_mut();
    for index in 0..unsafe { (*state).rows.len() } {
        if !create_program_edit_dialog_row_controls(
            content,
            state,
            index,
            &static_class,
            &edit_class,
        ) {
            return false;
        }
        if first_edit.is_null() {
            first_edit = unsafe { (&(*state).rows)[index].executable_edit_hwnd };
        }
    }

    let ok = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            (*state).ok_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            430,
            program_edit_dialog_button_y(PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT),
            PROGRAM_EDIT_DIALOG_BUTTON_WIDTH,
            PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT,
            hwnd,
            control_id(PROGRAM_EDIT_DIALOG_OK_ID),
            null_mut(),
            null_mut(),
        )
    };
    if ok.is_null() {
        return false;
    }
    set_dialog_font(ok);
    unsafe {
        (*state).ok_hwnd = ok;
    }

    let cancel = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            (*state).cancel_label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            516,
            program_edit_dialog_button_y(PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT),
            PROGRAM_EDIT_DIALOG_BUTTON_WIDTH,
            PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT,
            hwnd,
            control_id(PROGRAM_EDIT_DIALOG_CANCEL_ID),
            null_mut(),
            null_mut(),
        )
    };
    if cancel.is_null() {
        return false;
    }
    set_dialog_font(cancel);
    unsafe {
        (*state).cancel_hwnd = cancel;
    }

    resize_program_edit_dialog(hwnd, state);
    let focus = if unsafe { (*state).preset_name_edit_hwnd }.is_null() {
        first_edit
    } else {
        unsafe { (*state).preset_name_edit_hwnd }
    };
    if !focus.is_null() {
        unsafe { SetFocus(focus) };
    }
    true
}

fn create_program_edit_dialog_row_controls(
    parent: HWND,
    state: *mut ProgramEditDialogState,
    index: usize,
    static_class: &[u16],
    edit_class: &[u16],
) -> bool {
    let row = unsafe { &mut (&mut (*state).rows)[index] };
    let y = PROGRAM_EDIT_DIALOG_ROW_OFFSCREEN_Y;
    let header = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            row.header.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            14,
            y,
            560,
            18,
            parent,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if header.is_null() {
        return false;
    }
    set_dialog_font(header);
    row.header_hwnd = header;

    let executable_label = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).executable_label.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            24,
            y + 28,
            118,
            18,
            parent,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if executable_label.is_null() {
        return false;
    }
    set_dialog_font(executable_label);
    row.executable_label_hwnd = executable_label;

    let executable_edit = unsafe {
        CreateWindowExW(
            0,
            edit_class.as_ptr(),
            row.executable_initial.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            150,
            y + 24,
            430,
            24,
            parent,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if executable_edit.is_null() {
        return false;
    }
    set_dialog_font(executable_edit);
    row.executable_edit_hwnd = executable_edit;

    let arguments_label = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            (*state).arguments_label.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            24,
            y + 62,
            118,
            18,
            parent,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if arguments_label.is_null() {
        return false;
    }
    set_dialog_font(arguments_label);
    row.arguments_label_hwnd = arguments_label;

    let arguments_edit = unsafe {
        CreateWindowExW(
            0,
            edit_class.as_ptr(),
            row.arguments_initial.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            150,
            y + 58,
            430,
            24,
            parent,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if arguments_edit.is_null() {
        return false;
    }
    set_dialog_font(arguments_edit);
    row.arguments_edit_hwnd = arguments_edit;

    true
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

fn accept_program_edit_dialog(hwnd: HWND, state: *mut ProgramEditDialogState) {
    let preset_name = match read_window_text(unsafe { (*state).preset_name_edit_hwnd }) {
        Ok(name) => normalize_tab_preset_name(&name),
        Err(last_error) => {
            unsafe {
                (*state).read_error = Some(last_error);
                (*state).done = true;
                DestroyWindow(hwnd);
            }
            return;
        }
    };
    if let Err(error) = validate_tab_preset_name(&preset_name) {
        show_tab_preset_edit_validation(
            hwnd,
            state,
            tab_preset_name_validation_message(unsafe { (*state).language }, error),
            unsafe { (*state).preset_name_edit_hwnd },
        );
        return;
    }

    let mut programs = Vec::with_capacity(unsafe { (*state).rows.len() });
    for index in 0..unsafe { (*state).rows.len() } {
        let row = unsafe { &(&(*state).rows)[index] };
        let executable = match read_window_text(row.executable_edit_hwnd) {
            Ok(executable) => executable,
            Err(last_error) => {
                unsafe {
                    (*state).read_error = Some(last_error);
                    (*state).done = true;
                    DestroyWindow(hwnd);
                }
                return;
            }
        };
        let arguments_text = match read_window_text(row.arguments_edit_hwnd) {
            Ok(arguments_text) => arguments_text,
            Err(last_error) => {
                unsafe {
                    (*state).read_error = Some(last_error);
                    (*state).done = true;
                    DestroyWindow(hwnd);
                }
                return;
            }
        };
        let arguments = match parse_program_arguments(&arguments_text) {
            Ok(arguments) => arguments,
            Err(error) => {
                show_program_edit_dialog_validation(
                    hwnd,
                    state,
                    index,
                    ProgramEditDialogValidation::Arguments(error),
                    row.arguments_edit_hwnd,
                );
                return;
            }
        };
        match ExternalProgramSpec::new_with_arguments(executable, arguments, row.title.clone()) {
            Ok(program) => programs.push(program),
            Err(error) => {
                let focus = if matches!(error, DomainError::InvalidProgramArgument) {
                    row.arguments_edit_hwnd
                } else {
                    row.executable_edit_hwnd
                };
                show_program_edit_dialog_validation(
                    hwnd,
                    state,
                    index,
                    ProgramEditDialogValidation::Domain(error),
                    focus,
                );
                return;
            }
        }
    }

    unsafe {
        (*state).result = Some(TabPresetEditDialogResult {
            name: preset_name,
            programs,
        });
        (*state).done = true;
        DestroyWindow(hwnd);
    }
}

fn cancel_program_edit_dialog(hwnd: HWND, state: *mut ProgramEditDialogState) {
    unsafe {
        (*state).result = None;
        (*state).done = true;
        DestroyWindow(hwnd);
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ProgramEditDialogValidation {
    Arguments(ProgramArgumentsParseError),
    Domain(DomainError),
}

fn show_program_edit_dialog_validation(
    hwnd: HWND,
    state: *mut ProgramEditDialogState,
    row_index: usize,
    validation: ProgramEditDialogValidation,
    focus: HWND,
) {
    scroll_program_edit_dialog_to_row(hwnd, state, row_index);
    let language = unsafe { (*state).language };
    let message = program_edit_dialog_validation_message(language, row_index + 1, validation);
    show_tab_preset_edit_validation(hwnd, state, message, focus);
}

fn show_tab_preset_edit_validation(
    hwnd: HWND,
    state: *mut ProgramEditDialogState,
    message: String,
    focus: HWND,
) {
    let language = unsafe { (*state).language };
    let title = ui_text(language, "Tab preset edit", "탭 프리셋 편집");
    let message = wide_null(&message);
    let title = wide_null(title);
    unsafe {
        MessageBoxW(
            hwnd,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
        SetFocus(focus);
    }
}

fn tab_preset_name_validation_message(language: UiLanguage, error: DomainError) -> String {
    if language == UiLanguage::English {
        match error {
            DomainError::EmptyTabPresetName => "Tab preset name cannot be empty.".to_owned(),
            _ => "Tab preset name is invalid.".to_owned(),
        }
    } else {
        error.user_message().to_owned()
    }
}

fn program_edit_dialog_validation_message(
    language: UiLanguage,
    program_number: usize,
    validation: ProgramEditDialogValidation,
) -> String {
    let message = match validation {
        ProgramEditDialogValidation::Arguments(error) => error.user_message(language),
        ProgramEditDialogValidation::Domain(error) => {
            if language == UiLanguage::English {
                match error {
                    DomainError::EmptyProgramExecutablePath => {
                        "Program executable path cannot be empty."
                    }
                    DomainError::InvalidProgramArgument => {
                        "Program argument contains an invalid character."
                    }
                    _ => "Program information is invalid.",
                }
            } else {
                error.user_message()
            }
        }
    };

    if language == UiLanguage::English {
        format!("Program {program_number}: {message}")
    } else {
        format!("프로그램 {program_number}: {message}")
    }
}

fn set_program_edit_dialog_min_track_size(lparam: LPARAM) {
    if lparam == 0 {
        return;
    }

    let (min_width, min_height) = program_edit_dialog_window_size_for_client(
        PROGRAM_EDIT_DIALOG_MIN_CLIENT_WIDTH,
        PROGRAM_EDIT_DIALOG_MIN_CLIENT_HEIGHT,
    );
    let info = lparam as *mut MINMAXINFO;
    unsafe {
        (*info).ptMinTrackSize.x = min_width;
        (*info).ptMinTrackSize.y = min_height;
    }
}

fn resize_program_edit_dialog(hwnd: HWND, state: *mut ProgramEditDialogState) {
    let (client_width, client_height) = program_edit_dialog_client_size(hwnd);
    let client_width = client_width.max(PROGRAM_EDIT_DIALOG_MIN_CLIENT_WIDTH);
    let client_height = client_height.max(PROGRAM_EDIT_DIALOG_MIN_CLIENT_HEIGHT);
    let text_width = client_width
        .saturating_sub(PROGRAM_EDIT_DIALOG_TEXT_X)
        .saturating_sub(PROGRAM_EDIT_DIALOG_TEXT_RIGHT_MARGIN)
        .max(PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH);
    let preset_name_width = client_width
        .saturating_sub(PROGRAM_EDIT_DIALOG_PRESET_NAME_EDIT_X)
        .saturating_sub(PROGRAM_EDIT_DIALOG_PRESET_NAME_EDIT_RIGHT_MARGIN)
        .max(PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH);
    let viewport_width = program_edit_dialog_viewport_width_for_client(client_width);
    let viewport_height = program_edit_dialog_viewport_height_for_client(client_height);
    let button_y = program_edit_dialog_button_y(client_height);
    let cancel_x = client_width
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_RIGHT_MARGIN)
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_WIDTH);
    let ok_x = cancel_x
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_GAP)
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_WIDTH);

    unsafe {
        (*state).viewport_height = viewport_height;
        set_child_position(
            (*state).title_hwnd,
            PROGRAM_EDIT_DIALOG_TEXT_X,
            12,
            text_width,
            22,
        );
        set_child_position(
            (*state).instruction_hwnd,
            PROGRAM_EDIT_DIALOG_TEXT_X,
            42,
            text_width,
            20,
        );
        set_child_position(
            (*state).preset_name_label_hwnd,
            PROGRAM_EDIT_DIALOG_TEXT_X,
            74,
            118,
            18,
        );
        set_child_position(
            (*state).preset_name_edit_hwnd,
            PROGRAM_EDIT_DIALOG_PRESET_NAME_EDIT_X,
            70,
            preset_name_width,
            24,
        );
        set_child_position(
            (*state).content_hwnd,
            PROGRAM_EDIT_DIALOG_VIEWPORT_X,
            PROGRAM_EDIT_DIALOG_VIEWPORT_Y,
            viewport_width,
            viewport_height,
        );
        set_child_position(
            (*state).ok_hwnd,
            ok_x,
            button_y,
            PROGRAM_EDIT_DIALOG_BUTTON_WIDTH,
            PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT,
        );
        set_child_position(
            (*state).cancel_hwnd,
            cancel_x,
            button_y,
            PROGRAM_EDIT_DIALOG_BUTTON_WIDTH,
            PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT,
        );
    }

    update_program_edit_dialog_scrollbar(hwnd, state);
    position_program_edit_dialog_rows(state);
}

fn program_edit_dialog_client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return (
            PROGRAM_EDIT_DIALOG_CLIENT_WIDTH,
            PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT,
        );
    }
    (
        rect.right.saturating_sub(rect.left),
        rect.bottom.saturating_sub(rect.top),
    )
}

fn program_edit_dialog_viewport_width_for_client(client_width: i32) -> i32 {
    client_width
        .saturating_sub(PROGRAM_EDIT_DIALOG_VIEWPORT_X)
        .saturating_sub(PROGRAM_EDIT_DIALOG_VIEWPORT_RIGHT_MARGIN)
        .max(PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH)
}

fn program_edit_dialog_viewport_height_for_client(client_height: i32) -> i32 {
    program_edit_dialog_button_y(client_height)
        .saturating_sub(PROGRAM_EDIT_DIALOG_VIEWPORT_Y)
        .saturating_sub(PROGRAM_EDIT_DIALOG_CONTENT_BUTTON_GAP)
        .max(PROGRAM_EDIT_DIALOG_MIN_VIEWPORT_HEIGHT)
}

fn program_edit_dialog_button_y(client_height: i32) -> i32 {
    client_height
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_BOTTOM_MARGIN)
        .saturating_sub(PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT)
}

fn update_program_edit_dialog_scrollbar(hwnd: HWND, state: *mut ProgramEditDialogState) {
    let content_height = unsafe { (*state).content_height() };
    let viewport_height = unsafe { (*state).viewport_height.max(1) };
    let max_scroll =
        program_edit_dialog_max_scroll_position_for_viewport(content_height, viewport_height);
    unsafe {
        (*state).scroll_pos = (*state).scroll_pos.clamp(0, max_scroll);
    }
    let mut info = unsafe { zeroed::<SCROLLINFO>() };
    info.cbSize = size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
    info.nMin = 0;
    info.nMax = content_height.saturating_sub(1);
    info.nPage = viewport_height as u32;
    info.nPos = unsafe { (*state).scroll_pos };
    unsafe {
        SetScrollInfo(hwnd, SB_VERT, &info, 1);
    }
}

fn handle_program_edit_dialog_scroll(hwnd: HWND, state: *mut ProgramEditDialogState, request: u16) {
    let content_height = unsafe { (*state).content_height() };
    let viewport_height = unsafe { (*state).viewport_height.max(1) };
    let max_scroll =
        program_edit_dialog_max_scroll_position_for_viewport(content_height, viewport_height);
    let current = unsafe { (*state).scroll_pos };
    let target = match i32::from(request) {
        SB_LINEUP => current.saturating_sub(PROGRAM_EDIT_DIALOG_SCROLL_LINE),
        SB_LINEDOWN => current.saturating_add(PROGRAM_EDIT_DIALOG_SCROLL_LINE),
        SB_PAGEUP => current.saturating_sub(viewport_height),
        SB_PAGEDOWN => current.saturating_add(viewport_height),
        SB_TOP => 0,
        SB_BOTTOM => max_scroll,
        SB_THUMBTRACK | SB_THUMBPOSITION => {
            let mut info = unsafe { zeroed::<SCROLLINFO>() };
            info.cbSize = size_of::<SCROLLINFO>() as u32;
            info.fMask = SIF_TRACKPOS;
            unsafe {
                GetScrollInfo(hwnd, SB_VERT, &mut info);
            }
            info.nTrackPos
        }
        _ => current,
    };
    set_program_edit_dialog_scroll_pos(hwnd, state, target);
}

fn scroll_program_edit_dialog_to_row(
    hwnd: HWND,
    state: *mut ProgramEditDialogState,
    row_index: usize,
) {
    let row_top = PROGRAM_EDIT_DIALOG_ROW_TOP
        .saturating_add((row_index as i32).saturating_mul(PROGRAM_EDIT_DIALOG_ROW_HEIGHT));
    let row_bottom = row_top.saturating_add(PROGRAM_EDIT_DIALOG_ROW_HEIGHT);
    let current = unsafe { (*state).scroll_pos };
    let viewport_height = unsafe { (*state).viewport_height.max(1) };
    let viewport_bottom = current.saturating_add(viewport_height);
    let target = if row_top < current {
        row_top
    } else if row_bottom > viewport_bottom {
        row_bottom.saturating_sub(viewport_height)
    } else {
        current
    };
    set_program_edit_dialog_scroll_pos(hwnd, state, target);
}

fn set_program_edit_dialog_scroll_pos(hwnd: HWND, state: *mut ProgramEditDialogState, target: i32) {
    let content_height = unsafe { (*state).content_height() };
    let viewport_height = unsafe { (*state).viewport_height.max(1) };
    let max_scroll =
        program_edit_dialog_max_scroll_position_for_viewport(content_height, viewport_height);
    let target = target.clamp(0, max_scroll);
    if target == unsafe { (*state).scroll_pos } {
        return;
    }

    unsafe {
        (*state).scroll_pos = target;
    }
    update_program_edit_dialog_scrollbar(hwnd, state);
    position_program_edit_dialog_rows(state);
}

fn position_program_edit_dialog_rows(state: *mut ProgramEditDialogState) {
    let scroll_pos = unsafe { (*state).scroll_pos };
    let viewport_height = unsafe { (*state).viewport_height.max(1) };
    let row_count = unsafe { (*state).rows.len() };
    let content_width = program_edit_dialog_child_width(
        unsafe { (*state).content_hwnd },
        PROGRAM_EDIT_DIALOG_VIEWPORT_WIDTH,
    );
    let visible_range =
        program_edit_dialog_visible_row_range(row_count, scroll_pos, viewport_height);
    let layout = ProgramEditDialogRowLayoutCache {
        visible_range,
        scroll_pos,
        content_width,
    };
    let previous_layout = unsafe { (*state).row_layout };
    if previous_layout == Some(layout) {
        return;
    }

    let header_width = content_width
        .saturating_sub(28)
        .max(PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH);
    let edit_width = content_width
        .saturating_sub(PROGRAM_EDIT_DIALOG_ROW_EDIT_X)
        .saturating_sub(PROGRAM_EDIT_DIALOG_ROW_EDIT_RIGHT_MARGIN)
        .max(PROGRAM_EDIT_DIALOG_MIN_EDIT_WIDTH);

    for range in program_edit_dialog_row_update_ranges(
        previous_layout.map(|layout| layout.visible_range),
        visible_range,
    )
    .iter()
    .copied()
    {
        position_program_edit_dialog_row_range(state, range, scroll_pos, header_width, edit_width);
    }

    unsafe {
        (*state).row_layout = Some(layout);
    }
}

fn position_program_edit_dialog_row_range(
    state: *mut ProgramEditDialogState,
    range: ProgramEditDialogRowRange,
    scroll_pos: i32,
    header_width: i32,
    edit_width: i32,
) {
    if range.is_empty() {
        return;
    }

    for index in range.start..range.end {
        let y = program_edit_dialog_row_y(index, scroll_pos);
        let row = unsafe { &(&(*state).rows)[index] };
        position_program_edit_dialog_row(row, y, header_width, edit_width);
    }
}

fn position_program_edit_dialog_row(
    row: &ProgramEditDialogRow,
    y: i32,
    header_width: i32,
    edit_width: i32,
) {
    set_child_position(row.header_hwnd, 14, y, header_width, 18);
    set_child_position(
        row.executable_label_hwnd,
        PROGRAM_EDIT_DIALOG_ROW_LABEL_X,
        y + 28,
        118,
        18,
    );
    set_child_position(
        row.executable_edit_hwnd,
        PROGRAM_EDIT_DIALOG_ROW_EDIT_X,
        y + 24,
        edit_width,
        24,
    );
    set_child_position(
        row.arguments_label_hwnd,
        PROGRAM_EDIT_DIALOG_ROW_LABEL_X,
        y + 62,
        118,
        18,
    );
    set_child_position(
        row.arguments_edit_hwnd,
        PROGRAM_EDIT_DIALOG_ROW_EDIT_X,
        y + 58,
        edit_width,
        24,
    );
}

fn program_edit_dialog_row_y(index: usize, scroll_pos: i32) -> i32 {
    PROGRAM_EDIT_DIALOG_ROW_TOP
        .saturating_add((index as i32).saturating_mul(PROGRAM_EDIT_DIALOG_ROW_HEIGHT))
        .saturating_sub(scroll_pos)
}

fn program_edit_dialog_visible_row_range(
    row_count: usize,
    scroll_pos: i32,
    viewport_height: i32,
) -> ProgramEditDialogRowRange {
    if row_count == 0 {
        return ProgramEditDialogRowRange::empty(0);
    }

    let viewport_top = scroll_pos.max(0);
    let viewport_bottom = viewport_top.saturating_add(viewport_height.max(1));
    let start = if viewport_top <= PROGRAM_EDIT_DIALOG_ROW_TOP {
        0
    } else {
        ((viewport_top - PROGRAM_EDIT_DIALOG_ROW_TOP) / PROGRAM_EDIT_DIALOG_ROW_HEIGHT) as usize
    }
    .min(row_count);
    let end = if viewport_bottom <= PROGRAM_EDIT_DIALOG_ROW_TOP {
        0
    } else {
        ((viewport_bottom - PROGRAM_EDIT_DIALOG_ROW_TOP)
            .saturating_add(PROGRAM_EDIT_DIALOG_ROW_HEIGHT - 1)
            / PROGRAM_EDIT_DIALOG_ROW_HEIGHT) as usize
    }
    .min(row_count);

    ProgramEditDialogRowRange {
        start,
        end: end.max(start),
    }
}

fn program_edit_dialog_row_update_ranges(
    previous: Option<ProgramEditDialogRowRange>,
    current: ProgramEditDialogRowRange,
) -> [ProgramEditDialogRowRange; 3] {
    let Some(previous) = previous else {
        return [
            ProgramEditDialogRowRange::empty(current.start),
            ProgramEditDialogRowRange::empty(current.start),
            current,
        ];
    };

    let leaving_before = if previous.start < current.start {
        ProgramEditDialogRowRange {
            start: previous.start,
            end: previous.end.min(current.start),
        }
    } else {
        ProgramEditDialogRowRange::empty(previous.start)
    };
    let leaving_after = if previous.end > current.end {
        ProgramEditDialogRowRange {
            start: previous.start.max(current.end),
            end: previous.end,
        }
    } else {
        ProgramEditDialogRowRange::empty(previous.end)
    };

    [leaving_before, leaving_after, current]
}

fn program_edit_dialog_child_width(hwnd: HWND, fallback: i32) -> i32 {
    if hwnd.is_null() {
        return fallback;
    }

    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return fallback;
    }

    rect.right.saturating_sub(rect.left).max(1)
}

fn set_child_position(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    if hwnd.is_null() {
        return;
    }

    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub(super) fn program_edit_dialog_content_height(program_count: usize) -> i32 {
    PROGRAM_EDIT_DIALOG_ROW_TOP
        .saturating_mul(2)
        .saturating_add((program_count as i32).saturating_mul(PROGRAM_EDIT_DIALOG_ROW_HEIGHT))
}

#[cfg(test)]
pub(super) fn program_edit_dialog_max_scroll_position(content_height: i32) -> i32 {
    program_edit_dialog_max_scroll_position_for_viewport(
        content_height,
        PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT,
    )
}

fn program_edit_dialog_max_scroll_position_for_viewport(
    content_height: i32,
    viewport_height: i32,
) -> i32 {
    content_height.saturating_sub(viewport_height.max(1)).max(0)
}

#[cfg(test)]
pub(super) fn program_edit_dialog_client_height() -> i32 {
    PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT
}

#[cfg(test)]
pub(super) fn program_edit_dialog_button_bottom() -> i32 {
    program_edit_dialog_button_y(PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT)
        .saturating_add(PROGRAM_EDIT_DIALOG_BUTTON_HEIGHT)
}

#[cfg(test)]
pub(super) fn program_edit_dialog_min_client_height() -> i32 {
    PROGRAM_EDIT_DIALOG_MIN_CLIENT_HEIGHT
}

#[cfg(test)]
pub(super) fn program_edit_dialog_viewport_height_for_test(client_height: i32) -> i32 {
    program_edit_dialog_viewport_height_for_client(client_height)
}

fn cleanup_program_edit_dialog_create_failure(hwnd: HWND) {
    // SAFETY: `hwnd` was just returned by CreateWindowExW on this thread, and the
    // Box-owned dialog state is still alive. Clear GWLP_USERDATA before dropping
    // the Box so a surviving window cannot keep a stale raw pointer.
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        DestroyWindow(hwnd);
    }
}

pub(super) fn prompt_tab_preset_edit(
    owner: HWND,
    title: &str,
    language: UiLanguage,
    preset_name: &str,
    programs: &[ExternalProgramSpec],
    ok_label: &str,
    cancel_label: &str,
) -> Result<Option<TabPresetEditDialogResult>, EntryError> {
    let hinstance = module_handle()?;
    register_program_edit_dialog_class(hinstance)?;

    let class_name = wide_null(PROGRAM_EDIT_DIALOG_CLASS_NAME);
    let title = wide_null(title);
    let (window_width, window_height) = program_edit_dialog_window_size();
    let (x, y) = centered_window_position(owner, window_width, window_height);
    let mut state = Box::new(ProgramEditDialogState::new(
        language,
        preset_name,
        programs,
        ok_label,
        cancel_label,
    ));
    let state_ptr = state.as_mut() as *mut ProgramEditDialogState;

    let hwnd = unsafe {
        CreateWindowExW(
            PROGRAM_EDIT_DIALOG_EX_STYLE,
            class_name.as_ptr(),
            title.as_ptr(),
            PROGRAM_EDIT_DIALOG_STYLE,
            x,
            y,
            window_width,
            window_height,
            owner,
            null_mut(),
            hinstance,
            state_ptr.cast(),
        )
    };

    if hwnd.is_null() {
        return Err(EntryError::win32(
            "CreateWindowExW",
            "탭 preset 편집 창을 열 수 없습니다.",
        ));
    }
    if unsafe { SetWindowTextW(hwnd, title.as_ptr()) } == 0 {
        cleanup_program_edit_dialog_create_failure(hwnd);
        return Err(EntryError::win32(
            "SetWindowTextW",
            "탭 preset 편집 창 제목을 설정할 수 없습니다.",
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
                "탭 preset 편집 메시지를 가져올 수 없습니다.",
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
            user_message: "입력한 탭 preset 정보를 읽을 수 없습니다.",
        });
    }

    Ok(state.result.take())
}

fn program_edit_dialog_window_size() -> (i32, i32) {
    program_edit_dialog_window_size_for_client(
        PROGRAM_EDIT_DIALOG_CLIENT_WIDTH,
        PROGRAM_EDIT_DIALOG_CLIENT_HEIGHT,
    )
}

fn program_edit_dialog_window_size_for_client(client_width: i32, client_height: i32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    let adjusted = unsafe {
        AdjustWindowRectEx(
            &mut rect,
            PROGRAM_EDIT_DIALOG_STYLE,
            0,
            PROGRAM_EDIT_DIALOG_EX_STYLE,
        )
    };
    if adjusted == 0 {
        return (client_width, client_height);
    }

    (
        rect.right.saturating_sub(rect.left),
        rect.bottom.saturating_sub(rect.top),
    )
}

fn register_program_edit_dialog_class(hinstance: HINSTANCE) -> Result<(), EntryError> {
    let class_name = wide_null(PROGRAM_EDIT_DIALOG_CLASS_NAME);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(program_edit_dialog_proc),
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
        Err(EntryError::Win32 {
            api: "RegisterClassW",
            last_error,
            user_message: "탭 preset 편집 창 class를 등록할 수 없습니다.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> ProgramEditDialogRowRange {
        ProgramEditDialogRowRange { start, end }
    }

    #[test]
    fn visible_row_range_includes_partially_visible_rows() {
        assert_eq!(
            program_edit_dialog_visible_row_range(10, 0, PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT),
            range(0, 4)
        );
        assert_eq!(
            program_edit_dialog_visible_row_range(
                10,
                PROGRAM_EDIT_DIALOG_ROW_TOP + PROGRAM_EDIT_DIALOG_ROW_HEIGHT - 1,
                PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT,
            ),
            range(0, 5)
        );
        assert_eq!(
            program_edit_dialog_visible_row_range(
                10,
                PROGRAM_EDIT_DIALOG_ROW_TOP + PROGRAM_EDIT_DIALOG_ROW_HEIGHT,
                PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT,
            ),
            range(1, 5)
        );
    }

    #[test]
    fn visible_row_range_clamps_to_available_rows() {
        assert_eq!(
            program_edit_dialog_visible_row_range(0, 0, PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT),
            range(0, 0)
        );
        assert_eq!(
            program_edit_dialog_visible_row_range(3, i32::MAX, PROGRAM_EDIT_DIALOG_VIEWPORT_HEIGHT),
            range(3, 3)
        );
    }

    #[test]
    fn row_update_ranges_do_not_scan_jump_gaps() {
        assert_eq!(
            program_edit_dialog_row_update_ranges(Some(range(0, 4)), range(9, 13)),
            [range(0, 4), range(4, 4), range(9, 13)]
        );
        assert_eq!(
            program_edit_dialog_row_update_ranges(Some(range(3, 7)), range(5, 9)),
            [range(3, 5), range(7, 7), range(5, 9)]
        );
        assert_eq!(
            program_edit_dialog_row_update_ranges(Some(range(5, 9)), range(3, 7)),
            [range(5, 5), range(7, 9), range(3, 7)]
        );
    }
}
