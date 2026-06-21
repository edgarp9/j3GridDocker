use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::process::{Child, Command};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::cairo;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use pangocairo::pango;

use crate::app::{
    App, AppError, PlacementRegistration, ShutdownReport, TabDeletionReport, TabSwitchReport,
    UndockReport, UndockStatus, WindowController, WindowOperation,
};
use crate::domain::{
    DEFAULT_MIN_REGION_SIZE, DomainError, ExternalProgramSpec, Rect, RegionId, SplitDirection,
    SplitterPath, TabId, TabPreset, TabPresetProgramPlacement, UiLanguage, WindowHandle,
    WorkspaceOptions,
};
use crate::infra::PreservedStartupSessionSettings;
use crate::infra::{
    DefaultWindowController, LinuxOverlayWindow, LinuxPointerState, SettingsFileError,
    SettingsFileStore,
};

#[path = "license_notices.rs"]
mod license_notices;
#[path = "shutdown.rs"]
mod shutdown;

use license_notices::{
    PROJECT_URL, about_notice_text, about_version_label_text, about_window_title_text,
};
use shutdown::{
    SettingsSavePolicy, ShutdownAttemptReport, ShutdownMode, ShutdownSettingsSaveError,
    ShutdownSettingsSaver, log_undock_failures, shutdown_report_after_settings_save,
    shutdown_report_is_complete,
};

const APPLICATION_ID: &str = "io.github.j3soon.j3griddocker";
const WINDOW_TITLE: &str = "j3GridDocker";
const ABOUT_DIALOG_DEFAULT_WIDTH: i32 = 540;
const ABOUT_DIALOG_DEFAULT_HEIGHT: i32 = 360;
const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 700;
const SPLITTER_HIT_TOLERANCE: i32 = 5;
const TAB_DRAG_THRESHOLD: f64 = 4.0;
const TOP_BAR_LEFT_MARGIN: i32 = 8;
const TOP_BAR_BUTTON_SPACING: i32 = 8;
const WORKSPACE_TOGGLE_BUTTON_WIDTH: i32 = 64;
const NEW_TAB_BUTTON_WIDTH: i32 = 48;
const COMMAND_BAR_LEFT_MARGIN: i32 = 8;
const TAB_BAR_HEIGHT: i32 = 34;
const COMMAND_BAR_HEIGHT: i32 = 36;
const STATUS_BAR_HEIGHT: i32 = 24;
const GTK_TAB_WIDTH: i32 = 132;
const GTK_TAB_GAP: i32 = 4;
const GTK_TAB_BUTTON_GAP: i32 = 2;
const GTK_TAB_CLOSE_BUTTON_WIDTH: i32 = 24;
const GTK_TAB_LABEL_WIDTH: i32 = GTK_TAB_WIDTH - GTK_TAB_CLOSE_BUTTON_WIDTH - GTK_TAB_BUTTON_GAP;
const TAB_OVERFLOW_BUTTON_WIDTH: i32 = 28;
const GTK_TAB_REORDER_SCROLL_ZONE: i32 = 28;
const COMMAND_BUTTON_WIDTHS: [i32; 4] = [112, 128, 112, 118];
const REGION_TITLE_HORIZONTAL_INSET: i32 = 8;
const REGION_TITLE_VERTICAL_INSET: i32 = 6;
const DROP_POLL_INTERVAL_MS: u64 = 125;
const DROP_WINDOW_MOVE_THRESHOLD: i32 = 4;
const SPLITTER_OVERLAY_POLL_INTERVAL_MS: u64 = 50;
const PRESET_RESTORE_POLL_INTERVAL_MS: u64 = 125;
const PRESET_RESTORE_TIMEOUT: Duration = Duration::from_secs(8);
const APP_CSS: &str = r#"
.j3-root {
    background-color: #f8f8f8;
    color: #202020;
}

.j3-top-bar,
.j3-menu-bar,
.j3-command-bar {
    background-color: #dfe7ec;
}

.j3-top-bar {
    padding-left: @TOP_BAR_LEFT_PADDING@;
}

.j3-command-bar {
    padding-left: @COMMAND_BAR_LEFT_PADDING@;
}

.j3-main-menu-button > button {
    background: transparent;
    border: none;
    min-height: 0;
    padding: 3px 10px;
}

.j3-main-menu-button > button:hover,
.j3-main-menu-button > button:focus,
.j3-main-menu-button > button:checked {
    background: #ffffff;
    border: 1px solid #808080;
    padding: 2px 9px;
}

.j3-main-menu-button arrow {
    min-width: 0;
    min-height: 0;
    margin: 0;
    padding: 0;
    -gtk-icon-size: 0;
    opacity: 0;
}

.j3-status {
    background-color: #f0f0f0;
    color: #202020;
    padding-left: 0;
}

button,
menubutton > button,
checkbutton {
    background: #ffffff;
    color: #202020;
    border: 1px solid #808080;
    border-radius: 0;
    box-shadow: none;
    text-shadow: none;
}

button:disabled,
menubutton > button:disabled,
checkbutton:disabled {
    background: #e8e8e8;
    color: #808080;
}

.j3-top-button,
.j3-command-button,
.j3-tab-active,
.j3-tab-inactive {
    min-width: 0;
    min-height: 0;
    padding: 0 4px;
}

.j3-tab-close {
    min-width: 0;
    min-height: 0;
    padding: 0;
}

.j3-overflow-button > button {
    min-width: 0;
    min-height: 0;
    padding: 0;
}

.j3-overflow-button arrow {
    min-width: 0;
    min-height: 0;
    margin: 0;
    padding: 0;
    -gtk-icon-size: 0;
    opacity: 0;
}

.j3-tab-active {
    background: #ffffff;
}

.j3-tab-inactive {
    background: #d8d8d8;
}

.j3-dialog,
.j3-dialog-content,
.j3-dialog-content box,
.j3-dialog-content grid,
.j3-dialog-content scrolledwindow,
.j3-dialog-content viewport {
    background-color: #f8f8f8;
    color: #202020;
}

.j3-dialog label {
    color: #202020;
}

popover,
popover contents {
    background: #ffffff;
    color: #202020;
    border: 1px solid #808080;
    border-radius: 0;
}

entry {
    background: #ffffff;
    color: #202020;
    border: 1px solid #808080;
    box-shadow: none;
}

label {
    color: #202020;
}
"#;

#[derive(Debug)]
pub enum EntryError {
    App(AppError),
    Settings(SettingsFileError),
    Gtk(String),
}

impl EntryError {
    pub fn user_message(&self) -> &str {
        match self {
            Self::App(error) => error.user_message(),
            Self::Settings(error) => error.user_message(),
            Self::Gtk(_) => "GTK4 UI를 시작할 수 없습니다.",
        }
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::App(error) => write!(formatter, "{error}"),
            Self::Settings(error) => write!(formatter, "{error}"),
            Self::Gtk(message) => write!(formatter, "{message}"),
        }
    }
}

impl Error for EntryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::App(error) => Some(error),
            Self::Settings(error) => Some(error),
            Self::Gtk(_) => None,
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

pub fn run() -> Result<(), EntryError> {
    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .build();
    let startup_error = Rc::new(RefCell::new(None));
    let startup_error_for_activate = Rc::clone(&startup_error);

    application.connect_activate(
        move |application| match LinuxMainWindow::build(application) {
            Ok(main_window) => {
                let window = main_window.borrow().widgets.window.clone();
                window.present();
            }
            Err(error) => {
                *startup_error_for_activate.borrow_mut() = Some(error);
                application.quit();
            }
        },
    );

    application.run();
    if let Some(error) = startup_error.borrow_mut().take() {
        return Err(error);
    }
    Ok(())
}

#[derive(Clone)]
struct LinuxWidgets {
    window: gtk::ApplicationWindow,
    root: gtk::Box,
    menu_bar: gtk::Box,
    top_bar: gtk::Box,
    workspace_toggle_button: gtk::Button,
    new_tab_button: gtk::Button,
    tab_strip: gtk::Box,
    tab_bar: gtk::Box,
    overflow_button: gtk::MenuButton,
    command_bar: gtk::Box,
    drawing_area: gtk::DrawingArea,
    status_label: gtk::Label,
}

struct LinuxMainWindow {
    app: App<DefaultWindowController>,
    settings_store: SettingsFileStore,
    settings_save_policy: SettingsSavePolicy,
    preserved_startup_session: Option<PreservedStartupSessionSettings>,
    widgets: LinuxWidgets,
    active_region: Option<RegionId>,
    dragging_splitter: Option<SplitterPath>,
    splitter_overlay: LinuxSplitterOverlayController,
    splitter_overlay_last_left_button_down: bool,
    drop_candidate: Option<LinuxDropCandidate>,
    preset_restore: Option<LinuxTabPresetRestoreState>,
    released_preset_children: Vec<Child>,
    tab_reorder_drag: Option<LinuxTabReorderDrag>,
    main_window_move_drag: Option<LinuxMainWindowMoveDrag>,
    suppressed_tab_click: Option<TabId>,
    tab_overflow_first_visible_index: usize,
    last_tab_visible_capacity: usize,
    last_active_tab_sync_bounds: Option<Rect>,
    main_window_maximized: bool,
    main_window_minimized: bool,
    workspace_options: WorkspaceOptions,
    workspace_ui_visible: bool,
    next_tab_number: u32,
    status: String,
    shutdown_done: bool,
}

impl LinuxMainWindow {
    fn build(application: &gtk::Application) -> Result<Rc<RefCell<Self>>, EntryError> {
        let (
            app,
            settings_store,
            settings_save_policy,
            preserved_startup_session,
            workspace_options,
            status,
        ) = load_app()?;
        let next_tab_number = next_tab_number(app.state().workspace().next_tab_id());
        let widgets = build_widgets(application);
        let main_window_maximized = widgets.window.is_maximized();
        let main_window = Rc::new(RefCell::new(Self {
            app,
            settings_store,
            settings_save_policy,
            preserved_startup_session,
            widgets,
            active_region: None,
            dragging_splitter: None,
            splitter_overlay: LinuxSplitterOverlayController::default(),
            splitter_overlay_last_left_button_down: false,
            drop_candidate: None,
            preset_restore: None,
            released_preset_children: Vec::new(),
            tab_reorder_drag: None,
            main_window_move_drag: None,
            suppressed_tab_click: None,
            tab_overflow_first_visible_index: 0,
            last_tab_visible_capacity: 0,
            last_active_tab_sync_bounds: None,
            main_window_maximized,
            main_window_minimized: false,
            workspace_options,
            workspace_ui_visible: true,
            next_tab_number,
            status,
            shutdown_done: false,
        }));

        install_callbacks(&main_window);
        install_drop_poll(&main_window);
        install_splitter_overlay_poll(&main_window);
        install_preset_restore_poll(&main_window);
        main_window.borrow_mut().refresh_all(&main_window);

        Ok(main_window)
    }

    fn language(&self) -> UiLanguage {
        self.workspace_options.ui_language()
    }

    fn refresh_all(&mut self, owner: &Rc<RefCell<Self>>) {
        self.refresh_menus(owner);
        self.refresh_tab_bar(owner);
        self.refresh_commands(owner);
        self.refresh_workspace_visibility();
        self.refresh_status();
        self.widgets.drawing_area.queue_draw();
    }

    fn refresh_menus(&self, owner: &Rc<RefCell<Self>>) {
        clear_box(&self.widgets.menu_bar);
        self.append_workspace_menu(owner);
        self.append_layout_menu(owner);
        self.append_presets_menu(owner);
        self.append_view_menu(owner);
        self.append_options_menu(owner);
        self.append_window_menu(owner);
        self.append_help_menu(owner);
    }

    fn append_workspace_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Workspace", "작업공간"));
        let content = menu_content();
        let has_active_tab = self.app.active_tab_id().is_some();
        append_menu_action(
            owner,
            &content,
            t(self.language(), "New Tab", "새 탭"),
            |state| state.borrow_mut().add_tab(&state),
        );
        append_menu_action_enabled(
            owner,
            &content,
            t(self.language(), "Rename Tab...", "탭 이름 변경..."),
            has_active_tab,
            |state| LinuxMainWindow::rename_active_tab(&state),
        );
        append_menu_action_enabled(
            owner,
            &content,
            t(self.language(), "Close Tab", "탭 닫기"),
            has_active_tab,
            |state| state.borrow_mut().close_active_tab(&state),
        );
        append_menu_action_enabled(
            owner,
            &content,
            t(self.language(), "Close Other Tabs", "다른 탭 닫기"),
            has_active_tab,
            |state| state.borrow_mut().close_other_tabs(&state),
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_layout_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Layout", "레이아웃"));
        let content = menu_content();
        append_menu_action(
            owner,
            &content,
            t(self.language(), "Split Region Vertically", "영역 세로 분할"),
            |state| {
                state
                    .borrow_mut()
                    .split_active_region(&state, SplitDirection::Vertical)
            },
        );
        append_menu_action(
            owner,
            &content,
            t(
                self.language(),
                "Split Region Horizontally",
                "영역 가로 분할",
            ),
            |state| {
                state
                    .borrow_mut()
                    .split_active_region(&state, SplitDirection::Horizontal)
            },
        );
        append_menu_action(
            owner,
            &content,
            t(self.language(), "Delete Selected Region", "선택 영역 삭제"),
            |state| state.borrow_mut().delete_active_region(&state),
        );
        append_menu_action(
            owner,
            &content,
            t(
                self.language(),
                "Undock Selected Window",
                "선택 창 배치 해제",
            ),
            |state| state.borrow_mut().undock_active_region(&state),
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_presets_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Presets", "프리셋"));
        let content = menu_content();
        let has_active_tab = self.app.active_tab_id().is_some();
        append_menu_action_enabled(
            owner,
            &content,
            t(self.language(), "Save Tab Preset...", "탭 preset 저장..."),
            has_active_tab,
            |state| LinuxMainWindow::save_active_tab_preset(&state),
        );
        append_preset_action_menu_button(
            owner,
            &content,
            PresetAction::Load.title(self.language()),
            PresetAction::Load,
            PresetTarget::Active,
            self.language(),
            &self.tab_preset_names(),
        );
        append_preset_action_menu_button(
            owner,
            &content,
            PresetAction::Edit.title(self.language()),
            PresetAction::Edit,
            PresetTarget::Active,
            self.language(),
            &self.tab_preset_names(),
        );
        append_preset_action_menu_button(
            owner,
            &content,
            PresetAction::Delete.title(self.language()),
            PresetAction::Delete,
            PresetTarget::Active,
            self.language(),
            &self.tab_preset_names(),
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_view_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "View", "보기"));
        let content = menu_content();
        append_menu_action(
            owner,
            &content,
            workspace_ui_toggle_menu_label(self.language(), self.workspace_ui_visible),
            |state| state.borrow_mut().toggle_workspace_ui(&state),
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_options_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Options", "옵션"));
        let content = menu_content();
        self.append_options_menu_items(owner, &content, false);
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_options_menu_items(
        &self,
        owner: &Rc<RefCell<Self>>,
        content: &gtk::Box,
        compact: bool,
    ) {
        append_check_menu_action(
            owner,
            content,
            t(
                self.language(),
                if compact {
                    "Dock while hidden"
                } else {
                    "Dock While Workspace Controls Are Hidden"
                },
                if compact {
                    "숨김 상태에서도 Dock"
                } else {
                    "작업 영역 컨트롤 숨김 중 Dock"
                },
            ),
            self.workspace_options.dock_hidden_workspace_ui(),
            |state| state.borrow_mut().toggle_dock_hidden_workspace_ui(&state),
        );
        append_separator(content);
        if compact {
            append_check_menu_action(
                owner,
                content,
                t(self.language(), "Language: English", "언어: 영어"),
                self.language() == UiLanguage::English,
                |state| {
                    state
                        .borrow_mut()
                        .set_ui_language(&state, UiLanguage::English)
                },
            );
            append_check_menu_action(
                owner,
                content,
                t(self.language(), "Language: Korean", "언어: 한국어"),
                self.language() == UiLanguage::Korean,
                |state| {
                    state
                        .borrow_mut()
                        .set_ui_language(&state, UiLanguage::Korean)
                },
            );
        } else {
            append_language_menu_button(owner, content, self.language());
        }
    }

    fn append_window_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Window", "창"));
        let content = menu_content();
        append_menu_action(
            owner,
            &content,
            t(self.language(), "Minimize", "최소화"),
            |state| minimize_main_window(&state),
        );
        append_menu_action(
            owner,
            &content,
            main_window_maximize_restore_label(self.language(), self.main_window_maximized),
            |state| toggle_main_window_maximized(&state),
        );
        append_menu_action(
            owner,
            &content,
            t(self.language(), "Close Window", "창 닫기"),
            |state| {
                let window = state.borrow().widgets.window.clone();
                window.close();
            },
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn append_help_menu(&self, owner: &Rc<RefCell<Self>>) {
        let menu = main_menu_button(t(self.language(), "Help", "도움말"));
        let content = menu_content();
        append_menu_action(
            owner,
            &content,
            t(self.language(), "About j3GridDocker", "j3GridDocker 정보"),
            |state| state.borrow().show_about_dialog(),
        );
        attach_popover(owner, &menu, content);
        self.widgets.menu_bar.append(&menu);
    }

    fn refresh_tab_bar(&mut self, owner: &Rc<RefCell<Self>>) {
        clear_box(&self.widgets.tab_bar);
        let active = self.app.active_tab_id();
        let (visible_start, visible_end) = self.sync_tab_overflow_range();
        let tabs = self.app.state().workspace().tabs();

        for (index, tab) in tabs
            .iter()
            .enumerate()
            .skip(visible_start)
            .take(visible_end.saturating_sub(visible_start))
        {
            if self.tab_reorder_insertion_before_index() == Some(index) {
                self.widgets.tab_bar.append(&tab_reorder_indicator());
            }
            let tab_id = tab.id();
            let tab_item = gtk::Box::new(gtk::Orientation::Horizontal, GTK_TAB_BUTTON_GAP);
            tab_item.add_css_class("linked");
            tab_item.set_width_request(GTK_TAB_WIDTH);
            tab_item.set_height_request(TAB_BAR_HEIGHT);
            tab_item.set_hexpand(false);
            tab_item.set_halign(gtk::Align::Start);

            let label = if active == Some(tab_id) {
                format!("{} *", tab.name())
            } else {
                tab.name().to_owned()
            };
            let button = gtk::Button::with_label(&label);
            button.set_width_request(GTK_TAB_LABEL_WIDTH);
            button.set_hexpand(false);
            if active == Some(tab_id) {
                button.add_css_class("j3-tab-active");
            } else {
                button.add_css_class("j3-tab-inactive");
            }
            let tab_tooltip = self.tab_tooltip_text_for_tab(tab_id);
            button.set_tooltip_text(tab_tooltip.as_deref());
            let owner_for_click = Rc::clone(owner);
            button.connect_clicked(move |_| {
                owner_for_click
                    .borrow_mut()
                    .switch_tab_from_button(&owner_for_click, tab_id);
            });

            let owner_for_drag = Rc::clone(owner);
            let drag = gtk::GestureDrag::new();
            let owner_for_drag_begin = Rc::clone(&owner_for_drag);
            drag.set_button(1);
            drag.connect_drag_begin(move |gesture, start_x, _start_y| {
                let Some(widget) = gesture.widget() else {
                    return;
                };
                let mut state = owner_for_drag_begin.borrow_mut();
                if state.begin_hidden_tab_window_move(tab_id) {
                    return;
                }
                state.begin_tab_reorder_drag(tab_id, index, &widget, start_x);
            });
            let owner_for_drag_update = Rc::clone(&owner_for_drag);
            drag.connect_drag_update(move |_, offset_x, offset_y| {
                let mut state = owner_for_drag_update.borrow_mut();
                if state.update_main_window_move_drag(offset_x, offset_y) {
                    return;
                }
                state.update_tab_reorder_drag(offset_x, &owner_for_drag_update);
            });
            drag.connect_drag_end(move |_, _, _| {
                let mut state = owner_for_drag.borrow_mut();
                if state.finish_main_window_move_drag() {
                    return;
                }
                state.finish_tab_reorder_drag(&owner_for_drag);
            });
            button.add_controller(drag);

            let owner_for_context = Rc::clone(owner);
            let context_click = gtk::GestureClick::new();
            context_click.set_button(3);
            context_click.connect_pressed(move |gesture, _, x, y| {
                if let Some(widget) = gesture.widget() {
                    show_tab_context_menu(&owner_for_context, tab_id, &widget, x, y);
                }
            });
            button.add_controller(context_click);

            let close = gtk::Button::with_label("X");
            close.add_css_class("j3-tab-close");
            close.set_width_request(GTK_TAB_CLOSE_BUTTON_WIDTH);
            close.set_hexpand(false);
            close.set_tooltip_text(tab_tooltip.as_deref().or(Some(t(
                self.language(),
                "Close tab",
                "탭 닫기",
            ))));
            let owner_for_close = Rc::clone(owner);
            close.connect_clicked(move |_| {
                owner_for_close
                    .borrow_mut()
                    .delete_tab(tab_id, &owner_for_close);
            });
            let owner_for_close_context = Rc::clone(owner);
            let close_context_click = gtk::GestureClick::new();
            close_context_click.set_button(3);
            close_context_click.connect_pressed(move |gesture, _, x, y| {
                if let Some(widget) = gesture.widget() {
                    show_tab_context_menu(&owner_for_close_context, tab_id, &widget, x, y);
                }
            });
            close.add_controller(close_context_click);

            tab_item.append(&button);
            tab_item.append(&close);
            self.widgets.tab_bar.append(&tab_item);
        }

        if self.tab_reorder_insertion_before_index() == Some(visible_end) {
            self.widgets.tab_bar.append(&tab_reorder_indicator());
        }

        self.refresh_overflow(owner, visible_start, visible_end);
    }

    fn tab_reorder_insertion_before_index(&self) -> Option<usize> {
        let before_tab_id = self.tab_reorder_drag.as_ref()?.insertion?.before_tab_id;
        match before_tab_id {
            Some(tab_id) => self
                .app
                .state()
                .workspace()
                .tabs()
                .iter()
                .position(|tab| tab.id() == tab_id),
            None => Some(self.app.state().workspace().tabs().len()),
        }
    }

    fn sync_tab_overflow_range(&mut self) -> (usize, usize) {
        let tabs = self.app.state().workspace().tabs();
        let tab_count = tabs.len();
        let visible_count = self.visible_tab_capacity(tab_count);
        self.last_tab_visible_capacity = visible_count;
        let active_index = self
            .app
            .active_tab_id()
            .and_then(|active_tab| tabs.iter().position(|tab| tab.id() == active_tab));
        let (first, end) = linux_tab_overflow_range(
            tab_count,
            visible_count,
            self.tab_overflow_first_visible_index,
            active_index,
            self.tab_reorder_drag.is_some(),
        );
        self.tab_overflow_first_visible_index = first;
        (first, end)
    }

    fn visible_tab_capacity(&self, tab_count: usize) -> usize {
        visible_tab_capacity_for_tab_strip_width(
            self.widgets.tab_strip.allocated_width(),
            tab_count,
        )
    }

    fn refresh_tab_bar_after_resize_if_needed(&mut self, owner: &Rc<RefCell<Self>>) {
        let tab_count = self.app.state().workspace().tabs().len();
        let capacity = self.visible_tab_capacity(tab_count);
        if capacity != self.last_tab_visible_capacity {
            self.refresh_all(owner);
        }
    }

    fn tab_tooltip_text_for_tab(&self, tab_id: TabId) -> Option<String> {
        let placements = self
            .app
            .state()
            .workspace()
            .placements_for_tab(tab_id)
            .ok()?;
        tab_tooltip_text(placements.iter().filter_map(|placement| {
            match self.app.controller().window_title(placement.hwnd()) {
                Ok(Some(title)) => Some(title),
                Ok(None) if self.app.controller().current_rect(placement.hwnd()).is_ok() => {
                    Some(t(self.language(), "(untitled window)", "(제목 없는 창)").to_owned())
                }
                _ => None,
            }
        }))
    }

    fn refresh_overflow(
        &self,
        owner: &Rc<RefCell<Self>>,
        visible_start: usize,
        visible_end: usize,
    ) {
        let content = menu_content();
        let mut hidden_count = 0usize;
        for (index, tab) in self.app.state().workspace().tabs().iter().enumerate() {
            if index >= visible_start && index < visible_end {
                continue;
            }
            hidden_count += 1;
            let tab_id = tab.id();
            let label = tab_overflow_menu_label(index, tab.name());
            append_menu_action(owner, &content, &label, move |state| {
                state.borrow_mut().switch_tab(&state, tab_id)
            });
        }
        self.widgets.overflow_button.set_visible(hidden_count > 0);
        attach_popover(owner, &self.widgets.overflow_button, content);
    }

    fn refresh_commands(&self, owner: &Rc<RefCell<Self>>) {
        clear_box(&self.widgets.command_bar);
        append_command_button(
            owner,
            &self.widgets.command_bar,
            t(self.language(), "Split vertical", "세로 분할"),
            COMMAND_BUTTON_WIDTHS[0],
            |state| {
                state
                    .borrow_mut()
                    .split_active_region(&state, SplitDirection::Vertical)
            },
        );
        append_command_button(
            owner,
            &self.widgets.command_bar,
            t(self.language(), "Split horizontal", "가로 분할"),
            COMMAND_BUTTON_WIDTHS[1],
            |state| {
                state
                    .borrow_mut()
                    .split_active_region(&state, SplitDirection::Horizontal)
            },
        );
        append_command_button(
            owner,
            &self.widgets.command_bar,
            t(self.language(), "Delete region", "영역 삭제"),
            COMMAND_BUTTON_WIDTHS[2],
            |state| state.borrow_mut().delete_active_region(&state),
        );
        append_command_button(
            owner,
            &self.widgets.command_bar,
            t(self.language(), "Undock window", "창 해제"),
            COMMAND_BUTTON_WIDTHS[3],
            |state| state.borrow_mut().undock_active_region(&state),
        );
    }

    fn refresh_workspace_visibility(&self) {
        self.widgets.menu_bar.set_visible(self.workspace_ui_visible);
        self.widgets
            .command_bar
            .set_visible(self.workspace_ui_visible);
        self.widgets
            .drawing_area
            .set_visible(self.workspace_ui_visible);
        self.widgets
            .status_label
            .set_visible(self.workspace_ui_visible);
        self.widgets.window.set_decorated(self.workspace_ui_visible);
        self.widgets
            .workspace_toggle_button
            .set_label(workspace_ui_toggle_button_label(
                self.language(),
                self.workspace_ui_visible,
            ));
        self.widgets
            .new_tab_button
            .set_label(new_tab_button_label(self.language()));
    }

    fn refresh_status(&self) {
        self.widgets.status_label.set_text(&self.status);
    }

    fn detach_attached_popovers(&self) {
        detach_menu_button_popovers(self.widgets.window.upcast_ref());
    }

    fn on_main_window_minimized(&mut self) {
        if self.main_window_minimized {
            return;
        }

        self.main_window_minimized = true;
        self.last_active_tab_sync_bounds = None;
        self.dragging_splitter = None;
        self.hide_splitter_overlay();
        self.drop_candidate = None;
        self.tab_reorder_drag = None;
        self.main_window_move_drag = None;

        if let Err(error) = self.app.hide_active_tab() {
            self.report_app_error(error);
        }
    }

    fn on_main_window_restored(&mut self) {
        if !self.main_window_minimized {
            return;
        }

        self.main_window_minimized = false;
        self.last_active_tab_sync_bounds = None;
        self.show_active_tab_at_current_bounds();
    }

    fn set_main_window_maximized(&mut self, owner: &Rc<RefCell<Self>>, maximized: bool) {
        if self.main_window_maximized == maximized {
            return;
        }

        self.main_window_maximized = maximized;
        self.refresh_menus(owner);
    }

    fn sync_main_window_toplevel_state(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        minimized: bool,
        maximized: bool,
    ) {
        if minimized {
            self.on_main_window_minimized();
        } else {
            self.on_main_window_restored();
        }
        self.set_main_window_maximized(owner, maximized);
    }

    fn tab_preset_names(&self) -> Vec<String> {
        self.app
            .list_tab_presets()
            .iter()
            .map(|preset| preset.name().to_owned())
            .collect()
    }

    fn tab_switch_status_context(&self, target: TabId) -> LinuxTabSwitchStatusContext {
        LinuxTabSwitchStatusContext {
            target: self.tab_status_label(target),
            previous_active: self
                .app
                .active_tab_id()
                .map(|tab_id| self.tab_status_label(tab_id)),
        }
    }

    fn tab_deletion_status_context(&self, deleted_tab: TabId) -> LinuxTabDeletionStatusContext {
        LinuxTabDeletionStatusContext {
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
    ) -> Option<LinuxTabStatusLabel> {
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

        Some(LinuxTabStatusLabel {
            tab_id: target.id(),
            name: Some(target.name().to_owned()),
        })
    }

    fn tab_status_label(&self, tab_id: TabId) -> LinuxTabStatusLabel {
        let name = self
            .app
            .state()
            .workspace()
            .tab(tab_id)
            .ok()
            .map(|tab| tab.name().to_owned());
        LinuxTabStatusLabel { tab_id, name }
    }

    fn add_tab(&mut self, owner: &Rc<RefCell<Self>>) {
        if self.layout_bounds_screen().is_none() {
            return;
        }
        let name = format!("Tab {}", self.next_tab_number);
        self.next_tab_number = self.next_tab_number.saturating_add(1);
        match self.app.add_tab(name) {
            Ok(tab_id) => {
                self.record_workspace_change();
                self.switch_tab(owner, tab_id);
                return;
            }
            Err(error) => self.report_app_error(error),
        }
        self.refresh_all(owner);
    }

    fn switch_tab(&mut self, owner: &Rc<RefCell<Self>>, tab_id: TabId) {
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let context = self.tab_switch_status_context(tab_id);
        match self.app.switch_tab(tab_id, bounds) {
            Ok(report) => {
                if report.removed_stale_placements() > 0 {
                    self.record_workspace_change();
                }
                self.active_region = None;
                self.status = tab_switch_success_status_text(self.language(), &context, report);
            }
            Err(error) => self.report_switch_tab_error(&context, error),
        }
        self.refresh_all(owner);
    }

    fn delete_tab(&mut self, tab_id: TabId, owner: &Rc<RefCell<Self>>) {
        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_localized_status(
                "Tab delete failed: workspace bounds could not be calculated. Undock: not attempted",
                "탭 삭제 실패: 작업 영역 좌표를 계산할 수 없습니다. Undock: 시도하지 않음",
            );
            return;
        };
        let context = self.tab_deletion_status_context(tab_id);
        match self.app.delete_tab(tab_id, bounds) {
            Ok(report) => {
                self.record_workspace_change();
                if report.current_active_tab().is_none() {
                    self.active_region = None;
                }
                if self.app.state().workspace().tabs().is_empty() {
                    self.next_tab_number =
                        next_tab_number(self.app.state().workspace().next_tab_id());
                }
                self.status = tab_deletion_status_text(self.language(), &report);
            }
            Err(error) => self.report_tab_deletion_error(&context, error),
        }
        self.refresh_all(owner);
    }

    fn close_active_tab(&mut self, owner: &Rc<RefCell<Self>>) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        self.delete_tab(tab_id, owner);
    }

    fn close_other_tabs(&mut self, owner: &Rc<RefCell<Self>>) {
        let Some(active_tab) = self.app.active_tab_id() else {
            self.set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        self.close_other_tabs_for(owner, active_tab);
    }

    fn close_other_tabs_for(&mut self, owner: &Rc<RefCell<Self>>, target_tab: TabId) {
        if !self
            .app
            .state()
            .workspace()
            .tabs()
            .iter()
            .any(|tab| tab.id() == target_tab)
        {
            self.status = close_other_target_missing_status_text(self.language(), target_tab);
            self.refresh_all(owner);
            return;
        }
        let targets = self
            .app
            .state()
            .workspace()
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .filter(|tab_id| *tab_id != target_tab)
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.set_localized_status(
                "There are no other tabs to close.",
                "닫을 다른 탭이 없습니다.",
            );
            return;
        }
        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_status(close_other_bounds_failure_status_text(self.language()));
            return;
        };

        let mut closed = 0usize;
        let mut undock = LinuxUndockCounts::default();
        let mut failures = Vec::new();
        for tab_id in targets.iter().copied() {
            match self.app.delete_tab(tab_id, bounds) {
                Ok(report) => {
                    closed += 1;
                    self.record_workspace_change();
                    undock.add_report(report.undock());
                }
                Err(error) => {
                    eprintln!("{error}");
                    if let Some(source) = error.source() {
                        eprintln!("cause: {source}");
                    }
                    failures.push(LinuxTabOperationFailure {
                        tab_id,
                        operation: "delete",
                        message: linux_app_error_message(self.language(), &error),
                    });
                }
            }
        }
        self.status = close_other_tabs_status_text(
            self.language(),
            target_tab,
            targets.len(),
            closed,
            self.app.active_tab_id(),
            undock,
            &failures,
        );
        self.refresh_all(owner);
    }

    fn switch_tab_from_button(&mut self, owner: &Rc<RefCell<Self>>, tab_id: TabId) {
        if self.suppressed_tab_click.take() == Some(tab_id) {
            return;
        }
        self.switch_tab(owner, tab_id);
    }

    fn begin_tab_reorder_drag(
        &mut self,
        tab_id: TabId,
        index: usize,
        widget: &gtk::Widget,
        start_x: f64,
    ) {
        if !self.workspace_ui_visible {
            return;
        }
        if self
            .app
            .state()
            .workspace()
            .tabs()
            .get(index)
            .is_none_or(|tab| tab.id() != tab_id)
        {
            return;
        }

        let tab_origin_x = widget
            .parent()
            .map(|parent| parent.allocation().x())
            .unwrap_or(0)
            .saturating_add(widget.allocation().x());
        self.tab_reorder_drag = Some(LinuxTabReorderDrag {
            tab_id,
            start_pointer_x: f64::from(tab_origin_x) + start_x,
            insertion: None,
        });
    }

    fn begin_hidden_tab_window_move(&mut self, tab_id: TabId) -> bool {
        if self.workspace_ui_visible {
            return false;
        }
        if !self.begin_main_window_move_drag() {
            return false;
        }

        self.suppressed_tab_click = Some(tab_id);
        true
    }

    fn begin_hidden_tabbar_window_move(
        &mut self,
        tab_bar: &gtk::Box,
        overflow_button: &gtk::MenuButton,
        x: f64,
        y: f64,
    ) -> bool {
        if tab_strip_position_hits_non_empty_child(tab_bar, overflow_button, x, y) {
            return false;
        }
        self.begin_main_window_move_drag()
    }

    fn begin_main_window_move_drag(&mut self) -> bool {
        if self.workspace_ui_visible || self.main_window_minimized {
            return false;
        };
        let Some(xid) = self.main_window_xid() else {
            return false;
        };
        let initial_rect = match self.app.controller().current_rect_for_xid(xid) {
            Ok(rect) => rect,
            Err(error) => {
                self.report_app_error(AppError::from(error));
                return false;
            }
        };

        self.main_window_move_drag = Some(LinuxMainWindowMoveDrag { xid, initial_rect });
        true
    }

    fn update_main_window_move_drag(&mut self, offset_x: f64, offset_y: f64) -> bool {
        let Some(drag) = self.main_window_move_drag else {
            return false;
        };
        if offset_x.abs() < TAB_DRAG_THRESHOLD && offset_y.abs() < TAB_DRAG_THRESHOLD {
            return true;
        }
        let left = drag
            .initial_rect
            .left()
            .saturating_add(drag_offset_to_i32(offset_x));
        let top = drag
            .initial_rect
            .top()
            .saturating_add(drag_offset_to_i32(offset_y));
        let Ok(rect) = Rect::new(
            left,
            top,
            drag.initial_rect.width(),
            drag.initial_rect.height(),
        ) else {
            return true;
        };

        if let Err(error) = self.app.controller_mut().set_rect_for_xid(drag.xid, rect) {
            self.main_window_move_drag = None;
            self.report_app_error(AppError::from(error));
        }
        true
    }

    fn finish_main_window_move_drag(&mut self) -> bool {
        self.main_window_move_drag.take().is_some()
    }

    fn main_window_xid(&self) -> Option<u64> {
        self.widgets
            .window
            .surface()
            .and_then(|surface| surface.downcast::<gdk4_x11::X11Surface>().ok())
            .map(|surface| surface.xid())
    }

    fn main_window_current_rect(&self) -> Option<Rect> {
        self.app
            .controller()
            .current_rect_for_xid(self.main_window_xid()?)
            .ok()
    }

    fn main_window_contains(&self, root_x: i32, root_y: i32) -> bool {
        self.main_window_current_rect()
            .is_some_and(|rect| rect.contains_point(root_x, root_y))
    }

    fn external_window_control_available(&self) -> bool {
        if !self.app.controller().x11_available() {
            return false;
        }
        self.main_window_xid().is_some()
    }

    fn ensure_external_window_control_available(&mut self) -> bool {
        if self.external_window_control_available() {
            return true;
        }
        self.set_status(t(
            self.language(),
            "Linux window docking requires an X11 GTK session.",
            "Linux 창 도킹은 X11 GTK 세션에서만 사용할 수 있습니다.",
        ));
        false
    }

    fn update_tab_reorder_drag(&mut self, offset_x: f64, owner: &Rc<RefCell<Self>>) {
        if offset_x.abs() < TAB_DRAG_THRESHOLD {
            return;
        }
        let Some(drag) = self.tab_reorder_drag.as_ref() else {
            return;
        };
        let pointer_x = drag.start_pointer_x + offset_x;
        if self.scroll_tab_reorder_view(pointer_x) {
            self.refresh_all(owner);
        }

        let Some(insertion) = self.tab_reorder_insertion_for_x(pointer_x) else {
            return;
        };

        let changed = self
            .tab_reorder_drag
            .as_ref()
            .is_some_and(|drag| drag.insertion != Some(insertion));
        if let Some(drag) = self.tab_reorder_drag.as_mut() {
            drag.insertion = Some(insertion);
        }
        if changed {
            self.refresh_all(owner);
        }
    }

    fn finish_tab_reorder_drag(&mut self, owner: &Rc<RefCell<Self>>) {
        let Some(drag) = self.tab_reorder_drag.take() else {
            return;
        };
        let Some(insertion) = drag.insertion else {
            self.refresh_all(owner);
            return;
        };
        match self
            .app
            .reorder_tab_before(drag.tab_id, insertion.before_tab_id)
        {
            Ok(true) => {
                self.record_workspace_change();
                self.suppress_next_tab_click(owner, drag.tab_id);
                self.status = tab_reorder_status_text(
                    self.language(),
                    drag.tab_id,
                    insertion.before_tab_id,
                    true,
                );
            }
            Ok(false) => {
                self.suppress_next_tab_click(owner, drag.tab_id);
                self.status = tab_reorder_status_text(
                    self.language(),
                    drag.tab_id,
                    insertion.before_tab_id,
                    false,
                );
            }
            Err(error) => self.report_tab_operation_error(drag.tab_id, "reorder", error),
        }
        self.refresh_all(owner);
    }

    fn suppress_next_tab_click(&mut self, owner: &Rc<RefCell<Self>>, tab_id: TabId) {
        self.suppressed_tab_click = Some(tab_id);
        let owner_for_clear = Rc::clone(owner);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let mut state = owner_for_clear.borrow_mut();
            if state.suppressed_tab_click == Some(tab_id) {
                state.suppressed_tab_click = None;
            }
        });
    }

    fn tab_reorder_insertion_for_x(&self, pointer_x: f64) -> Option<LinuxTabReorderInsertion> {
        let tabs = self.app.state().workspace().tabs();
        let tab_count = tabs.len();
        let visible_count = self.visible_tab_capacity(tab_count);
        let insertion_index = linux_tab_reorder_insertion_index_for_x(
            pointer_x,
            tab_count,
            visible_count,
            self.tab_overflow_first_visible_index,
        )?;
        Some(LinuxTabReorderInsertion {
            before_tab_id: tabs.get(insertion_index).map(|tab| tab.id()),
        })
    }

    fn scroll_tab_reorder_view(&mut self, pointer_x: f64) -> bool {
        let tab_count = self.app.state().workspace().tabs().len();
        let visible_count = self.visible_tab_capacity(tab_count);
        if let Some(first) = linux_tab_reorder_scroll_first_index(
            pointer_x,
            self.widgets.tab_bar.allocated_width(),
            tab_count,
            visible_count,
            self.tab_overflow_first_visible_index,
        ) {
            self.tab_overflow_first_visible_index = first;
            return true;
        }

        false
    }

    fn rename_active_tab(owner: &Rc<RefCell<Self>>) {
        let active_tab = owner.borrow().app.active_tab_id();
        let Some(tab_id) = active_tab else {
            owner
                .borrow_mut()
                .set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        Self::rename_tab(owner, tab_id);
    }

    fn rename_tab(owner: &Rc<RefCell<Self>>, tab_id: TabId) {
        let tab_result = { owner.borrow().app.state().workspace().tab(tab_id).cloned() };
        let current_name = match tab_result {
            Ok(tab) => tab.name().to_owned(),
            Err(error) => {
                owner.borrow_mut().report_app_error(AppError::from(error));
                return;
            }
        };
        let language = owner.borrow().language();
        let title = t(language, "Rename tab", "탭 이름 변경").to_owned();
        let prompt = tab_rename_prompt_label_text(language);
        prompt_text(
            owner,
            &title,
            prompt,
            &current_name,
            move |state, value| {
                let rename_result = { state.borrow_mut().app.rename_tab(tab_id, value) };
                match rename_result {
                    Ok(()) => {
                        let mut state = state.borrow_mut();
                        state.record_workspace_change();
                        state.status = tab_rename_success_status_text(state.language(), tab_id);
                    }
                    Err(error) => state
                        .borrow_mut()
                        .report_tab_operation_error(tab_id, "rename", error),
                }
                state.borrow_mut().refresh_all(&state);
            },
            move |owner| {
                let mut state = owner.borrow_mut();
                state.status = tab_rename_cancel_status_text(state.language(), tab_id);
                state.refresh_all(&owner);
            },
        );
    }

    fn split_active_region(&mut self, owner: &Rc<RefCell<Self>>, direction: SplitDirection) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_localized_status(
                "Select a region to split first.",
                "분할할 영역을 먼저 선택하세요.",
            );
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        match self.app.split_region(tab_id, region_id, direction, bounds) {
            Ok(_) => {
                self.record_workspace_change();
                self.status = t(
                    self.language(),
                    "Region split complete.",
                    "영역을 분할했습니다.",
                )
                .to_owned();
            }
            Err(error) => self.report_app_error(error),
        }
        self.refresh_all(owner);
    }

    fn delete_active_region(&mut self, owner: &Rc<RefCell<Self>>) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_localized_status(
                "Select a region to delete first.",
                "삭제할 영역을 먼저 선택하세요.",
            );
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        match self.app.delete_region(tab_id, region_id, bounds) {
            Ok(_) => {
                self.record_workspace_change();
                self.active_region = None;
                self.status =
                    t(self.language(), "Region deleted.", "영역을 삭제했습니다.").to_owned();
            }
            Err(error) => self.report_app_error(error),
        }
        self.refresh_all(owner);
    }

    fn undock_active_region(&mut self, owner: &Rc<RefCell<Self>>) {
        let Some(tab_id) = self.app.active_tab_id() else {
            self.set_localized_status("There is no active tab.", "활성 탭이 없습니다.");
            return;
        };
        let Some(region_id) = self.active_region else {
            self.set_localized_status(
                "Select a region to undock first.",
                "해제할 영역을 먼저 선택하세요.",
            );
            return;
        };
        match self.app.unregister_placement(tab_id, region_id) {
            Ok(_) => {
                self.record_workspace_change();
                self.status = t(
                    self.language(),
                    "External window placement was undocked.",
                    "외부 윈도우 배치를 해제했습니다.",
                )
                .to_owned();
            }
            Err(error) => self.report_app_error(error),
        }
        self.refresh_all(owner);
    }

    fn save_active_tab_preset(owner: &Rc<RefCell<Self>>) {
        let active_tab = owner.borrow().app.active_tab_id();
        let Some(tab_id) = active_tab else {
            let mut state = owner.borrow_mut();
            state.status = tab_preset_save_missing_tab_status_text(state.language()).to_owned();
            state.refresh_status();
            return;
        };
        Self::save_tab_preset_for_tab(owner, tab_id);
    }

    fn save_tab_preset_for_tab(owner: &Rc<RefCell<Self>>, tab_id: TabId) {
        let preset_count = owner.borrow().app.list_tab_presets().len();
        let initial_name = next_tab_preset_name(preset_count);
        let programs_result = { owner.borrow_mut().program_specs_for_tab(tab_id) };
        let programs = match programs_result {
            Ok(programs) => programs,
            Err(error) => {
                owner.borrow_mut().report_app_error(error);
                owner.borrow_mut().refresh_all(owner);
                return;
            }
        };
        let preset_result = {
            owner
                .borrow()
                .app
                .tab_preset_for_tab(tab_id, initial_name, programs)
        };
        let preset = match preset_result {
            Ok(preset) => preset,
            Err(error) => {
                owner.borrow_mut().report_app_error(error);
                owner.borrow_mut().refresh_all(owner);
                return;
            }
        };
        prompt_preset_edit(
            owner,
            preset,
            move |state, _original_name, edited| {
                let program_count = edited.program_specs().len();
                let save_result = { state.borrow_mut().app.save_tab_preset_value(edited) };
                match save_result {
                    Ok(preset) => {
                        let mut state = state.borrow_mut();
                        state.record_workspace_change();
                        state.status = tab_preset_save_success_status_text(
                            state.language(),
                            preset.name(),
                            program_count,
                        );
                    }
                    Err(error) => state.borrow_mut().report_app_error(error),
                }
                state.borrow_mut().refresh_all(&state);
            },
            move |owner| {
                let mut state = owner.borrow_mut();
                state.status = tab_preset_save_cancel_status_text(state.language()).to_owned();
                state.refresh_all(&owner);
            },
        );
    }

    fn run_tab_preset_action(
        owner: &Rc<RefCell<Self>>,
        action: PresetAction,
        target: PresetTarget,
        name: String,
    ) {
        match action {
            PresetAction::Load => {
                if let Some(target_tab) = target.tab_id(owner) {
                    owner
                        .borrow_mut()
                        .apply_tab_preset_to_tab(owner, &name, target_tab);
                } else {
                    let mut owner = owner.borrow_mut();
                    owner.status =
                        tab_preset_load_missing_active_tab_status_text(owner.language()).to_owned();
                    owner.refresh_status();
                }
            }
            PresetAction::Edit => Self::edit_tab_preset(owner, &name),
            PresetAction::Delete => {
                let delete_result = { owner.borrow_mut().app.delete_tab_preset(&name) };
                match delete_result {
                    Ok(preset) => {
                        let mut owner = owner.borrow_mut();
                        owner.record_workspace_change();
                        owner.status =
                            tab_preset_delete_success_status_text(owner.language(), preset.name());
                    }
                    Err(error) => {
                        let mut owner = owner.borrow_mut();
                        log_app_error(&error);
                        owner.status =
                            tab_preset_delete_failure_status_text(owner.language(), &name, &error);
                    }
                }
                owner.borrow_mut().refresh_all(owner);
            }
        }
    }

    fn edit_tab_preset(owner: &Rc<RefCell<Self>>, preset_name: &str) {
        let preset = owner
            .borrow()
            .app
            .list_tab_presets()
            .iter()
            .find(|preset| preset.name() == preset_name)
            .cloned();
        let Some(preset) = preset else {
            let error = AppError::Domain(DomainError::TabPresetNotFound(preset_name.to_owned()));
            let mut state = owner.borrow_mut();
            log_app_error(&error);
            state.status =
                tab_preset_edit_failure_status_text(state.language(), preset_name, &error);
            state.refresh_status();
            return;
        };
        prompt_preset_edit(
            owner,
            preset,
            move |state, original_name, edited| {
                let program_count = edited.program_specs().len();
                let edit_result = {
                    state
                        .borrow_mut()
                        .app
                        .replace_tab_preset(&original_name, edited)
                };
                match edit_result {
                    Ok(preset) => {
                        let mut state = state.borrow_mut();
                        state.record_workspace_change();
                        state.status = tab_preset_edit_success_status_text(
                            state.language(),
                            preset.name(),
                            program_count,
                        );
                    }
                    Err(error) => {
                        let mut state = state.borrow_mut();
                        log_app_error(&error);
                        state.status = tab_preset_edit_failure_status_text(
                            state.language(),
                            &original_name,
                            &error,
                        );
                    }
                }
                state.borrow_mut().refresh_all(&state);
            },
            move |owner| {
                let mut state = owner.borrow_mut();
                state.status = tab_preset_edit_cancel_status_text(state.language()).to_owned();
                state.refresh_all(&owner);
            },
        );
    }

    fn apply_tab_preset_to_tab(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        preset_name: &str,
        target_tab: TabId,
    ) {
        if !self.ensure_external_window_control_available() {
            return;
        }
        let target_label = self.tab_status_label(target_tab);
        let Some(bounds) = self.layout_bounds_screen() else {
            self.set_localized_status(
                "Tab preset load failed: workspace bounds could not be calculated.",
                "탭 preset 적용 실패: 작업 영역 좌표를 계산할 수 없습니다.",
            );
            return;
        };
        match self
            .app
            .apply_tab_preset_to_tab_replacing_existing_placements(preset_name, target_tab, bounds)
        {
            Ok((report, undocked)) => {
                self.record_workspace_change();
                let target_label = self.tab_status_label(report.target_tab_id());
                self.start_tab_preset_restore(
                    report.preset_name().to_owned(),
                    target_label,
                    report.target_tab_id(),
                    undocked,
                    report.program_placements(),
                );
            }
            Err(error) => {
                log_app_error(&error);
                self.status = tab_preset_apply_failure_status_text(
                    self.language(),
                    preset_name,
                    &target_label,
                    &error,
                );
                self.refresh_status();
            }
        }
        self.refresh_all(owner);
    }

    fn start_tab_preset_restore(
        &mut self,
        preset_name: String,
        target: LinuxTabStatusLabel,
        target_tab_id: TabId,
        undocked: usize,
        programs: &[TabPresetProgramPlacement],
    ) {
        self.cancel_tab_preset_restore();
        let mut restore = LinuxTabPresetRestoreState::new(
            preset_name,
            target,
            target_tab_id,
            undocked,
            programs.len(),
            self.language(),
            Instant::now(),
        );

        for program in programs {
            match PendingLinuxTabPresetProgram::start(program) {
                Ok(pending) => restore.push_pending(pending),
                Err(error) => restore.record_failure(error.into_failure(self.language())),
            }
        }

        if restore.has_pending() {
            self.status = restore.status_text();
            self.preset_restore = Some(restore);
        } else {
            self.status = restore.finished_status_text();
        }
    }

    fn cancel_tab_preset_restore(&mut self) {
        self.preset_restore = None;
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
            let title = self
                .app
                .controller()
                .window_title(placement.hwnd())
                .ok()
                .flatten();
            let program = self
                .app
                .controller_mut()
                .program_spec_for_snapshot(placement.snapshot(), title)?;
            programs.insert(placement.region_id(), program);
        }
        Ok(programs)
    }

    fn toggle_workspace_ui(&mut self, owner: &Rc<RefCell<Self>>) {
        self.workspace_ui_visible = !self.workspace_ui_visible;
        self.last_active_tab_sync_bounds = None;
        self.main_window_move_drag = None;
        self.hide_splitter_overlay();
        self.status = if self.workspace_ui_visible {
            t(
                self.language(),
                "Workspace UI is now visible.",
                "작업 영역 UI를 표시했습니다.",
            )
        } else {
            t(
                self.language(),
                "Workspace UI is now hidden.",
                "작업 영역 UI를 숨겼습니다.",
            )
        }
        .to_owned();
        if !self.workspace_ui_visible {
            self.tab_reorder_drag = None;
            self.dragging_splitter = None;
            self.drop_candidate = None;
        }
        self.refresh_all(owner);
        schedule_active_tab_sync(owner);
    }

    fn toggle_dock_hidden_workspace_ui(&mut self, owner: &Rc<RefCell<Self>>) {
        let enabled = !self.workspace_options.dock_hidden_workspace_ui();
        self.workspace_options = self
            .workspace_options
            .with_dock_hidden_workspace_ui(enabled);
        self.record_workspace_options_change();
        self.status = if enabled {
            t(
                self.language(),
                "Docking while the workspace UI is hidden is enabled.",
                "숨김 상태에서도 Dock을 허용합니다.",
            )
            .to_owned()
        } else {
            t(
                self.language(),
                "Docking while the workspace UI is hidden is disabled.",
                "숨김 상태 Dock을 비활성화했습니다.",
            )
            .to_owned()
        };
        self.refresh_all(owner);
    }

    fn set_ui_language(&mut self, owner: &Rc<RefCell<Self>>, language: UiLanguage) {
        if self.language() == language {
            self.status = match language {
                UiLanguage::English => "UI language is already English.",
                UiLanguage::Korean => "UI 언어가 이미 한국어입니다.",
            }
            .to_owned();
            self.refresh_all(owner);
            return;
        }
        self.workspace_options = self.workspace_options.with_ui_language(language);
        self.record_workspace_options_change();
        self.status = match language {
            UiLanguage::English => "UI language changed to English.",
            UiLanguage::Korean => "UI 언어를 한국어로 변경했습니다.",
        }
        .to_owned();
        self.refresh_all(owner);
    }

    fn show_about_dialog(&self) {
        let language = self.language();
        let title = about_dialog_title_text(language);
        let dialog = gtk::Dialog::builder()
            .transient_for(&self.widgets.window)
            .modal(true)
            .title(&title)
            .default_width(ABOUT_DIALOG_DEFAULT_WIDTH)
            .default_height(ABOUT_DIALOG_DEFAULT_HEIGHT)
            .build();
        apply_dialog_style(&dialog);
        dialog.add_button(t(language, "OK", "확인"), gtk::ResponseType::Ok);
        dialog.set_default_response(gtk::ResponseType::Ok);

        let version_label = gtk::Label::new(Some(&about_version_label_text()));
        version_label.set_xalign(0.0);
        dialog.content_area().append(&version_label);

        let text_view = gtk::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_wrap_mode(gtk::WrapMode::WordChar);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.buffer().set_text(&about_notice_text(language));

        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .min_content_width(500)
            .min_content_height(220)
            .child(&text_view)
            .build();
        dialog.content_area().append(&scrolled);

        let link = gtk::LinkButton::with_label(PROJECT_URL, PROJECT_URL);
        link.set_halign(gtk::Align::Start);
        let parent_for_link = dialog.clone();
        link.connect_activate_link(move |_| {
            open_about_url_from_dialog(&parent_for_link, language);
            glib::Propagation::Stop
        });
        dialog.content_area().append(&link);
        let handled = Rc::new(Cell::new(false));
        let handled_for_response = Rc::clone(&handled);
        dialog.connect_response(move |dialog, _| {
            if handled_for_response.replace(true) {
                return;
            }
            dialog.close();
        });
        dialog.present();
    }

    fn on_region_press(&mut self, x: f64, y: f64) {
        let Some(tab_id) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let x = bounds.left().saturating_add(x.round() as i32);
        let y = bounds.top().saturating_add(y.round() as i32);

        if self.begin_splitter_drag_at_root(tab_id, bounds, x, y) {
            return;
        }

        match self.app.hit_test_region(tab_id, bounds, x, y) {
            Ok(region) => {
                self.active_region = region;
                self.widgets.drawing_area.queue_draw();
            }
            Err(error) => self.report_app_error(error),
        }
    }

    fn on_region_context(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        widget: &gtk::Widget,
        x: f64,
        y: f64,
    ) {
        let Some(tab_id) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let root_x = bounds.left().saturating_add(x.round() as i32);
        let root_y = bounds.top().saturating_add(y.round() as i32);
        match self.app.hit_test_region(tab_id, bounds, root_x, root_y) {
            Ok(Some(region_id)) => {
                self.active_region = Some(region_id);
                self.widgets.drawing_area.queue_draw();
                show_region_context_menu(owner, widget, x, y, self.language());
            }
            Ok(None) => {}
            Err(error) => self.report_app_error(error),
        }
    }

    fn on_region_motion(&mut self, x: f64, y: f64) {
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        let x = bounds.left().saturating_add(x.round() as i32);
        let y = bounds.top().saturating_add(y.round() as i32);
        self.resize_dragging_splitter_at_root(bounds, x, y);
    }

    fn on_region_release(&mut self) {
        self.finish_splitter_drag();
    }

    fn begin_splitter_drag_at_root(
        &mut self,
        tab_id: TabId,
        bounds: Rect,
        root_x: i32,
        root_y: i32,
    ) -> bool {
        match self
            .app
            .hit_test_splitter(tab_id, bounds, root_x, root_y, SPLITTER_HIT_TOLERANCE)
        {
            Ok(Some(splitter)) => {
                self.dragging_splitter = Some(splitter.path().clone());
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.report_app_error(error);
                true
            }
        }
    }

    fn begin_splitter_drag_at_current_bounds(&mut self, root_x: i32, root_y: i32) -> bool {
        let Some(tab_id) = self.app.active_tab_id() else {
            return false;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return false;
        };
        self.begin_splitter_drag_at_root(tab_id, bounds, root_x, root_y)
    }

    fn resize_dragging_splitter_at_root(&mut self, bounds: Rect, root_x: i32, root_y: i32) -> bool {
        let Some(path) = self.dragging_splitter.clone() else {
            return false;
        };
        let Some(tab_id) = self.app.active_tab_id() else {
            return false;
        };

        match self
            .app
            .resize_splitter(tab_id, &path, bounds, root_x, root_y)
        {
            Ok(()) => {
                self.widgets.drawing_area.queue_draw();
                true
            }
            Err(error) => {
                self.report_app_error(error);
                false
            }
        }
    }

    fn finish_splitter_drag(&mut self) -> bool {
        if self.dragging_splitter.take().is_some() {
            self.record_workspace_change();
            true
        } else {
            false
        }
    }

    fn draw(&self, context: &cairo::Context, width: i32, height: i32) {
        set_source_rgb(context, 0xF8, 0xF8, 0xF8);
        let _ = context.paint();
        let Some(tab_id) = self.app.active_tab_id() else {
            draw_centered_text(
                context,
                width,
                height,
                t(self.language(), "No active tab", "활성 탭 없음"),
            );
            return;
        };
        let Ok(bounds) = Rect::new(0, 0, width.max(1), height.max(1)) else {
            return;
        };
        let Ok(regions) = self.app.layout_for_tab(tab_id, bounds) else {
            draw_centered_text(
                context,
                width,
                height,
                t(
                    self.language(),
                    "Layout cannot be calculated",
                    "레이아웃을 계산할 수 없습니다.",
                ),
            );
            return;
        };
        let placements = self
            .app
            .state()
            .workspace()
            .placements_for_tab(tab_id)
            .unwrap_or(&[]);
        let language = self.language();
        for region in regions {
            let occupied = placements
                .iter()
                .any(|placement| placement.region_id() == region.region_id());
            draw_region(
                context,
                language,
                region.region_id(),
                region.rect(),
                self.active_region,
                occupied,
            );
        }
        if let Ok(splitters) = self
            .app
            .splitter_rects(tab_id, bounds, SPLITTER_HIT_TOLERANCE)
        {
            for splitter in splitters {
                let rect = splitter.rect();
                set_source_rgb(context, 0x90, 0x90, 0x90);
                context.rectangle(
                    f64::from(rect.left()),
                    f64::from(rect.top()),
                    f64::from(rect.width()),
                    f64::from(rect.height()),
                );
                let _ = context.fill();
            }
        }
    }

    fn layout_bounds_client(&self) -> Option<Rect> {
        let (_, _, width, height) = self.layout_allocation_in_window();
        Rect::new(0, 0, width.max(1), height.max(1)).ok()
    }

    fn layout_bounds_screen(&self) -> Option<Rect> {
        let client = self.layout_bounds_client()?;
        let Some(surface) = self.widgets.window.surface() else {
            return Some(client);
        };
        let Ok(x11_surface) = surface.downcast::<gdk4_x11::X11Surface>() else {
            return Some(client);
        };
        let window_rect = self
            .app
            .controller()
            .current_rect_for_xid(x11_surface.xid())
            .ok()?;
        let (allocation_x, allocation_y, _, _) = self.layout_allocation_in_window();
        let left = window_rect.left().checked_add(allocation_x)?;
        let top = window_rect.top().checked_add(allocation_y)?;

        Rect::new(left, top, client.width(), client.height()).ok()
    }

    fn layout_allocation_in_window(&self) -> (i32, i32, i32, i32) {
        if self.workspace_ui_visible {
            let allocation = self.widgets.drawing_area.allocation();
            return (
                allocation.x(),
                allocation.y(),
                allocation.width(),
                allocation.height(),
            );
        }

        let root_width = self.widgets.root.allocated_width();
        let root_height = self.widgets.root.allocated_height();
        let top_bar = self.widgets.top_bar.allocation();
        let top = top_bar.y().saturating_add(top_bar.height());
        let height = root_height.saturating_sub(top);
        (0, top, root_width, height)
    }

    fn sync_active_tab_if_bounds_changed(&mut self) {
        if self.main_window_minimized {
            return;
        }
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        if self.last_active_tab_sync_bounds == Some(bounds) {
            return;
        }
        self.sync_active_tab_to_bounds(bounds);
    }

    fn sync_active_tab_to_current_bounds(&mut self) {
        if self.main_window_minimized {
            return;
        }
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        self.sync_active_tab_to_bounds(bounds);
    }

    fn show_active_tab_at_current_bounds(&mut self) {
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };
        match self.app.show_active_tab(bounds) {
            Ok(removed_stale) => {
                if removed_stale > 0 {
                    self.record_workspace_change();
                }
                self.last_active_tab_sync_bounds = Some(bounds);
            }
            Err(error) => {
                self.last_active_tab_sync_bounds = None;
                self.report_app_error(error);
            }
        }
    }

    fn sync_active_tab_to_bounds(&mut self, bounds: Rect) {
        match self.app.sync_active_tab(bounds) {
            Ok(removed_stale) => {
                if removed_stale > 0 {
                    self.record_workspace_change();
                }
                self.last_active_tab_sync_bounds = Some(bounds);
            }
            Err(error) => {
                self.last_active_tab_sync_bounds = None;
                self.report_app_error(error);
            }
        }
    }

    fn poll_splitter_overlay(&mut self) {
        if !self.external_window_control_available() {
            self.hide_splitter_overlay();
            self.splitter_overlay_last_left_button_down = false;
            return;
        }

        let pointer = match self.app.controller().pointer_state() {
            Ok(pointer) => pointer,
            Err(_) => {
                self.hide_splitter_overlay();
                self.splitter_overlay_last_left_button_down = false;
                return;
            }
        };

        if self.dragging_splitter.is_some() {
            self.hide_splitter_overlay();
            if pointer.left_button_down() {
                if let Some(bounds) = self.layout_bounds_screen() {
                    self.resize_dragging_splitter_at_root(
                        bounds,
                        pointer.root_x(),
                        pointer.root_y(),
                    );
                }
            } else {
                self.finish_splitter_drag();
            }
            self.splitter_overlay_last_left_button_down = pointer.left_button_down();
            return;
        }

        if !splitter_overlay_should_show(
            self.workspace_ui_visible,
            self.workspace_options.dock_hidden_workspace_ui(),
            self.widgets.window.is_active(),
            self.main_window_minimized,
            self.pointer_drag_active(),
            pointer.control_down(),
        ) {
            self.hide_splitter_overlay();
            self.splitter_overlay_last_left_button_down = pointer.left_button_down();
            return;
        }

        if let Err(error) = self.sync_splitter_overlay() {
            self.hide_splitter_overlay();
            self.report_app_error(error);
            self.splitter_overlay_last_left_button_down = pointer.left_button_down();
            return;
        }

        if pointer.left_button_down()
            && !self.splitter_overlay_last_left_button_down
            && self.begin_splitter_drag_at_current_bounds(pointer.root_x(), pointer.root_y())
        {
            self.hide_splitter_overlay();
        }
        self.splitter_overlay_last_left_button_down = pointer.left_button_down();
    }

    fn pointer_drag_active(&self) -> bool {
        self.dragging_splitter.is_some()
            || self.drop_candidate.is_some()
            || self.tab_reorder_drag.is_some()
            || self.main_window_move_drag.is_some()
    }

    fn sync_splitter_overlay(&mut self) -> Result<(), AppError> {
        let Some(active_tab) = self.app.active_tab_id() else {
            self.hide_splitter_overlay();
            return Ok(());
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            self.hide_splitter_overlay();
            return Ok(());
        };
        let splitters = self
            .app
            .splitter_rects(active_tab, bounds, SPLITTER_HIT_TOLERANCE)?;
        let rects = splitters
            .into_iter()
            .map(|splitter| splitter.rect())
            .collect::<Vec<_>>();
        self.splitter_overlay
            .sync(self.app.controller_mut(), &rects)?;
        Ok(())
    }

    fn hide_splitter_overlay(&mut self) {
        self.splitter_overlay.hide_all(self.app.controller_mut());
    }

    fn teardown_splitter_overlay(&mut self) {
        self.splitter_overlay.destroy_all(self.app.controller_mut());
    }

    fn poll_external_drop(&mut self, owner: &Rc<RefCell<Self>>) {
        if self.main_window_minimized {
            self.drop_candidate = None;
            return;
        }
        if !self.external_window_control_available() {
            self.drop_candidate = None;
            return;
        }
        self.sync_active_region_from_active_window();
        let pointer = match self.app.controller().pointer_state() {
            Ok(pointer) => pointer,
            Err(_) => {
                self.drop_candidate = None;
                return;
            }
        };
        if pointer.control_down() || self.dragging_splitter.is_some() {
            self.drop_candidate = None;
            return;
        }

        if pointer.left_button_down() {
            self.track_external_press(pointer);
            return;
        }

        let Some(candidate) = self.drop_candidate.take() else {
            return;
        };
        if candidate.moved {
            self.try_place_or_detach_drop(
                owner,
                candidate.hwnd,
                pointer.root_x(),
                pointer.root_y(),
            );
        }
    }

    fn track_external_press(&mut self, pointer: LinuxPointerState) {
        if let Some(candidate) = self.drop_candidate.as_mut() {
            match self.app.controller().current_rect(candidate.hwnd) {
                Ok(rect) => candidate.observe_rect(rect),
                Err(_) => candidate.moved = false,
            }
            return;
        }

        let Some(hwnd) = pointer.hwnd() else {
            return;
        };
        if let Ok(Some(region_id)) = self.app.active_tab_region_for_window(hwnd) {
            self.select_active_region_for_placed_window(region_id);
        }
        let Ok(rect) = self.app.controller().current_rect(hwnd) else {
            return;
        };
        self.drop_candidate = Some(LinuxDropCandidate::new(hwnd, rect));
    }

    fn sync_active_region_from_active_window(&mut self) {
        let active_window = match self.app.controller().active_window() {
            Ok(Some(hwnd)) => hwnd,
            Ok(None) | Err(_) => return,
        };
        let region = match self.app.active_tab_region_for_window(active_window) {
            Ok(Some(region)) => region,
            Ok(None) | Err(_) => return,
        };
        if self.active_region != Some(region) {
            self.select_active_region_for_placed_window(region);
        }
    }

    fn select_active_region_for_placed_window(&mut self, region_id: RegionId) -> bool {
        if self.active_region == Some(region_id) {
            return false;
        }

        self.active_region = Some(region_id);
        self.status = docked_window_selection_status_text(self.language()).to_owned();
        self.refresh_status();
        self.widgets.drawing_area.queue_draw();
        true
    }

    fn try_place_or_detach_drop(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        hwnd: WindowHandle,
        root_x: i32,
        root_y: i32,
    ) {
        let Some(tab_id) = self.app.active_tab_id() else {
            return;
        };
        let Some(bounds) = self.layout_bounds_screen() else {
            return;
        };

        let hit_test_enabled = drop_uses_workspace_hit_test(
            self.workspace_ui_visible,
            self.workspace_options.dock_hidden_workspace_ui(),
        );
        if hit_test_enabled {
            match self.app.hit_test_region(tab_id, bounds, root_x, root_y) {
                Ok(Some(region_id)) => {
                    self.place_dropped_window(owner, tab_id, region_id, hwnd, bounds);
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    self.report_app_error(error);
                    self.refresh_all(owner);
                    return;
                }
            }
        }

        self.detach_dropped_window(owner, hwnd, root_x, root_y);
    }

    fn place_dropped_window(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        bounds: Rect,
    ) {
        let source_region_id = match self.app.active_tab_region_for_window(hwnd) {
            Ok(region_id) => region_id,
            Err(error) => {
                self.report_app_error(error);
                self.refresh_all(owner);
                return;
            }
        };

        match self.app.register_placement(tab_id, region_id, hwnd, bounds) {
            Ok(registration) => {
                self.record_workspace_change();
                self.active_region = Some(registration.target_region_id());
                self.status =
                    placement_registration_status_text(self.language(), registration).to_owned();
            }
            Err(error) => self.report_drop_registration_error(source_region_id, region_id, error),
        }
        self.refresh_all(owner);
    }

    fn detach_dropped_window(
        &mut self,
        owner: &Rc<RefCell<Self>>,
        hwnd: WindowHandle,
        root_x: i32,
        root_y: i32,
    ) {
        if self.main_window_contains(root_x, root_y) {
            return;
        }
        match self.app.active_tab_region_for_window(hwnd) {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(error) => {
                self.report_app_error(error);
                self.refresh_all(owner);
                return;
            }
        }
        let Ok(rect) = self.app.controller().current_rect(hwnd) else {
            self.set_localized_status(
                "External window detach failed: current position could not be read.",
                "외부 윈도우 detach 실패: 현재 위치를 조회할 수 없습니다.",
            );
            return;
        };
        match self.app.detach_active_placement_at(hwnd, rect) {
            Ok(Some(status)) => {
                self.record_workspace_change();
                self.active_region = None;
                self.status = drop_detach_success_status_text(self.language(), status).to_owned();
            }
            Ok(None) => {}
            Err(error) => self.report_app_error(error),
        }
        self.refresh_all(owner);
    }

    fn poll_tab_preset_restore(&mut self, owner: &Rc<RefCell<Self>>) {
        self.reap_released_preset_children();

        let Some(mut restore) = self.preset_restore.take() else {
            return;
        };

        restore.observe_child_statuses();
        restore.refresh_tracked_processes();
        let matches = match self
            .app
            .controller()
            .top_level_windows_for_processes(restore.tracked_process_ids())
        {
            Ok(matches) => matches,
            Err(error) => {
                let children = restore.fail_remaining_with_message(linux_window_error_message(
                    self.language(),
                    &error,
                ));
                self.track_released_preset_children(children);
                self.status = restore.finished_status_text();
                self.refresh_all(owner);
                return;
            }
        };

        let Some(bounds) = self.layout_bounds_screen() else {
            self.status = t(
                self.language(),
                "Waiting to dock preset programs because workspace bounds could not be calculated.",
                "작업 영역 좌표를 계산할 수 없어 preset 프로그램 dock을 기다립니다.",
            )
            .to_owned();
            self.preset_restore = Some(restore);
            self.refresh_status();
            return;
        };

        let mut index = 0usize;
        while index < restore.pending_len() {
            let Some(hwnd) = restore.matching_hwnd(index, &matches) else {
                index += 1;
                continue;
            };

            let released = restore.remove_pending(index).release();
            self.track_released_preset_child(released.child);
            match self.app.register_placement(
                restore.target_tab_id(),
                released.region_id,
                hwnd,
                bounds,
            ) {
                Ok(registration) => {
                    restore.record_docked();
                    self.record_workspace_change();
                    self.active_region = Some(registration.target_region_id());
                }
                Err(error) => restore.record_failure(LinuxTabPresetProgramFailure {
                    label: released.label,
                    message: linux_app_error_message(self.language(), &error),
                }),
            }
        }

        let children = restore.fail_timed_out(Instant::now());
        self.track_released_preset_children(children);
        if restore.has_pending() {
            self.status = restore.status_text();
            self.preset_restore = Some(restore);
        } else {
            self.status = restore.finished_status_text();
        }
        self.refresh_all(owner);
    }

    fn track_released_preset_child(&mut self, child: Option<Child>) {
        if let Some(child) = child {
            self.released_preset_children.push(child);
        }
    }

    fn track_released_preset_children(&mut self, children: Vec<Child>) {
        self.released_preset_children.extend(children);
    }

    fn reap_released_preset_children(&mut self) {
        let mut index = 0usize;
        while index < self.released_preset_children.len() {
            let should_remove = match self.released_preset_children[index].try_wait() {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(error) => {
                    eprintln!("failed to observe released preset child process: {error}");
                    true
                }
            };
            if should_remove {
                let mut child = self.released_preset_children.swap_remove(index);
                let _ = child.wait();
            } else {
                index += 1;
            }
        }
    }

    fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
        self.refresh_status();
    }

    fn set_localized_status(&mut self, english: &'static str, korean: &'static str) {
        self.set_status(t(self.language(), english, korean));
    }

    fn report_app_error(&mut self, error: AppError) {
        log_app_error(&error);
        self.status = linux_app_error_message(self.language(), &error);
        self.refresh_status();
    }

    fn report_drop_registration_error(
        &mut self,
        source_region_id: Option<RegionId>,
        target_region_id: RegionId,
        error: AppError,
    ) {
        log_app_error(&error);
        self.status = drop_registration_error_status_text(
            self.language(),
            source_region_id,
            target_region_id,
            &error,
        );
        self.refresh_status();
    }

    fn report_tab_operation_error(&mut self, tab_id: TabId, operation: &str, error: AppError) {
        log_app_error(&error);
        self.status = tab_operation_error_status_text(self.language(), tab_id, operation, &error);
        self.refresh_status();
    }

    fn report_switch_tab_error(&mut self, context: &LinuxTabSwitchStatusContext, error: AppError) {
        log_app_error(&error);
        self.status = switch_tab_failure_status_text(
            self.language(),
            context,
            self.app.active_tab_id(),
            &error,
        );
        self.refresh_status();
    }

    fn report_tab_deletion_error(
        &mut self,
        context: &LinuxTabDeletionStatusContext,
        error: AppError,
    ) {
        log_app_error(&error);
        self.status = tab_deletion_error_status_text(
            self.language(),
            context,
            self.app.active_tab_id(),
            &error,
        );
        self.refresh_status();
    }

    fn record_workspace_change(&mut self) {
        self.settings_save_policy.allow_after_workspace_change();
        self.preserved_startup_session = None;
    }

    fn record_workspace_options_change(&mut self) {
        self.settings_save_policy
            .allow_after_workspace_options_change();
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

    fn shutdown(&mut self, mode: ShutdownMode) -> bool {
        if self.shutdown_done {
            self.teardown_splitter_overlay();
            self.cancel_tab_preset_restore();
            return true;
        }

        if mode == ShutdownMode::Forced {
            self.cancel_tab_preset_restore();
        }
        self.hide_splitter_overlay();
        let settings_save_result = self.save_settings_before_shutdown();
        let active_tab_hidden = self.main_window_minimized;
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
        if self.shutdown_done {
            self.teardown_splitter_overlay();
            self.cancel_tab_preset_restore();
        }
        self.shutdown_done
    }

    fn report_shutdown_attempt(&mut self, attempt: ShutdownAttemptReport) {
        log_undock_failures(&attempt.report);
        let undock = shutdown_undock_summary_text(self.language(), &attempt.report);
        if let Some(error) = attempt.settings_save_error {
            let settings_save_error_text =
                shutdown_settings_save_error_message(self.language(), &error);
            self.report_shutdown_settings_save_error(error);
            self.status = format!("{settings_save_error_text} {undock}");
        } else {
            self.status = undock;
        }
    }

    fn report_shutdown_settings_save_error(&mut self, error: ShutdownSettingsSaveError) {
        match &error {
            ShutdownSettingsSaveError::App(error) => log_app_error(error),
            ShutdownSettingsSaveError::Settings(error) => log_settings_error(error),
        }
        self.status = match error {
            ShutdownSettingsSaveError::App(error) => {
                linux_app_error_message(self.language(), &error)
            }
            ShutdownSettingsSaveError::Settings(error) => {
                linux_settings_error_message(self.language(), &error).to_owned()
            }
        };
    }
}

#[derive(Debug, Default)]
struct LinuxSplitterOverlayController {
    windows: Vec<LinuxOverlayWindow>,
    visible_count: usize,
    visible_rects: Vec<Rect>,
}

impl LinuxSplitterOverlayController {
    fn sync(
        &mut self,
        controller: &mut DefaultWindowController,
        rects: &[Rect],
    ) -> Result<bool, AppError> {
        if rects.is_empty() {
            self.hide_all(controller);
            return Ok(false);
        }

        while self.windows.len() < rects.len() {
            self.windows.push(
                controller
                    .create_splitter_overlay_window()
                    .map_err(AppError::from)?,
            );
        }

        for (index, rect) in rects.iter().enumerate() {
            if self.visible_rect_at(index) == Some(*rect) {
                continue;
            }

            controller
                .set_splitter_overlay_rect(self.windows[index], *rect)
                .map_err(AppError::from)?;
            self.remember_visible_rect(index, *rect);
        }

        for window in self
            .windows
            .iter()
            .skip(rects.len())
            .take(self.visible_count.saturating_sub(rects.len()))
        {
            let _ = controller.hide_splitter_overlay_window(*window);
        }
        self.visible_count = rects.len();
        self.visible_rects.truncate(rects.len());

        Ok(true)
    }

    fn hide_all(&mut self, controller: &mut DefaultWindowController) {
        for window in self.windows.iter().take(self.visible_count) {
            let _ = controller.hide_splitter_overlay_window(*window);
        }
        self.visible_count = 0;
        self.visible_rects.clear();
    }

    fn destroy_all(&mut self, controller: &mut DefaultWindowController) {
        self.visible_count = 0;
        self.visible_rects.clear();
        for window in self.windows.drain(..) {
            let _ = controller.destroy_splitter_overlay_window(window);
        }
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

#[derive(Debug, Clone, Copy)]
struct LinuxDropCandidate {
    hwnd: WindowHandle,
    initial_rect: Rect,
    moved: bool,
}

#[derive(Debug, Clone, Copy)]
struct LinuxMainWindowMoveDrag {
    xid: u64,
    initial_rect: Rect,
}

impl LinuxDropCandidate {
    const fn new(hwnd: WindowHandle, initial_rect: Rect) -> Self {
        Self {
            hwnd,
            initial_rect,
            moved: false,
        }
    }

    fn observe_rect(&mut self, rect: Rect) {
        let dx = rect.left().abs_diff(self.initial_rect.left());
        let dy = rect.top().abs_diff(self.initial_rect.top());
        let dw = rect.width().abs_diff(self.initial_rect.width());
        let dh = rect.height().abs_diff(self.initial_rect.height());
        let threshold = DROP_WINDOW_MOVE_THRESHOLD as u32;
        if dx >= threshold || dy >= threshold || dw >= threshold || dh >= threshold {
            self.moved = true;
        }
    }
}

fn drag_offset_to_i32(offset: f64) -> i32 {
    if offset.is_nan() {
        0
    } else if offset <= f64::from(i32::MIN) {
        i32::MIN
    } else if offset >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        offset.round() as i32
    }
}

#[derive(Debug, Clone, Copy)]
struct LinuxTabReorderDrag {
    tab_id: TabId,
    start_pointer_x: f64,
    insertion: Option<LinuxTabReorderInsertion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxTabReorderInsertion {
    before_tab_id: Option<TabId>,
}

fn visible_tab_capacity_for_width(width: i32, tab_count: usize) -> usize {
    let width = width.max(0);
    let step = GTK_TAB_WIDTH.saturating_add(GTK_TAB_GAP).max(1);
    let capacity = width.saturating_add(GTK_TAB_GAP) / step;
    usize::try_from(capacity).map_or(0, |capacity| capacity.min(tab_count))
}

fn visible_tab_capacity_for_tab_strip_width(width: i32, tab_count: usize) -> usize {
    let width = width.max(0);
    let tab_width = tabs_total_width(tab_count);
    let viewport_width = if tab_width > width && width > 0 {
        width
            .saturating_sub(TAB_OVERFLOW_BUTTON_WIDTH)
            .saturating_sub(GTK_TAB_GAP)
    } else {
        width
    };
    visible_tab_capacity_for_width(viewport_width, tab_count)
}

fn tabs_total_width(tab_count: usize) -> i32 {
    if tab_count == 0 {
        return 0;
    }
    let Ok(tab_count) = i32::try_from(tab_count) else {
        return i32::MAX;
    };
    let tab_width = GTK_TAB_WIDTH.saturating_mul(tab_count);
    let gaps = GTK_TAB_GAP.saturating_mul(tab_count.saturating_sub(1));
    tab_width.saturating_add(gaps)
}

fn linux_tab_overflow_range(
    tab_count: usize,
    visible_count: usize,
    current_first_visible_index: usize,
    active_index: Option<usize>,
    preserve_current_first: bool,
) -> (usize, usize) {
    if visible_count >= tab_count {
        return (0, tab_count);
    }
    if visible_count == 0 {
        return (0, 0);
    }

    let max_first = tab_count.saturating_sub(visible_count);
    let mut first = current_first_visible_index.min(max_first);
    if !preserve_current_first
        && let Some(active_index) = active_index.filter(|index| *index < tab_count)
    {
        if active_index < first {
            first = active_index;
        } else if active_index >= first + visible_count {
            first = active_index + 1 - visible_count;
        }
    }
    first = first.min(max_first);
    (first, first + visible_count)
}

fn linux_tab_reorder_insertion_index_for_x(
    pointer_x: f64,
    tab_count: usize,
    visible_count: usize,
    first_visible_index: usize,
) -> Option<usize> {
    if tab_count == 0 || visible_count == 0 {
        return None;
    }

    let visible_start = first_visible_index.min(tab_count.saturating_sub(visible_count));
    let visible_end = visible_start.saturating_add(visible_count).min(tab_count);
    if visible_start >= visible_end {
        return None;
    }

    let step = GTK_TAB_WIDTH.saturating_add(GTK_TAB_GAP).max(1);
    for index in visible_start..visible_end {
        let relative = index.saturating_sub(visible_start) as f64;
        let midpoint = relative.mul_add(f64::from(step), f64::from(GTK_TAB_WIDTH) / 2.0);
        if pointer_x < midpoint {
            return Some(index);
        }
    }

    Some(visible_end)
}

fn linux_tab_reorder_scroll_first_index(
    pointer_x: f64,
    tab_bar_width: i32,
    tab_count: usize,
    visible_count: usize,
    first_visible_index: usize,
) -> Option<usize> {
    if visible_count == 0 || visible_count >= tab_count {
        return None;
    }

    let max_first = tab_count.saturating_sub(visible_count);
    let first = first_visible_index.min(max_first);
    let visible_end = first.saturating_add(visible_count).min(tab_count);
    if first > 0 && pointer_x <= f64::from(GTK_TAB_REORDER_SCROLL_ZONE) {
        return Some(first - 1);
    }

    let width = tab_bar_width.max(0);
    if visible_end < tab_count
        && pointer_x >= f64::from(width.saturating_sub(GTK_TAB_REORDER_SCROLL_ZONE))
    {
        return Some(first + 1);
    }

    None
}

fn drop_uses_workspace_hit_test(
    workspace_ui_visible: bool,
    dock_hidden_workspace_ui: bool,
) -> bool {
    workspace_ui_visible || dock_hidden_workspace_ui
}

fn splitter_overlay_should_show(
    workspace_ui_visible: bool,
    dock_hidden_workspace_ui: bool,
    main_window_active: bool,
    is_minimized: bool,
    pointer_drag_active: bool,
    control_down: bool,
) -> bool {
    splitter_overlay_workspace_enabled(workspace_ui_visible, dock_hidden_workspace_ui)
        && main_window_active
        && !is_minimized
        && !pointer_drag_active
        && control_down
}

fn splitter_overlay_workspace_enabled(
    workspace_ui_visible: bool,
    dock_hidden_workspace_ui: bool,
) -> bool {
    workspace_ui_visible || dock_hidden_workspace_ui
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LinuxUndockCounts {
    attempted: usize,
    restored: usize,
    missing: usize,
    failures: usize,
}

impl LinuxUndockCounts {
    fn add_report(&mut self, report: &UndockReport) {
        self.attempted = self.attempted.saturating_add(report.attempted());
        self.restored = self.restored.saturating_add(report.restored());
        self.missing = self.missing.saturating_add(report.missing());
        self.failures = self.failures.saturating_add(report.failures().len());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabOperationFailure {
    tab_id: TabId,
    operation: &'static str,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabStatusLabel {
    tab_id: TabId,
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabSwitchStatusContext {
    target: LinuxTabStatusLabel,
    previous_active: Option<LinuxTabStatusLabel>,
}

impl LinuxTabSwitchStatusContext {
    fn is_reselecting_active_tab(&self) -> bool {
        self.previous_active
            .as_ref()
            .is_some_and(|previous| previous.tab_id == self.target.tab_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabDeletionStatusContext {
    deleted: LinuxTabStatusLabel,
    previous_active: Option<LinuxTabStatusLabel>,
    automatic_target: Option<LinuxTabStatusLabel>,
}

fn close_other_target_missing_status_text(language: UiLanguage, target_tab_id: TabId) -> String {
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

fn close_other_bounds_failure_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "Close other tabs failed: workspace bounds could not be calculated. Undock: not attempted",
        "Close other tabs 실패: 작업 영역 좌표를 계산할 수 없습니다. Undock: 시도하지 않음",
    )
}

fn tab_rename_success_status_text(language: UiLanguage, tab_id: TabId) -> String {
    match language {
        UiLanguage::English => format!("Tab {} renamed.", tab_id.value()),
        UiLanguage::Korean => format!("탭 {} 이름을 변경했습니다.", tab_id.value()),
    }
}

fn tab_rename_cancel_status_text(language: UiLanguage, tab_id: TabId) -> String {
    match language {
        UiLanguage::English => format!("Tab {} rename canceled.", tab_id.value()),
        UiLanguage::Korean => format!("탭 {} 이름 변경을 취소했습니다.", tab_id.value()),
    }
}

fn tab_reorder_status_text(
    language: UiLanguage,
    tab_id: TabId,
    before_tab_id: Option<TabId>,
    changed: bool,
) -> String {
    let destination = tab_reorder_destination_text(language, before_tab_id);
    match (language, changed) {
        (UiLanguage::English, true) => {
            format!("Tab order changed: tab {} -> {destination}", tab_id.value())
        }
        (UiLanguage::English, false) => format!(
            "Tab order was not changed: tab {} stayed in the same position.",
            tab_id.value()
        ),
        (UiLanguage::Korean, true) => {
            format!(
                "탭 순서를 변경했습니다: 탭 {} -> {destination}",
                tab_id.value()
            )
        }
        (UiLanguage::Korean, false) => {
            format!(
                "탭 순서를 변경하지 않았습니다: 탭 {} 위치가 그대로입니다.",
                tab_id.value()
            )
        }
    }
}

fn tab_reorder_destination_text(language: UiLanguage, before_tab_id: Option<TabId>) -> String {
    match before_tab_id {
        Some(tab_id) => match language {
            UiLanguage::English => format!("before tab {}", tab_id.value()),
            UiLanguage::Korean => format!("탭 {} 앞", tab_id.value()),
        },
        None => t(language, "last position", "마지막 위치").to_owned(),
    }
}

fn tab_operation_error_status_text(
    language: UiLanguage,
    tab_id: TabId,
    operation: &str,
    error: &AppError,
) -> String {
    match language {
        UiLanguage::English => format!(
            "Tab {} {} failed: {}",
            tab_id.value(),
            tab_operation_label(language, operation),
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "탭 {} {} 실패: {}",
            tab_id.value(),
            tab_operation_label(language, operation),
            linux_app_error_message(language, error)
        ),
    }
}

fn docked_window_selection_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "Selected the docked external window region. Drag to an empty region to move it, or outside to undock it.",
        "배치된 외부 윈도우 영역을 선택했습니다. 빈 영역으로 끌면 이동, 바깥으로 끌면 배치 해제합니다.",
    )
}

fn tab_preset_save_success_status_text(
    language: UiLanguage,
    preset_name: &str,
    program_count: usize,
) -> String {
    match language {
        UiLanguage::English => {
            format!("Tab preset saved: {preset_name}. Program(s): {program_count}")
        }
        UiLanguage::Korean => {
            format!("탭 preset 저장 완료: {preset_name}. 프로그램 {program_count}개")
        }
    }
}

fn tab_preset_save_missing_tab_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "There is no tab to save.",
        "저장할 탭이 없습니다.",
    )
}

fn tab_preset_load_missing_active_tab_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "There is no active tab to load the tab preset into.",
        "탭 preset을 적용할 활성 탭이 없습니다.",
    )
}

fn tab_preset_load_missing_tab_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "There is no tab to load the tab preset into.",
        "탭 preset을 적용할 탭이 없습니다.",
    )
}

fn tab_rename_prompt_label_text(language: UiLanguage) -> &'static str {
    t(language, "Tab name:", "탭 이름:")
}

fn tab_preset_empty_status_text(language: UiLanguage, action: PresetAction) -> &'static str {
    match action {
        PresetAction::Load => t(
            language,
            "There are no saved tab presets.",
            "저장된 탭 preset이 없습니다.",
        ),
        PresetAction::Edit => t(
            language,
            "There are no saved tab presets to edit.",
            "편집할 저장된 탭 preset이 없습니다.",
        ),
        PresetAction::Delete => t(
            language,
            "There are no saved tab presets to delete.",
            "삭제할 저장된 탭 preset이 없습니다.",
        ),
    }
}

fn tab_preset_edit_success_status_text(
    language: UiLanguage,
    preset_name: &str,
    program_count: usize,
) -> String {
    match language {
        UiLanguage::English => {
            format!("Tab preset edited: {preset_name}. Program(s): {program_count}")
        }
        UiLanguage::Korean => {
            format!("탭 preset 편집 완료: {preset_name}. 프로그램 {program_count}개")
        }
    }
}

fn tab_preset_delete_success_status_text(language: UiLanguage, preset_name: &str) -> String {
    match language {
        UiLanguage::English => format!("Tab preset deleted: {preset_name}"),
        UiLanguage::Korean => format!("탭 preset 삭제 완료: {preset_name}"),
    }
}

fn tab_preset_save_cancel_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "Tab preset save was canceled.",
        "탭 preset 저장을 취소했습니다.",
    )
}

fn tab_preset_edit_cancel_status_text(language: UiLanguage) -> &'static str {
    t(
        language,
        "Tab preset edit was canceled.",
        "탭 preset 편집을 취소했습니다.",
    )
}

fn tab_preset_edit_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    error: &AppError,
) -> String {
    match language {
        UiLanguage::English => format!(
            "Tab preset edit failed: {preset_name}. Cause: {}",
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "탭 preset 편집 실패: {preset_name}. 원인: {}",
            linux_app_error_message(language, error)
        ),
    }
}

fn tab_preset_delete_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    error: &AppError,
) -> String {
    match language {
        UiLanguage::English => format!(
            "Tab preset delete failed: {preset_name}. Cause: {}",
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "탭 preset 삭제 실패: {preset_name}. 원인: {}",
            linux_app_error_message(language, error)
        ),
    }
}

fn tab_preset_apply_success_status_text_for_preset(
    language: UiLanguage,
    preset_name: &str,
    target: &LinuxTabStatusLabel,
    undocked: usize,
    restore: &LinuxTabPresetProgramRestoreReport,
) -> String {
    let target = tab_status_label_text(language, target);
    let failures = restore.failures.len();
    match language {
        UiLanguage::English => format!(
            "Tab preset loaded: {} -> {target}. Existing undocked: {undocked}. Programs docked {}/{}; failures {}{}",
            preset_name,
            restore.docked,
            restore.expected,
            failures,
            tab_preset_program_failures_suffix(language, &restore.failures)
        ),
        UiLanguage::Korean => format!(
            "탭 preset 불러오기 완료: {} -> {target}. 기존 Undock {undocked}개. 프로그램 dock {}/{}개, 실패 {}개{}",
            preset_name,
            restore.docked,
            restore.expected,
            failures,
            tab_preset_program_failures_suffix(language, &restore.failures)
        ),
    }
}

fn tab_preset_apply_failure_status_text(
    language: UiLanguage,
    preset_name: &str,
    target: &LinuxTabStatusLabel,
    error: &AppError,
) -> String {
    let target = tab_status_label_text(language, target);
    match language {
        UiLanguage::English => format!(
            "Tab preset load failed: {preset_name} -> {target}. Cause: {}",
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "탭 preset 불러오기 실패: {preset_name} -> {target}. 원인: {}",
            linux_app_error_message(language, error)
        ),
    }
}

fn tab_preset_program_failures_suffix(
    language: UiLanguage,
    failures: &[LinuxTabPresetProgramFailure],
) -> String {
    if failures.is_empty() {
        return String::new();
    }

    let mut text = match language {
        UiLanguage::English => String::from(". Failed: "),
        UiLanguage::Korean => String::from(". 실패: "),
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

fn placement_registration_status_text(
    language: UiLanguage,
    registration: PlacementRegistration,
) -> &'static str {
    match registration {
        PlacementRegistration::Placed { .. } => t(
            language,
            "External window was docked into the region.",
            "외부 윈도우를 영역에 배치했습니다.",
        ),
        PlacementRegistration::Moved { .. } => t(
            language,
            "External window was moved to another region.",
            "외부 윈도우를 다른 영역으로 이동했습니다.",
        ),
        PlacementRegistration::Resynced { .. } => t(
            language,
            "External window was fitted to the current region again.",
            "외부 윈도우를 현재 영역에 다시 맞췄습니다.",
        ),
    }
}

fn drop_detach_success_status_text(language: UiLanguage, status: UndockStatus) -> &'static str {
    match status {
        UndockStatus::Restored => t(
            language,
            "External window was undocked at its current position.",
            "외부 윈도우를 현재 위치에서 배치 해제했습니다.",
        ),
        UndockStatus::WindowMissing => t(
            language,
            "External window placement was removed because the external window is no longer valid.",
            "외부 윈도우가 유효하지 않아 배치 정보를 제거했습니다.",
        ),
    }
}

fn drop_registration_error_status_text(
    language: UiLanguage,
    source_region_id: Option<RegionId>,
    target_region_id: RegionId,
    error: &AppError,
) -> String {
    let operation = drop_registration_operation_name(language, source_region_id, target_region_id);

    match error {
        AppError::Domain(DomainError::RegionAlreadyOccupied(region_id))
            if *region_id == target_region_id =>
        {
            match language {
                UiLanguage::English => format!(
                    "External window {operation} failed: target region already has another external window."
                ),
                UiLanguage::Korean => {
                    format!(
                        "외부 윈도우 {operation} 실패: 대상 영역에 이미 다른 외부 윈도우가 있습니다."
                    )
                }
            }
        }
        AppError::Domain(DomainError::InvalidWindowHandle) if source_region_id.is_some() => {
            match language {
                UiLanguage::English => format!(
                    "External window {operation} failed: docked window can no longer be verified."
                ),
                UiLanguage::Korean => {
                    format!("외부 윈도우 {operation} 실패: 배치된 창을 더 이상 확인할 수 없습니다.")
                }
            }
        }
        AppError::Window(error)
            if source_region_id.is_some() && error.operation() == WindowOperation::SetPosition =>
        {
            match language {
                UiLanguage::English => format!(
                    "External window {operation} failed: kept the previous region. {}",
                    linux_window_error_message(language, error)
                ),
                UiLanguage::Korean => format!(
                    "외부 윈도우 {operation} 실패: 기존 영역을 유지했습니다. {}",
                    linux_window_error_message(language, error)
                ),
            }
        }
        _ => match language {
            UiLanguage::English => format!(
                "External window {operation} failed: {}",
                linux_app_error_message(language, error)
            ),
            UiLanguage::Korean => format!(
                "외부 윈도우 {operation} 실패: {}",
                linux_app_error_message(language, error)
            ),
        },
    }
}

fn drop_registration_operation_name(
    language: UiLanguage,
    source_region_id: Option<RegionId>,
    target_region_id: RegionId,
) -> &'static str {
    match (language, source_region_id) {
        (UiLanguage::English, None) => "dock",
        (UiLanguage::English, Some(source_region_id)) if source_region_id == target_region_id => {
            "refit current region"
        }
        (UiLanguage::English, Some(_)) => "move",
        (UiLanguage::Korean, None) => "배치",
        (UiLanguage::Korean, Some(source_region_id)) if source_region_id == target_region_id => {
            "현재 영역 재맞춤"
        }
        (UiLanguage::Korean, Some(_)) => "이동",
    }
}

fn tab_deletion_status_text(language: UiLanguage, report: &TabDeletionReport) -> String {
    match language {
        UiLanguage::English => format!(
            "Tab {} deleted. Current active tab: {}. {}",
            report.deleted_tab_id().value(),
            active_tab_status_text(language, report.current_active_tab()),
            undock_summary_text(report.undock())
        ),
        UiLanguage::Korean => format!(
            "탭 {} 삭제 완료. 현재 활성 탭: {}. {}",
            report.deleted_tab_id().value(),
            active_tab_status_text(language, report.current_active_tab()),
            undock_summary_text(report.undock())
        ),
    }
}

fn switch_tab_failure_status_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    let operation = switch_tab_failure_operation_text(language, context, error);
    let target = tab_status_label_text(language, &context.target);
    let result = switch_tab_failure_result_text(language, context, current_active, error);
    match language {
        UiLanguage::English => format!(
            "{operation}: {target}. {result} Cause: {}",
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "{operation}: {target}. {result} 원인: {}",
            linux_app_error_message(language, error)
        ),
    }
}

fn switch_tab_failure_operation_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    error: &AppError,
) -> &'static str {
    match language {
        UiLanguage::English if context.is_reselecting_active_tab() => "Same tab redisplay failed",
        UiLanguage::English => match error {
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
        },
        UiLanguage::Korean if context.is_reselecting_active_tab() => "같은 탭 재표시 실패",
        UiLanguage::Korean => match error {
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
        },
    }
}

fn switch_tab_failure_result_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    let active_result = current_active_result_text(language, context, current_active);
    let rollback = match error {
        AppError::Window(error)
            if !context.is_reselecting_active_tab()
                && matches!(
                    error.operation(),
                    WindowOperation::SetPosition | WindowOperation::Hide
                ) =>
        {
            t(
                language,
                " Tried to roll back by hiding the target tab window.",
                " 대상 탭 창 숨김 롤백을 시도했습니다.",
            )
        }
        _ => "",
    };

    format!("{active_result}.{rollback}")
}

fn current_active_result_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    current_active: Option<TabId>,
) -> String {
    match (
        language,
        context.is_reselecting_active_tab(),
        current_active,
    ) {
        (UiLanguage::English, true, Some(tab_id)) if tab_id == context.target.tab_id => {
            format!(
                "Kept active tab {}",
                tab_status_label_text(language, &context.target)
            )
        }
        (UiLanguage::English, true, Some(tab_id)) => {
            format!("Current active tab is tab {}", tab_id.value())
        }
        (UiLanguage::English, true, None) => "There is no current active tab".to_owned(),
        (UiLanguage::English, false, Some(tab_id))
            if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) =>
        {
            format!(
                "Kept previous active tab {}",
                known_switch_tab_status_label_text(language, context, tab_id)
            )
        }
        (UiLanguage::English, false, Some(tab_id)) if tab_id == context.target.tab_id => {
            format!(
                "Target tab {} is active",
                known_switch_tab_status_label_text(language, context, tab_id)
            )
        }
        (UiLanguage::English, false, Some(tab_id)) => {
            format!("Current active tab is tab {}", tab_id.value())
        }
        (UiLanguage::English, false, None) => "There is no current active tab".to_owned(),
        (UiLanguage::Korean, true, Some(tab_id)) if tab_id == context.target.tab_id => {
            format!(
                "활성 탭 {}을 그대로 유지했습니다",
                tab_status_label_text(language, &context.target)
            )
        }
        (UiLanguage::Korean, true, Some(tab_id)) => {
            format!("현재 활성 탭은 탭 {}입니다", tab_id.value())
        }
        (UiLanguage::Korean, true, None) => "현재 활성 탭은 없습니다".to_owned(),
        (UiLanguage::Korean, false, Some(tab_id))
            if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) =>
        {
            format!(
                "이전 활성 탭 {}을 유지했습니다",
                known_switch_tab_status_label_text(language, context, tab_id)
            )
        }
        (UiLanguage::Korean, false, Some(tab_id)) if tab_id == context.target.tab_id => {
            format!(
                "대상 탭 {}이 활성 상태입니다",
                known_switch_tab_status_label_text(language, context, tab_id)
            )
        }
        (UiLanguage::Korean, false, Some(tab_id)) => {
            format!("현재 활성 탭은 탭 {}입니다", tab_id.value())
        }
        (UiLanguage::Korean, false, None) => "현재 활성 탭은 없습니다".to_owned(),
    }
}

fn tab_deletion_error_status_text(
    language: UiLanguage,
    context: &LinuxTabDeletionStatusContext,
    current_active: Option<TabId>,
    error: &AppError,
) -> String {
    if let Some(target) = &context.automatic_target
        && is_tab_activation_window_error(error)
    {
        return match language {
            UiLanguage::English => format!(
                "Automatic switch after tab delete failed: delete target {}, switch target {}. Rolled back the delete; current active tab: {}. Cause: {}. Undock: not completed because of failure",
                tab_status_label_text(language, &context.deleted),
                tab_status_label_text(language, target),
                tab_deletion_current_active_text(language, context, current_active),
                linux_app_error_message(language, error)
            ),
            UiLanguage::Korean => format!(
                "탭 삭제 후 자동 전환 실패: 삭제 대상 {}, 전환 대상 {}. 삭제를 롤백했고 현재 활성 탭: {}. 원인: {}. Undock: 실패로 완료되지 않음",
                tab_status_label_text(language, &context.deleted),
                tab_status_label_text(language, target),
                tab_deletion_current_active_text(language, context, current_active),
                linux_app_error_message(language, error)
            ),
        };
    }

    match language {
        UiLanguage::English => format!(
            "Tab delete failed: delete target {}. Rolled back the delete; current active tab: {}. Cause: {}. Undock: not completed because of failure",
            tab_status_label_text(language, &context.deleted),
            tab_deletion_current_active_text(language, context, current_active),
            linux_app_error_message(language, error)
        ),
        UiLanguage::Korean => format!(
            "탭 삭제 실패: 삭제 대상 {}. 삭제를 롤백했고 현재 활성 탭: {}. 원인: {}. Undock: 실패로 완료되지 않음",
            tab_status_label_text(language, &context.deleted),
            tab_deletion_current_active_text(language, context, current_active),
            linux_app_error_message(language, error)
        ),
    }
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
    language: UiLanguage,
    context: &LinuxTabDeletionStatusContext,
    current_active: Option<TabId>,
) -> String {
    match current_active {
        Some(tab_id) if Some(tab_id) == context.previous_active.as_ref().map(|tab| tab.tab_id) => {
            known_deletion_tab_status_label_text(language, context, tab_id)
        }
        Some(tab_id) if Some(tab_id) == context.automatic_target.as_ref().map(|tab| tab.tab_id) => {
            known_deletion_tab_status_label_text(language, context, tab_id)
        }
        Some(tab_id) => format!("{} {}", t(language, "tab", "탭"), tab_id.value()),
        None => t(language, "none", "없음").to_owned(),
    }
}

fn tab_switch_success_status_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    report: TabSwitchReport,
) -> String {
    let action = if context.is_reselecting_active_tab() {
        t(language, "Tab redisplayed", "탭을 다시 표시했습니다")
    } else {
        t(language, "Switched tab", "탭을 전환했습니다")
    };
    let mut text = format!(
        "{action}: {}",
        tab_status_label_text(language, &context.target)
    );

    if !context.is_reselecting_active_tab() {
        text.push_str(&format!(
            ". {}: {}",
            t(language, "Previous active tab", "이전 활성 탭"),
            optional_tab_status_label_text(language, context.previous_active.as_ref())
        ));
    }

    let removed = report.removed_stale_target_placements();
    if removed > 0 {
        text.push_str(&match language {
            UiLanguage::English => {
                format!(". Removed {removed} invalid target window(s) from placements")
            }
            UiLanguage::Korean => {
                format!(". 유효하지 않은 대상 창 {removed}개를 배치에서 제거했습니다")
            }
        });
    }
    let removed = report.removed_stale_previous_placements();
    if removed > 0 {
        text.push_str(&match language {
            UiLanguage::English => {
                format!(". Removed {removed} invalid previous active window(s) from placements")
            }
            UiLanguage::Korean => {
                format!(". 유효하지 않은 이전 활성 탭 창 {removed}개를 배치에서 제거했습니다")
            }
        });
    }

    text
}

fn close_other_tabs_status_text(
    language: UiLanguage,
    target_tab_id: TabId,
    total: usize,
    closed_count: usize,
    active_tab: Option<TabId>,
    undock: LinuxUndockCounts,
    failures: &[LinuxTabOperationFailure],
) -> String {
    let base = if failures.is_empty() {
        match language {
            UiLanguage::English => format!(
                "Close other tabs complete: closed {closed_count} tab(s) except tab {}.",
                target_tab_id.value()
            ),
            UiLanguage::Korean => format!(
                "Close other tabs 완료: 탭 {} 외 {closed_count}개 탭을 닫았습니다.",
                target_tab_id.value()
            ),
        }
    } else {
        match language {
            UiLanguage::English => format!(
                "Close other tabs partially failed: kept tab {}, success {closed_count}/{total}. Failures: {}.",
                target_tab_id.value(),
                tab_operation_failures_text(language, failures)
            ),
            UiLanguage::Korean => format!(
                "Close other tabs 일부 실패: 탭 {} 유지, 성공 {closed_count}/{total}. 실패: {}.",
                target_tab_id.value(),
                tab_operation_failures_text(language, failures)
            ),
        }
    };

    format!(
        "{base} {}: {}. Undock: attempted {}, restored {}, missing {}, failures {}",
        t(language, "Current active tab", "현재 활성 탭"),
        active_tab_status_text(language, active_tab),
        undock.attempted,
        undock.restored,
        undock.missing,
        undock.failures
    )
}

fn undock_summary_text(report: &UndockReport) -> String {
    format!(
        "Undock: attempted {}, restored {}, missing {}, failures {}",
        report.attempted(),
        report.restored(),
        report.missing(),
        report.failures().len()
    )
}

fn active_tab_status_text(language: UiLanguage, tab_id: Option<TabId>) -> String {
    match tab_id {
        Some(tab_id) => tab_id.value().to_string(),
        None => t(language, "none", "없음").to_owned(),
    }
}

fn tab_status_label_text(language: UiLanguage, label: &LinuxTabStatusLabel) -> String {
    let tab_word = t(language, "tab", "탭");
    match label.name.as_deref() {
        Some(name) => format!("{name} ({tab_word} {})", label.tab_id.value()),
        None => format!("{tab_word} {}", label.tab_id.value()),
    }
}

fn optional_tab_status_label_text(
    language: UiLanguage,
    label: Option<&LinuxTabStatusLabel>,
) -> String {
    match label {
        Some(label) => tab_status_label_text(language, label),
        None => t(language, "none", "없음").to_owned(),
    }
}

fn known_switch_tab_status_label_text(
    language: UiLanguage,
    context: &LinuxTabSwitchStatusContext,
    tab_id: TabId,
) -> String {
    if context.target.tab_id == tab_id {
        return tab_status_label_text(language, &context.target);
    }
    if let Some(previous) = &context.previous_active
        && previous.tab_id == tab_id
    {
        return tab_status_label_text(language, previous);
    }
    format!("{} {}", t(language, "tab", "탭"), tab_id.value())
}

fn known_deletion_tab_status_label_text(
    language: UiLanguage,
    context: &LinuxTabDeletionStatusContext,
    tab_id: TabId,
) -> String {
    if context.deleted.tab_id == tab_id {
        return tab_status_label_text(language, &context.deleted);
    }
    if let Some(previous) = &context.previous_active
        && previous.tab_id == tab_id
    {
        return tab_status_label_text(language, previous);
    }
    if let Some(target) = &context.automatic_target
        && target.tab_id == tab_id
    {
        return tab_status_label_text(language, target);
    }
    format!("{} {}", t(language, "tab", "탭"), tab_id.value())
}

fn tab_operation_failures_text(
    language: UiLanguage,
    failures: &[LinuxTabOperationFailure],
) -> String {
    let mut result = String::new();
    for (index, failure) in failures.iter().enumerate() {
        if index > 0 {
            result.push_str(", ");
        }
        result.push_str(&format!(
            "{} {} {}({})",
            t(language, "tab", "탭"),
            failure.tab_id.value(),
            tab_operation_label(language, failure.operation),
            failure.message
        ));
    }
    result
}

fn tab_operation_label(language: UiLanguage, operation: &str) -> &'static str {
    match (language, operation) {
        (UiLanguage::Korean, "rename" | "이름 변경") => "이름 변경",
        (UiLanguage::Korean, "reorder" | "순서 변경") => "순서 변경",
        (UiLanguage::Korean, "delete") => "삭제",
        (UiLanguage::Korean, _) => "작업",
        (UiLanguage::English, "rename" | "이름 변경") => "rename",
        (UiLanguage::English, "reorder" | "순서 변경") => "reorder",
        (UiLanguage::English, "delete") => "delete",
        (UiLanguage::English, _) => "operation",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PresetAction {
    Load,
    Edit,
    Delete,
}

impl PresetAction {
    fn title(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Load => t(language, "Load Tab Preset", "탭 preset 불러오기"),
            Self::Edit => t(language, "Edit Tab Preset", "탭 preset 편집"),
            Self::Delete => t(language, "Delete Tab Preset", "탭 preset 삭제"),
        }
    }
}

#[derive(Clone, Copy)]
enum PresetTarget {
    Active,
    Fixed(TabId),
}

impl PresetTarget {
    fn tab_id(self, owner: &Rc<RefCell<LinuxMainWindow>>) -> Option<TabId> {
        match self {
            Self::Active => owner.borrow().app.active_tab_id(),
            Self::Fixed(tab_id) => Some(tab_id),
        }
    }
}

type LinuxLoadedApp = (
    App<DefaultWindowController>,
    SettingsFileStore,
    SettingsSavePolicy,
    Option<PreservedStartupSessionSettings>,
    WorkspaceOptions,
    String,
);

fn load_app() -> Result<LinuxLoadedApp, EntryError> {
    let controller = DefaultWindowController::new();
    let settings_store = SettingsFileStore::for_current_exe()?;
    let mut settings_save_policy = SettingsSavePolicy::Enabled;
    let mut preserved_startup_session = None;
    let mut workspace_options = WorkspaceOptions::default();
    let mut status = "Ready".to_owned();
    let mut app = match settings_store.load_workspace_for_startup() {
        Ok(Some(settings)) => {
            workspace_options = settings.options();
            let saved_tab_count = settings.saved_tab_count();
            let saved_tab_preset_count = settings.tab_presets().len();
            let (tab_presets, startup_session) = settings.into_tab_presets_and_preserved_session();
            let state =
                crate::app::AppState::from_tab_presets_only(tab_presets, DEFAULT_MIN_REGION_SIZE)
                    .map_err(AppError::from)?;
            settings_save_policy = SettingsSavePolicy::PreserveStartupSessionUntilWorkspaceChange;
            preserved_startup_session = Some(startup_session);
            status = startup_saved_workspace_skipped_status_text(
                workspace_options.ui_language(),
                saved_tab_count,
                saved_tab_preset_count,
            );
            App::with_state(controller, state)
        }
        Ok(None) => App::new(controller),
        Err(error) => {
            log_settings_error(&error);
            status = settings_load_failure_status_text(workspace_options.ui_language(), &error);
            settings_save_policy = SettingsSavePolicy::WaitForWorkspaceChange;
            App::new(controller)
        }
    };

    if app.state().workspace().tabs().is_empty() {
        app.create_initial_tab("Tab 0")?;
    }

    Ok((
        app,
        settings_store,
        settings_save_policy,
        preserved_startup_session,
        workspace_options,
        status,
    ))
}

fn build_widgets(application: &gtk::Application) -> LinuxWidgets {
    install_app_css();
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title(WINDOW_TITLE)
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let menu_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let top_bar = gtk::Box::new(gtk::Orientation::Horizontal, TOP_BAR_BUTTON_SPACING);
    top_bar.set_height_request(TAB_BAR_HEIGHT);
    let workspace_toggle_button = gtk::Button::with_label("Hide");
    workspace_toggle_button.add_css_class("j3-top-button");
    workspace_toggle_button.set_width_request(WORKSPACE_TOGGLE_BUTTON_WIDTH);
    let new_tab_button = gtk::Button::with_label("New");
    new_tab_button.add_css_class("j3-top-button");
    new_tab_button.set_width_request(NEW_TAB_BUTTON_WIDTH);
    let tab_strip = gtk::Box::new(gtk::Orientation::Horizontal, GTK_TAB_GAP);
    tab_strip.set_hexpand(true);
    let tab_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    tab_bar.set_hexpand(true);
    let overflow_button = gtk::MenuButton::builder().label("...").build();
    overflow_button.add_css_class("j3-overflow-button");
    overflow_button.set_width_request(TAB_OVERFLOW_BUTTON_WIDTH);
    let command_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    command_bar.set_height_request(COMMAND_BAR_HEIGHT);
    let drawing_area = gtk::DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    drawing_area.set_content_width(DEFAULT_WIDTH);
    drawing_area.set_content_height(DEFAULT_HEIGHT);
    let status_label = gtk::Label::new(Some("Ready"));
    status_label.set_height_request(STATUS_BAR_HEIGHT);
    status_label.set_xalign(0.0);

    root.add_css_class("j3-root");
    menu_bar.add_css_class("j3-menu-bar");
    top_bar.add_css_class("j3-top-bar");
    command_bar.add_css_class("j3-command-bar");
    status_label.add_css_class("j3-status");

    root.append(&menu_bar);
    top_bar.append(&workspace_toggle_button);
    top_bar.append(&new_tab_button);
    tab_strip.append(&tab_bar);
    tab_strip.append(&overflow_button);
    top_bar.append(&tab_strip);
    root.append(&top_bar);
    root.append(&command_bar);
    root.append(&drawing_area);
    root.append(&status_label);
    window.set_child(Some(&root));

    LinuxWidgets {
        window,
        root,
        menu_bar,
        top_bar,
        workspace_toggle_button,
        new_tab_button,
        tab_strip,
        tab_bar,
        overflow_button,
        command_bar,
        drawing_area,
        status_label,
    }
}

fn install_app_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    let css = APP_CSS
        .replace(
            "@TOP_BAR_LEFT_PADDING@",
            &format!("{TOP_BAR_LEFT_MARGIN}px"),
        )
        .replace(
            "@COMMAND_BAR_LEFT_PADDING@",
            &format!("{COMMAND_BAR_LEFT_MARGIN}px"),
        );
    provider.load_from_data(&css);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn apply_dialog_style(dialog: &gtk::Dialog) {
    dialog.add_css_class("j3-dialog");
    dialog.content_area().add_css_class("j3-dialog-content");
}

fn install_callbacks(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let widgets = owner.borrow().widgets.clone();
    let key_controller = gtk::EventControllerKey::new();
    let menu_bar_for_key = widgets.menu_bar.clone();
    let window_for_key = widgets.window.clone();
    let alt_key_down = Rc::new(Cell::new(false));
    let alt_key_down_for_press = Rc::clone(&alt_key_down);
    key_controller.connect_key_pressed(move |_, key, _, modifiers| {
        if matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R) {
            alt_key_down_for_press.set(true);
        }
        if main_window_close_shortcut_key(key, modifiers, alt_key_down_for_press.get()) {
            window_for_key.close();
            glib::Propagation::Stop
        } else if main_menu_keyboard_entry_key(key, modifiers)
            && focus_first_main_menu_button(&menu_bar_for_key)
        {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    key_controller.connect_key_released(move |_, key, _, _| {
        if matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R) {
            alt_key_down.set(false);
        }
    });
    widgets.window.add_controller(key_controller);

    let owner_for_toggle = Rc::clone(owner);
    widgets.workspace_toggle_button.connect_clicked(move |_| {
        owner_for_toggle
            .borrow_mut()
            .toggle_workspace_ui(&owner_for_toggle)
    });
    let owner_for_toggle_context = Rc::clone(owner);
    let toggle_context = gtk::GestureClick::new();
    toggle_context.set_button(3);
    toggle_context.connect_pressed(move |gesture, _, x, y| {
        if let Some(widget) = gesture.widget() {
            show_options_context_menu(&owner_for_toggle_context, &widget, x, y);
        }
    });
    widgets
        .workspace_toggle_button
        .add_controller(toggle_context);

    let owner_for_new = Rc::clone(owner);
    widgets
        .new_tab_button
        .connect_clicked(move |_| owner_for_new.borrow_mut().add_tab(&owner_for_new));

    let tab_strip_click = gtk::GestureClick::new();
    tab_strip_click.set_button(0);
    let owner_for_tab_strip_click = Rc::clone(owner);
    let tab_bar_for_strip_click = widgets.tab_bar.clone();
    let overflow_for_strip_click = widgets.overflow_button.clone();
    tab_strip_click.connect_pressed(move |gesture, press_count, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let Ok(tab_strip) = widget.downcast::<gtk::Box>() else {
            return;
        };
        if tab_strip_position_hits_non_empty_child(
            &tab_bar_for_strip_click,
            &overflow_for_strip_click,
            x,
            y,
        ) {
            return;
        }

        match gesture.current_button() {
            1 if press_count == 2 => toggle_main_window_maximized(&owner_for_tab_strip_click),
            3 => show_tabbar_context_menu(&owner_for_tab_strip_click, tab_strip.upcast_ref(), x, y),
            _ => {}
        }
    });
    widgets.tab_strip.add_controller(tab_strip_click);

    let tab_strip_drag = gtk::GestureDrag::new();
    tab_strip_drag.set_button(1);
    let owner_for_tab_strip_drag = Rc::clone(owner);
    let owner_for_tab_strip_drag_begin = Rc::clone(&owner_for_tab_strip_drag);
    let tab_bar_for_strip_drag = widgets.tab_bar.clone();
    let overflow_for_strip_drag = widgets.overflow_button.clone();
    tab_strip_drag.connect_drag_begin(move |gesture, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let Ok(_tab_strip) = widget.clone().downcast::<gtk::Box>() else {
            return;
        };
        owner_for_tab_strip_drag_begin
            .borrow_mut()
            .begin_hidden_tabbar_window_move(
                &tab_bar_for_strip_drag,
                &overflow_for_strip_drag,
                x,
                y,
            );
    });
    let owner_for_tab_strip_drag_update = Rc::clone(&owner_for_tab_strip_drag);
    tab_strip_drag.connect_drag_update(move |_, offset_x, offset_y| {
        owner_for_tab_strip_drag_update
            .borrow_mut()
            .update_main_window_move_drag(offset_x, offset_y);
    });
    tab_strip_drag.connect_drag_end(move |_, _, _| {
        owner_for_tab_strip_drag
            .borrow_mut()
            .finish_main_window_move_drag();
    });
    widgets.tab_strip.add_controller(tab_strip_drag);

    let owner_for_draw = Rc::clone(owner);
    widgets
        .drawing_area
        .set_draw_func(move |_, context, width, height| {
            owner_for_draw.borrow().draw(context, width, height);
        });

    let click = gtk::GestureClick::new();
    click.set_button(0);
    let owner_for_press = Rc::clone(owner);
    click.connect_pressed(move |gesture, _, x, y| {
        if gesture.current_button() == 1 {
            owner_for_press.borrow_mut().on_region_press(x, y);
        } else if gesture.current_button() == 3
            && let Some(widget) = gesture.widget()
        {
            owner_for_press
                .borrow_mut()
                .on_region_context(&owner_for_press, &widget, x, y);
        }
    });
    let owner_for_release = Rc::clone(owner);
    click.connect_released(move |_, _, _, _| {
        owner_for_release.borrow_mut().on_region_release();
    });
    widgets.drawing_area.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    let owner_for_motion = Rc::clone(owner);
    motion.connect_motion(move |_, x, y| {
        owner_for_motion.borrow_mut().on_region_motion(x, y);
    });
    widgets.drawing_area.add_controller(motion);

    let owner_for_close = Rc::clone(owner);
    widgets.window.connect_close_request(move |_| {
        let should_close = owner_for_close
            .borrow_mut()
            .shutdown(ShutdownMode::Cancellable);
        if should_close {
            let window = {
                let state = owner_for_close.borrow();
                state.detach_attached_popovers();
                state.widgets.window.clone()
            };
            glib::idle_add_local_once(move || {
                window.destroy();
            });
            glib::Propagation::Stop
        } else {
            owner_for_close.borrow().refresh_status();
            glib::Propagation::Stop
        }
    });

    let owner_for_destroy = Rc::clone(owner);
    widgets.window.connect_destroy(move |_| {
        let _ = owner_for_destroy
            .borrow_mut()
            .shutdown(ShutdownMode::Forced);
        owner_for_destroy.borrow().detach_attached_popovers();
    });

    let owner_for_realize = Rc::clone(owner);
    widgets.window.connect_realize(move |window| {
        let Some(toplevel) = window_toplevel(window) else {
            return;
        };
        let owner_for_state = Rc::clone(&owner_for_realize);
        sync_main_window_toplevel_state_from_gdk(&owner_for_state, &toplevel);
        toplevel.connect_state_notify(move |toplevel| {
            sync_main_window_toplevel_state_from_gdk(&owner_for_state, toplevel);
        });
    });

    let owner_for_maximized = Rc::clone(owner);
    widgets.window.connect_maximized_notify(move |window| {
        let maximized = window.is_maximized();
        owner_for_maximized
            .borrow_mut()
            .set_main_window_maximized(&owner_for_maximized, maximized);
    });
}

fn install_drop_poll(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let owner_for_poll = Rc::clone(owner);
    glib::timeout_add_local(Duration::from_millis(DROP_POLL_INTERVAL_MS), move || {
        let mut state = owner_for_poll.borrow_mut();
        state.poll_external_drop(&owner_for_poll);
        state.refresh_tab_bar_after_resize_if_needed(&owner_for_poll);
        state.sync_active_tab_if_bounds_changed();
        glib::ControlFlow::Continue
    });
}

fn install_splitter_overlay_poll(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let owner_for_poll = Rc::clone(owner);
    glib::timeout_add_local(
        Duration::from_millis(SPLITTER_OVERLAY_POLL_INTERVAL_MS),
        move || {
            owner_for_poll.borrow_mut().poll_splitter_overlay();
            glib::ControlFlow::Continue
        },
    );
}

fn install_preset_restore_poll(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let owner_for_poll = Rc::clone(owner);
    glib::timeout_add_local(
        Duration::from_millis(PRESET_RESTORE_POLL_INTERVAL_MS),
        move || {
            owner_for_poll
                .borrow_mut()
                .poll_tab_preset_restore(&owner_for_poll);
            glib::ControlFlow::Continue
        },
    );
}

fn schedule_active_tab_sync(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let owner_for_sync = Rc::clone(owner);
    glib::idle_add_local_once(move || {
        let mut state = owner_for_sync.borrow_mut();
        if state.shutdown_done || !state.widgets.window.is_visible() {
            return;
        }
        state.sync_active_tab_to_current_bounds();
    });
}

fn window_toplevel(window: &gtk::ApplicationWindow) -> Option<gdk::Toplevel> {
    window
        .surface()
        .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok())
}

fn sync_main_window_toplevel_state_from_gdk(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    toplevel: &gdk::Toplevel,
) {
    let state = toplevel.state();
    let minimized = state.contains(gdk::ToplevelState::MINIMIZED);
    let maximized = state.contains(gdk::ToplevelState::MAXIMIZED);
    owner
        .borrow_mut()
        .sync_main_window_toplevel_state(owner, minimized, maximized);
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        detach_menu_button_popovers(&child);
        container.remove(&child);
    }
}

fn detach_menu_button_popovers(widget: &gtk::Widget) {
    if let Ok(popover) = widget.clone().downcast::<gtk::Popover>() {
        popover.popdown();
        if let Some(child) = popover.child() {
            detach_menu_button_popovers(&child);
        }
        if popover.parent().is_some() {
            popover.unparent();
        }
        return;
    }

    if let Ok(menu_button) = widget.clone().downcast::<gtk::MenuButton>() {
        if let Some(popover) = menu_button.popover() {
            popover.popdown();
            if let Some(child) = popover.child() {
                detach_menu_button_popovers(&child);
            }
        }
        menu_button.set_popover(None::<&gtk::Popover>);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        detach_menu_button_popovers(&current);
    }
}

fn menu_button(label: &str) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder().label(label).build();
    let key_controller = gtk::EventControllerKey::new();
    let button_for_key = button.clone();
    key_controller.connect_key_pressed(move |_, key, _, modifiers| {
        if menu_button_keyboard_popup_key(key, modifiers)
            && !menu_button_popover_is_visible(&button_for_key)
        {
            popdown_menu_button_root_popovers(&button_for_key);
            button_for_key.popup();
            glib::Propagation::Stop
        } else if menu_button_keyboard_popdown_key(key, modifiers) {
            popdown_menu_button_root_popovers(&button_for_key);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    button.add_controller(key_controller);
    button
}

fn main_menu_button(label: &str) -> gtk::MenuButton {
    let button = menu_button(label);
    button.add_css_class("j3-main-menu-button");
    button
}

fn menu_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);
    content
}

fn attach_popover(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    button: &gtk::MenuButton,
    content: gtk::Box,
) {
    let popover = gtk::Popover::new();
    popover.set_child(Some(&content));
    connect_active_tab_sync_after_popover_closed(owner, &popover);
    button.set_popover(Some(&popover));
}

fn main_menu_keyboard_entry_key(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    if matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R) {
        return true;
    }

    key == gdk::Key::F10
        && !modifiers.intersects(
            gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::META_MASK,
        )
}

fn main_window_close_shortcut_key(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    alt_key_down: bool,
) -> bool {
    key == gdk::Key::F4
        && (modifiers.contains(gdk::ModifierType::ALT_MASK) || alt_key_down)
        && !modifiers.intersects(
            gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::META_MASK,
        )
}

fn menu_button_keyboard_popup_key(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    matches!(
        key,
        gdk::Key::Down | gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space
    ) && !modifiers.intersects(
        gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::META_MASK,
    )
}

fn menu_button_keyboard_popdown_key(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    key == gdk::Key::Escape && modifiers.is_empty()
}

fn menu_button_popover_is_visible(button: &gtk::MenuButton) -> bool {
    button.popover().is_some_and(|popover| popover.is_visible())
}

fn popdown_menu_button_root_popovers(button: &gtk::MenuButton) {
    let mut root = button.clone().upcast::<gtk::Widget>();
    while let Some(parent) = root.parent() {
        root = parent;
    }
    popdown_widget_menu_popovers(&root);
}

fn focus_first_main_menu_button(menu_bar: &gtk::Box) -> bool {
    let mut child = menu_bar.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<gtk::MenuButton>() {
            return button.grab_focus();
        }
    }
    false
}

fn connect_active_tab_sync_after_popover_closed(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    popover: &gtk::Popover,
) {
    let owner_for_closed = Rc::clone(owner);
    popover.connect_closed(move |_| {
        schedule_active_tab_sync(&owner_for_closed);
    });
}

fn tab_reorder_indicator() -> gtk::Separator {
    let indicator = gtk::Separator::new(gtk::Orientation::Vertical);
    indicator.set_width_request(3);
    indicator.add_css_class("error");
    indicator
}

fn append_separator(content: &gtk::Box) {
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    content.append(&separator);
}

fn append_menu_action<F>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    label: &str,
    action: F,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    append_menu_action_enabled(owner, content, label, true, action);
}

fn append_menu_action_enabled<F>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    label: &str,
    enabled: bool,
    action: F,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    let button = gtk::Button::with_label(label);
    button.set_halign(gtk::Align::Fill);
    button.set_sensitive(enabled);
    let owner_for_action = Rc::clone(owner);
    let action = Rc::new(action);
    button.connect_clicked(move |button| {
        popdown_ancestor_popovers(button.upcast_ref());
        popdown_owned_menu_popovers(&owner_for_action);
        let owner_for_action = Rc::clone(&owner_for_action);
        let action = Rc::clone(&action);
        glib::idle_add_local_once(move || {
            popdown_owned_menu_popovers(&owner_for_action);
            let owner_for_sync = Rc::clone(&owner_for_action);
            action(owner_for_action);
            schedule_active_tab_sync(&owner_for_sync);
        });
    });
    content.append(&button);
}

fn append_check_menu_action<F>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    label: &str,
    checked: bool,
    action: F,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    let check = gtk::CheckButton::with_label(label);
    check.set_halign(gtk::Align::Fill);
    check.set_active(checked);
    let owner_for_action = Rc::clone(owner);
    let action = Rc::new(action);
    check.connect_toggled(move |check| {
        popdown_ancestor_popovers(check.upcast_ref());
        popdown_owned_menu_popovers(&owner_for_action);
        let owner_for_action = Rc::clone(&owner_for_action);
        let action = Rc::clone(&action);
        glib::idle_add_local_once(move || {
            popdown_owned_menu_popovers(&owner_for_action);
            let owner_for_sync = Rc::clone(&owner_for_action);
            action(owner_for_action);
            schedule_active_tab_sync(&owner_for_sync);
        });
    });
    content.append(&check);
}

fn popdown_ancestor_popovers(widget: &gtk::Widget) {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        current = widget.parent();
        if let Ok(popover) = widget.downcast::<gtk::Popover>() {
            popover.popdown();
        }
    }
}

fn popdown_owned_menu_popovers(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let window = owner.borrow().widgets.window.clone();
    popdown_widget_menu_popovers(window.upcast_ref());
}

fn popdown_widget_menu_popovers(widget: &gtk::Widget) {
    if let Ok(menu_button) = widget.clone().downcast::<gtk::MenuButton>()
        && let Some(popover) = menu_button.popover()
    {
        popover.popdown();
        if let Some(child) = popover.child() {
            popdown_widget_menu_popovers(&child);
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        popdown_widget_menu_popovers(&current);
    }
}

fn append_menu_label(content: &gtk::Box, label: &str) {
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_sensitive(false);
    content.append(&label);
}

fn append_language_menu_button(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    language: UiLanguage,
) {
    let button = menu_button("Language");
    let submenu = menu_content();
    append_check_menu_action(
        owner,
        &submenu,
        "English",
        language == UiLanguage::English,
        |state| {
            state
                .borrow_mut()
                .set_ui_language(&state, UiLanguage::English)
        },
    );
    append_check_menu_action(
        owner,
        &submenu,
        "Korean",
        language == UiLanguage::Korean,
        |state| {
            state
                .borrow_mut()
                .set_ui_language(&state, UiLanguage::Korean)
        },
    );
    attach_popover(owner, &button, submenu);
    content.append(&button);
}

fn append_preset_action_menu_button(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    label: &str,
    action: PresetAction,
    target: PresetTarget,
    language: UiLanguage,
    presets: &[String],
) {
    let button = menu_button(label);
    let submenu = menu_content();
    if presets.is_empty() {
        append_menu_label(
            &submenu,
            t(
                language,
                "(No saved tab presets)",
                "(저장된 탭 preset 없음)",
            ),
        );
    } else {
        for name in presets {
            let item_label = name.clone();
            let preset_name = name.clone();
            append_menu_action(owner, &submenu, &item_label, move |state| {
                LinuxMainWindow::run_tab_preset_action(&state, action, target, preset_name.clone())
            });
        }
    }
    attach_popover(owner, &button, submenu);
    content.append(&button);
}

fn show_context_preset_action_menu(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    action: PresetAction,
    target: PresetTarget,
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let (language, presets) = {
        let state = owner.borrow();
        (state.language(), state.tab_preset_names())
    };

    if action == PresetAction::Load && target.tab_id(owner).is_none() {
        let mut state = owner.borrow_mut();
        state.status = tab_preset_load_missing_tab_status_text(language).to_owned();
        state.refresh_status();
        return;
    }

    if presets.is_empty() {
        let mut state = owner.borrow_mut();
        state.status = tab_preset_empty_status_text(language, action).to_owned();
        state.refresh_status();
        return;
    }

    let content = menu_content();
    for name in presets {
        let item_label = name.clone();
        append_menu_action(owner, &content, &item_label, move |state| {
            LinuxMainWindow::run_tab_preset_action(&state, action, target, name.clone())
        });
    }
    popup_context_popover(owner, anchor, content, x, y);
}

fn tab_context_preset_anchor(widget: &gtk::Widget) -> gtk::Widget {
    widget.parent().unwrap_or_else(|| widget.clone())
}

fn append_command_button<F>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    content: &gtk::Box,
    label: &str,
    width: i32,
    action: F,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    let button = gtk::Button::with_label(label);
    button.add_css_class("j3-command-button");
    button.set_width_request(width);
    let owner_for_action = Rc::clone(owner);
    let action = Rc::new(action);
    button.connect_clicked(move |_| {
        let owner_for_action = Rc::clone(&owner_for_action);
        let action = Rc::clone(&action);
        glib::idle_add_local_once(move || {
            let owner_for_sync = Rc::clone(&owner_for_action);
            action(owner_for_action);
            schedule_active_tab_sync(&owner_for_sync);
        });
    });
    content.append(&button);
}

fn show_tab_context_menu(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    tab_id: TabId,
    widget: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let language = {
        let state = owner.borrow();
        state.language()
    };
    let content = menu_content();
    append_menu_action(
        owner,
        &content,
        t(language, "Rename tab", "탭 이름 변경"),
        move |state| LinuxMainWindow::rename_tab(&state, tab_id),
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Close tab", "탭 닫기"),
        move |state| state.borrow_mut().delete_tab(tab_id, &state),
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Close other tabs", "다른 탭 닫기"),
        move |state| state.borrow_mut().close_other_tabs_for(&state, tab_id),
    );
    append_separator(&content);
    append_menu_action(
        owner,
        &content,
        t(language, "Save tab preset...", "탭 preset 저장..."),
        move |state| LinuxMainWindow::save_tab_preset_for_tab(&state, tab_id),
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Load tab preset...", "탭 preset 불러오기..."),
        {
            let anchor = tab_context_preset_anchor(widget);
            move |state| {
                let y = f64::from(anchor.allocated_height().max(TAB_BAR_HEIGHT));
                show_context_preset_action_menu(
                    &state,
                    PresetAction::Load,
                    PresetTarget::Fixed(tab_id),
                    &anchor,
                    0.0,
                    y,
                )
            }
        },
    );
    popup_context_popover(owner, widget, content, x, y);
}

fn show_tabbar_context_menu(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    widget: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let (language, active_tab, is_maximized) = {
        let state = owner.borrow();
        (
            state.language(),
            state.app.active_tab_id(),
            state.main_window_maximized,
        )
    };
    let content = menu_content();
    append_menu_action(owner, &content, t(language, "New tab", "새 탭"), |state| {
        state.borrow_mut().add_tab(&state)
    });
    if let Some(active_tab) = active_tab {
        append_menu_action(
            owner,
            &content,
            t(language, "Rename active tab", "활성 탭 이름 변경"),
            move |state| LinuxMainWindow::rename_tab(&state, active_tab),
        );
        append_menu_action(
            owner,
            &content,
            t(language, "Close active tab", "활성 탭 닫기"),
            move |state| state.borrow_mut().delete_tab(active_tab, &state),
        );
        append_menu_action(
            owner,
            &content,
            t(language, "Close other tabs", "다른 탭 닫기"),
            move |state| state.borrow_mut().close_other_tabs_for(&state, active_tab),
        );
        append_separator(&content);
        append_menu_action(
            owner,
            &content,
            t(
                language,
                "Save active tab preset...",
                "활성 탭 preset 저장...",
            ),
            move |state| LinuxMainWindow::save_tab_preset_for_tab(&state, active_tab),
        );
        append_menu_action(
            owner,
            &content,
            t(
                language,
                "Load active tab preset...",
                "활성 탭 preset 불러오기...",
            ),
            move |state| {
                let anchor = {
                    let state = state.borrow();
                    state.widgets.tab_bar.clone().upcast::<gtk::Widget>()
                };
                show_context_preset_action_menu(
                    &state,
                    PresetAction::Load,
                    PresetTarget::Fixed(active_tab),
                    &anchor,
                    0.0,
                    f64::from(TAB_BAR_HEIGHT),
                )
            },
        );
        append_menu_action(
            owner,
            &content,
            t(language, "Edit tab preset...", "탭 preset 편집..."),
            |state| {
                let anchor = {
                    let state = state.borrow();
                    state.widgets.tab_bar.clone().upcast::<gtk::Widget>()
                };
                show_context_preset_action_menu(
                    &state,
                    PresetAction::Edit,
                    PresetTarget::Active,
                    &anchor,
                    0.0,
                    f64::from(TAB_BAR_HEIGHT),
                )
            },
        );
        append_menu_action(
            owner,
            &content,
            t(language, "Delete tab preset...", "탭 preset 삭제..."),
            |state| {
                let anchor = {
                    let state = state.borrow();
                    state.widgets.tab_bar.clone().upcast::<gtk::Widget>()
                };
                show_context_preset_action_menu(
                    &state,
                    PresetAction::Delete,
                    PresetTarget::Active,
                    &anchor,
                    0.0,
                    f64::from(TAB_BAR_HEIGHT),
                )
            },
        );
    }
    append_separator(&content);
    append_menu_action(
        owner,
        &content,
        t(language, "Minimize window", "창 최소화"),
        |state| minimize_main_window(&state),
    );
    append_menu_action(
        owner,
        &content,
        main_window_maximize_restore_label(language, is_maximized),
        |state| toggle_main_window_maximized(&state),
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Close window", "창 닫기"),
        |state| {
            let window = state.borrow().widgets.window.clone();
            window.close();
        },
    );

    popup_context_popover(owner, widget, content, x, y);
}

fn show_region_context_menu(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    widget: &gtk::Widget,
    x: f64,
    y: f64,
    language: UiLanguage,
) {
    let content = menu_content();
    append_menu_action(
        owner,
        &content,
        t(language, "Split vertical", "세로 분할"),
        |state| {
            state
                .borrow_mut()
                .split_active_region(&state, SplitDirection::Vertical)
        },
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Split horizontal", "가로 분할"),
        |state| {
            state
                .borrow_mut()
                .split_active_region(&state, SplitDirection::Horizontal)
        },
    );
    append_menu_action(
        owner,
        &content,
        t(language, "Delete region", "영역 삭제"),
        |state| state.borrow_mut().delete_active_region(&state),
    );
    append_menu_action(owner, &content, t(language, "Undock", "해제"), |state| {
        state.borrow_mut().undock_active_region(&state)
    });

    popup_context_popover(owner, widget, content, x, y);
}

fn show_options_context_menu(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    widget: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let content = menu_content();
    owner
        .borrow()
        .append_options_menu_items(owner, &content, true);
    popup_context_popover(owner, widget, content, x, y);
}

fn popup_context_popover(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    widget: &gtk::Widget,
    content: gtk::Box,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_child(Some(&content));
    let owner_for_closed = Rc::clone(owner);
    popover.connect_closed(move |popover| {
        schedule_active_tab_sync(&owner_for_closed);
        if popover.parent().is_some() {
            popover.unparent();
        }
    });
    popover.set_parent(widget);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.popup();
}

fn minimize_main_window(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let window = {
        let mut state = owner.borrow_mut();
        state.on_main_window_minimized();
        state.widgets.window.clone()
    };
    window.minimize();
}

fn toggle_main_window_maximized(owner: &Rc<RefCell<LinuxMainWindow>>) {
    let (window, maximize) = {
        let mut state = owner.borrow_mut();
        let maximize = !state.main_window_maximized;
        state.main_window_maximized = maximize;
        (state.widgets.window.clone(), maximize)
    };
    if maximize {
        window.maximize();
    } else {
        window.unmaximize();
    }
    let owner_for_refresh = Rc::clone(owner);
    glib::idle_add_local_once(move || {
        owner_for_refresh.borrow().refresh_menus(&owner_for_refresh);
    });
}

fn main_window_maximize_restore_label(language: UiLanguage, maximized: bool) -> &'static str {
    match (language, maximized) {
        (UiLanguage::English, true) => "Restore window",
        (UiLanguage::English, false) => "Maximize window",
        (UiLanguage::Korean, true) => "창 복원",
        (UiLanguage::Korean, false) => "창 최대화",
    }
}

fn tab_bar_position_hits_child(tab_bar: &gtk::Box, x: f64, y: f64) -> bool {
    let x = x.round() as i32;
    let y = y.round() as i32;
    let mut child = tab_bar.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        let allocation = widget.allocation();
        let left = allocation.x();
        let top = allocation.y();
        let right = left.saturating_add(allocation.width());
        let bottom = top.saturating_add(allocation.height());
        if x >= left && x < right && y >= top && y < bottom {
            return true;
        }
        child = next;
    }
    false
}

fn tab_strip_position_hits_non_empty_child(
    tab_bar: &gtk::Box,
    overflow_button: &gtk::MenuButton,
    x: f64,
    y: f64,
) -> bool {
    if overflow_button.is_visible()
        && widget_allocation_contains(overflow_button.upcast_ref(), x, y)
    {
        return true;
    }

    let allocation = tab_bar.allocation();
    let tab_bar_x = x.round() as i32 - allocation.x();
    let tab_bar_y = y.round() as i32 - allocation.y();
    if tab_bar_x < 0
        || tab_bar_y < 0
        || tab_bar_x >= allocation.width()
        || tab_bar_y >= allocation.height()
    {
        return false;
    }

    tab_bar_position_hits_child(tab_bar, f64::from(tab_bar_x), f64::from(tab_bar_y))
}

fn widget_allocation_contains(widget: &gtk::Widget, x: f64, y: f64) -> bool {
    let allocation = widget.allocation();
    let x = x.round() as i32;
    let y = y.round() as i32;
    x >= allocation.x()
        && y >= allocation.y()
        && x < allocation.x().saturating_add(allocation.width())
        && y < allocation.y().saturating_add(allocation.height())
}

fn prompt_text<F, C>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    title: &str,
    prompt_label: &str,
    initial: &str,
    on_accept: F,
    on_cancel: C,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>, String) + 'static,
    C: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    let dialog = gtk::Dialog::builder()
        .transient_for(&owner.borrow().widgets.window)
        .modal(true)
        .title(title)
        .build();
    apply_dialog_style(&dialog);
    dialog.add_button(
        t(owner.borrow().language(), "Cancel", "취소"),
        gtk::ResponseType::Cancel,
    );
    dialog.add_button(
        t(owner.borrow().language(), "OK", "확인"),
        gtk::ResponseType::Accept,
    );
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);
    let label = gtk::Label::new(Some(prompt_label));
    label.set_halign(gtk::Align::Start);
    dialog.content_area().append(&label);
    dialog.content_area().append(&entry);
    dialog.set_default_response(gtk::ResponseType::Accept);
    let entry_for_focus = entry.clone();
    let owner_for_response = Rc::clone(owner);
    let handled = Rc::new(Cell::new(false));
    let handled_for_response = Rc::clone(&handled);
    dialog.connect_response(move |dialog, response| {
        if handled_for_response.replace(true) {
            return;
        }
        if response == gtk::ResponseType::Accept {
            on_accept(Rc::clone(&owner_for_response), entry.text().to_string());
        } else {
            on_cancel(Rc::clone(&owner_for_response));
        }
        dialog.close();
        schedule_active_tab_sync(&owner_for_response);
    });
    dialog.present();
    entry_for_focus.grab_focus();
}

fn prompt_preset_edit<F, C>(
    owner: &Rc<RefCell<LinuxMainWindow>>,
    preset: TabPreset,
    on_accept: F,
    on_cancel: C,
) where
    F: Fn(Rc<RefCell<LinuxMainWindow>>, String, TabPreset) + 'static,
    C: Fn(Rc<RefCell<LinuxMainWindow>>) + 'static,
{
    let original_name = preset.name().to_owned();
    let language = owner.borrow().language();
    let dialog = gtk::Dialog::builder()
        .transient_for(&owner.borrow().widgets.window)
        .modal(true)
        .title(tab_preset_edit_window_title_text(language))
        .default_width(640)
        .default_height(520)
        .build();
    apply_dialog_style(&dialog);
    dialog.add_button(t(language, "Cancel", "취소"), gtk::ResponseType::Cancel);
    dialog.add_button(t(language, "OK", "확인"), gtk::ResponseType::Accept);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let name_entry = gtk::Entry::new();
    name_entry.set_text(preset.name());
    name_entry.set_activates_default(true);
    let instruction = gtk::Label::new(Some(&tab_preset_edit_instruction_text(
        language,
        preset.program_specs().len(),
    )));
    instruction.set_xalign(0.0);
    instruction.set_wrap(true);
    content.append(&instruction);
    content.append(&gtk::Label::new(Some(tab_preset_name_label_text(language))));
    content.append(&name_entry);

    let mut program_rows = Vec::new();
    let program_specs = preset.program_specs();
    if !program_specs.is_empty() {
        let grid = gtk::Grid::new();
        grid.set_row_spacing(6);
        grid.set_column_spacing(6);
        grid.attach(
            &gtk::Label::new(Some(tab_preset_executable_label_text(language))),
            1,
            0,
            1,
            1,
        );
        grid.attach(
            &gtk::Label::new(Some(tab_preset_arguments_label_text(language))),
            2,
            0,
            1,
            1,
        );
        for (index, program) in program_specs.iter().enumerate() {
            let path_entry = gtk::Entry::new();
            path_entry.set_text(program.executable_path());
            path_entry.set_activates_default(true);
            let args_entry = gtk::Entry::new();
            args_entry.set_text(&format_program_arguments(program.arguments()));
            args_entry.set_activates_default(true);
            let row = i32::try_from(index.saturating_add(1)).unwrap_or(1);
            grid.attach(
                &gtk::Label::new(Some(&tab_preset_program_row_header_text(
                    language,
                    index + 1,
                    program_specs.len(),
                    program,
                ))),
                0,
                row,
                1,
                1,
            );
            grid.attach(&path_entry, 1, row, 1, 1);
            grid.attach(&args_entry, 2, row, 1, 1);
            program_rows.push((path_entry, args_entry, program.title().map(str::to_owned)));
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .min_content_height(240)
            .child(&grid)
            .build();
        content.append(&scrolled);
    }
    dialog.content_area().append(&content);
    dialog.set_default_response(gtk::ResponseType::Accept);
    let name_entry_for_focus = name_entry.clone();

    let owner_for_response = Rc::clone(owner);
    let handled = Rc::new(Cell::new(false));
    let handled_for_response = Rc::clone(&handled);
    dialog.connect_response(move |dialog, response| {
        if handled_for_response.get() {
            return;
        }
        if response == gtk::ResponseType::Accept {
            let mut edited = preset.clone();
            if let Err(error) = edited.rename(name_entry.text().to_string()) {
                show_preset_edit_validation_dialog(
                    dialog,
                    language,
                    &tab_preset_name_validation_message(language, error),
                    name_entry.upcast_ref(),
                );
                return;
            }

            let mut programs = Vec::with_capacity(program_rows.len());
            for (index, (path, args, title)) in program_rows.iter().enumerate() {
                let arguments = match parse_program_arguments(&args.text()) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        show_preset_edit_validation_dialog(
                            dialog,
                            language,
                            &program_edit_dialog_validation_message(
                                language,
                                index + 1,
                                error.user_message(language),
                            ),
                            args.upcast_ref(),
                        );
                        return;
                    }
                };
                match ExternalProgramSpec::new_with_arguments(
                    path.text().to_string(),
                    arguments,
                    title.clone(),
                ) {
                    Ok(program) => programs.push(program),
                    Err(error) => {
                        let focus = if matches!(error, DomainError::InvalidProgramArgument) {
                            args
                        } else {
                            path
                        };
                        show_preset_edit_validation_dialog(
                            dialog,
                            language,
                            &program_edit_dialog_validation_message(
                                language,
                                index + 1,
                                linux_domain_error_message(language, &error),
                            ),
                            focus.upcast_ref(),
                        );
                        return;
                    }
                }
            }

            edited.replace_program_specs(programs);
            handled_for_response.set(true);
            on_accept(
                Rc::clone(&owner_for_response),
                original_name.clone(),
                edited,
            );
        } else {
            handled_for_response.set(true);
            on_cancel(Rc::clone(&owner_for_response));
        }
        dialog.close();
        schedule_active_tab_sync(&owner_for_response);
    });
    dialog.present();
    name_entry_for_focus.grab_focus();
}

fn show_preset_edit_validation_dialog(
    parent: &gtk::Dialog,
    language: UiLanguage,
    message: &str,
    focus: &gtk::Widget,
) {
    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title(tab_preset_edit_message_title_text(language))
        .build();
    apply_dialog_style(&dialog);
    dialog.add_button(t(language, "OK", "확인"), gtk::ResponseType::Ok);
    dialog.set_default_response(gtk::ResponseType::Ok);
    let label = gtk::Label::new(Some(message));
    label.set_xalign(0.0);
    label.set_wrap(true);
    dialog.content_area().append(&label);
    let focus = focus.clone();
    let handled = Rc::new(Cell::new(false));
    let handled_for_response = Rc::clone(&handled);
    dialog.connect_response(move |dialog, _| {
        if handled_for_response.replace(true) {
            return;
        }
        dialog.close();
        focus.grab_focus();
    });
    dialog.present();
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabPresetProgramFailure {
    label: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTabPresetProgramRestoreReport {
    expected: usize,
    docked: usize,
    failures: Vec<LinuxTabPresetProgramFailure>,
}

impl LinuxTabPresetProgramRestoreReport {
    fn new(expected: usize) -> Self {
        Self {
            expected,
            docked: 0,
            failures: Vec::new(),
        }
    }
}

struct LinuxTabPresetRestoreState {
    preset_name: String,
    target: LinuxTabStatusLabel,
    target_tab_id: TabId,
    undocked: usize,
    language: UiLanguage,
    report: LinuxTabPresetProgramRestoreReport,
    pending: Vec<PendingLinuxTabPresetProgram>,
    deadline: Instant,
}

impl LinuxTabPresetRestoreState {
    fn new(
        preset_name: String,
        target: LinuxTabStatusLabel,
        target_tab_id: TabId,
        undocked: usize,
        expected: usize,
        language: UiLanguage,
        now: Instant,
    ) -> Self {
        Self {
            preset_name,
            target,
            target_tab_id,
            undocked,
            language,
            report: LinuxTabPresetProgramRestoreReport::new(expected),
            pending: Vec::new(),
            deadline: now + PRESET_RESTORE_TIMEOUT,
        }
    }

    const fn target_tab_id(&self) -> TabId {
        self.target_tab_id
    }

    fn push_pending(&mut self, pending: PendingLinuxTabPresetProgram) {
        self.pending.push(pending);
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn tracked_process_ids(&self) -> Vec<u32> {
        self.pending
            .iter()
            .flat_map(|pending| pending.process_ids())
            .collect()
    }

    fn matching_hwnd(
        &self,
        index: usize,
        matches: &HashMap<u32, WindowHandle>,
    ) -> Option<WindowHandle> {
        self.pending
            .get(index)
            .and_then(|pending| pending.matching_hwnd(matches))
    }

    fn remove_pending(&mut self, index: usize) -> PendingLinuxTabPresetProgram {
        self.pending.remove(index)
    }

    fn record_docked(&mut self) {
        self.report.docked = self.report.docked.saturating_add(1);
    }

    fn record_failure(&mut self, failure: LinuxTabPresetProgramFailure) {
        self.report.failures.push(failure);
    }

    fn observe_child_statuses(&mut self) {
        for pending in &mut self.pending {
            pending.observe_child_status();
        }
    }

    fn refresh_tracked_processes(&mut self) {
        let Some(process_tree) = linux_process_tree_child_index() else {
            return;
        };
        let mut discovered = Vec::new();
        let mut stack = Vec::new();
        for pending in &mut self.pending {
            process_tree.append_new_descendants(
                &mut pending.tracked_process_ids,
                &mut discovered,
                &mut stack,
            );
        }
    }

    fn fail_remaining_with_message(&mut self, message: String) -> Vec<Child> {
        let pending = self.pending.drain(..).collect::<Vec<_>>();
        let mut children = Vec::new();
        for pending in pending {
            let released = pending.release();
            if let Some(child) = released.child {
                children.push(child);
            }
            self.record_failure(LinuxTabPresetProgramFailure {
                label: released.label,
                message: message.clone(),
            });
        }
        children
    }

    fn fail_timed_out(&mut self, now: Instant) -> Vec<Child> {
        if now < self.deadline {
            return Vec::new();
        }

        let pending = self.pending.drain(..).collect::<Vec<_>>();
        let mut children = Vec::new();
        for pending in pending {
            let released = pending.release();
            let message = match self.language {
                UiLanguage::English => format!(
                    "{} started as process {}, but no top-level X11 window was found",
                    released.path, released.process_id
                ),
                UiLanguage::Korean => format!(
                    "{}을(를) process {}로 시작했지만 top-level X11 window를 찾지 못했습니다.",
                    released.path, released.process_id
                ),
            };
            if let Some(child) = released.child {
                children.push(child);
            }
            self.record_failure(LinuxTabPresetProgramFailure {
                label: released.label,
                message,
            });
        }
        children
    }

    fn status_text(&self) -> String {
        tab_preset_apply_success_status_text_for_preset(
            self.language,
            &self.preset_name,
            &self.target,
            self.undocked,
            &self.report,
        )
    }

    fn finished_status_text(&self) -> String {
        tab_preset_apply_success_status_text_for_preset(
            self.language,
            &self.preset_name,
            &self.target,
            self.undocked,
            &self.report,
        )
    }
}

struct PendingLinuxTabPresetProgram {
    label: String,
    region_id: RegionId,
    path: String,
    process_id: u32,
    tracked_process_ids: HashSet<u32>,
    child: Child,
}

struct ReleasedLinuxTabPresetProgram {
    label: String,
    region_id: RegionId,
    path: String,
    process_id: u32,
    child: Option<Child>,
}

impl PendingLinuxTabPresetProgram {
    fn start(
        placement: &TabPresetProgramPlacement,
    ) -> Result<Self, LinuxTabPresetProgramLaunchError> {
        let program = placement.program();
        let path = program.executable_path().to_owned();
        let label = preset_program_label(program);
        let child = Command::new(program.executable_path_os())
            .args(program.arguments())
            .spawn()
            .map_err(|source| LinuxTabPresetProgramLaunchError::Spawn {
                label: label.clone(),
                path: path.clone(),
                source,
            })?;
        let process_id = child.id();
        Ok(Self {
            label,
            region_id: placement.region_id(),
            path,
            process_id,
            tracked_process_ids: HashSet::from([process_id]),
            child,
        })
    }

    fn process_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.tracked_process_ids.iter().copied()
    }

    fn matching_hwnd(&self, matches: &HashMap<u32, WindowHandle>) -> Option<WindowHandle> {
        self.process_ids()
            .find_map(|process_id| matches.get(&process_id).copied())
    }

    fn observe_child_status(&mut self) {
        let _ = self.child.try_wait();
    }

    fn release(mut self) -> ReleasedLinuxTabPresetProgram {
        let child = match self.child.try_wait() {
            Ok(Some(_)) => None,
            Ok(None) => Some(self.child),
            Err(error) => {
                eprintln!(
                    "failed to observe preset child process {} before release: {error}",
                    self.process_id
                );
                None
            }
        };
        ReleasedLinuxTabPresetProgram {
            label: self.label,
            region_id: self.region_id,
            path: self.path,
            process_id: self.process_id,
            child,
        }
    }
}

#[derive(Debug)]
enum LinuxTabPresetProgramLaunchError {
    Spawn {
        label: String,
        path: String,
        source: std::io::Error,
    },
}

impl LinuxTabPresetProgramLaunchError {
    fn into_failure(self, language: UiLanguage) -> LinuxTabPresetProgramFailure {
        match self {
            Self::Spawn {
                label,
                path,
                source,
            } => {
                let message = if language == UiLanguage::English {
                    format!("{path} could not be started: {source}")
                } else {
                    format!("{path} 실행 실패: {source}")
                };
                LinuxTabPresetProgramFailure { label, message }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxProcessTreeEntry {
    process_id: u32,
    parent_process_id: u32,
}

#[derive(Debug, Default)]
struct LinuxProcessTreeChildIndex {
    children_by_parent: HashMap<u32, Vec<u32>>,
}

impl LinuxProcessTreeChildIndex {
    fn insert(&mut self, entry: LinuxProcessTreeEntry) {
        if entry.process_id == 0 {
            return;
        }
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
        discovered.clear();
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
}

fn linux_process_tree_child_index() -> Option<LinuxProcessTreeChildIndex> {
    let mut index = LinuxProcessTreeChildIndex::default();
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(process_id) = name.parse::<u32>() else {
            continue;
        };
        let Some(process_entry) = linux_process_tree_entry(process_id) else {
            continue;
        };
        index.insert(process_entry);
    }
    Some(index)
}

fn linux_process_tree_entry(process_id: u32) -> Option<LinuxProcessTreeEntry> {
    let stat = fs::read_to_string(format!("/proc/{process_id}/stat")).ok()?;
    linux_process_tree_entry_from_stat(process_id, &stat)
}

fn linux_process_tree_entry_from_stat(
    process_id: u32,
    stat: &str,
) -> Option<LinuxProcessTreeEntry> {
    let close = stat.rfind(')')?;
    let mut fields = stat.get(close + 1..)?.split_whitespace();
    let _state = fields.next()?;
    let parent_process_id = fields.next()?.parse::<u32>().ok()?;
    Some(LinuxProcessTreeEntry {
        process_id,
        parent_process_id,
    })
}

fn preset_program_label(program: &ExternalProgramSpec) -> String {
    program
        .title()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| program.executable_path())
        .to_owned()
}

fn draw_region(
    context: &cairo::Context,
    language: UiLanguage,
    region_id: RegionId,
    rect: Rect,
    active_region: Option<RegionId>,
    occupied: bool,
) {
    if active_region == Some(region_id) {
        set_source_rgb(context, 0xFF, 0xF2, 0xE6);
    } else if occupied {
        set_source_rgb(context, 0xEC, 0xF7, 0xEC);
    } else {
        set_source_rgb(context, 0xFF, 0xFF, 0xFF);
    }
    context.rectangle(
        f64::from(rect.left()),
        f64::from(rect.top()),
        f64::from(rect.width()),
        f64::from(rect.height()),
    );
    let _ = context.fill_preserve();
    set_source_rgb(context, 0x60, 0x60, 0x60);
    context.set_line_width(1.0);
    let _ = context.stroke();
    let title = region_title_text(language, region_id, occupied);
    let title_rect = region_title_text_rect(rect);
    set_source_rgb(context, 0x20, 0x20, 0x20);
    draw_left_vcenter_text(context, title_rect, &title);
}

fn region_title_text(language: UiLanguage, region_id: RegionId, occupied: bool) -> String {
    match (language, occupied) {
        (UiLanguage::English, true) => format!("Region {} - docked", region_id.value()),
        (UiLanguage::English, false) => format!("Region {}", region_id.value()),
        (UiLanguage::Korean, true) => format!("영역 {} - 배치됨", region_id.value()),
        (UiLanguage::Korean, false) => format!("영역 {}", region_id.value()),
    }
}

fn region_title_text_rect(rect: Rect) -> CanvasTextRect {
    let horizontal_inset = REGION_TITLE_HORIZONTAL_INSET;
    let vertical_inset = REGION_TITLE_VERTICAL_INSET;
    CanvasTextRect {
        left: rect.left() + horizontal_inset,
        top: rect.top() + vertical_inset,
        width: (rect.width() - horizontal_inset.saturating_mul(2)).max(1),
        height: (rect.height() - vertical_inset.saturating_mul(2)).max(1),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CanvasTextRect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

fn draw_left_vcenter_text(context: &cairo::Context, rect: CanvasTextRect, text: &str) {
    let layout = pango_layout(context, text);
    layout.set_width(rect.width.saturating_mul(pango::SCALE));
    layout.set_ellipsize(pango::EllipsizeMode::End);
    let (_, text_height) = layout.pixel_size();
    let top = rect.top + (rect.height - text_height).max(0) / 2;
    context.move_to(f64::from(rect.left), f64::from(top));
    pangocairo::functions::show_layout(context, &layout);
}

fn draw_centered_text(context: &cairo::Context, width: i32, height: i32, text: &str) {
    set_source_rgb(context, 0x20, 0x20, 0x20);
    let layout = pango_layout(context, text);
    layout.set_width(width.max(1).saturating_mul(pango::SCALE));
    layout.set_alignment(pango::Alignment::Center);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    let (_, text_height) = layout.pixel_size();
    let top = (height - text_height).max(0) / 2;
    context.move_to(0.0, f64::from(top));
    pangocairo::functions::show_layout(context, &layout);
}

fn pango_layout(context: &cairo::Context, text: &str) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(context);
    layout.set_text(text);
    layout
}

fn set_source_rgb(context: &cairo::Context, r: u8, g: u8, b: u8) {
    context.set_source_rgb(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
}

fn tab_tooltip_text(titles: impl IntoIterator<Item = String>) -> Option<String> {
    let titles = titles
        .into_iter()
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();
    (!titles.is_empty()).then(|| titles.join("\n"))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramArgumentsParseError {
    UnterminatedQuote,
}

impl ProgramArgumentsParseError {
    fn user_message(self, language: UiLanguage) -> &'static str {
        match self {
            Self::UnterminatedQuote => t(
                language,
                "program arguments contain an unterminated quote",
                "프로그램 arguments에 닫히지 않은 큰따옴표가 있습니다",
            ),
        }
    }
}

fn parse_program_arguments(value: &str) -> Result<Vec<String>, ProgramArgumentsParseError> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut started = false;

    let mut chars = value.chars().peekable();
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

fn next_tab_number(next_tab_id: u64) -> u32 {
    u32::try_from(next_tab_id).unwrap_or(u32::MAX)
}

fn next_tab_preset_name(existing_count: usize) -> String {
    let next = existing_count.saturating_add(1);
    format!("Tab Preset {next}")
}

fn tab_overflow_menu_label(tab_index: usize, tab_name: &str) -> String {
    format!("{} {}", tab_index.saturating_add(1), tab_name)
}

fn workspace_ui_toggle_button_label(language: UiLanguage, visible: bool) -> &'static str {
    match (language, visible) {
        (UiLanguage::English, true) => "Hide",
        (UiLanguage::English, false) => "Show",
        (UiLanguage::Korean, true) => "숨기기",
        (UiLanguage::Korean, false) => "표시",
    }
}

fn new_tab_button_label(language: UiLanguage) -> &'static str {
    t(language, "New", "새 탭")
}

fn workspace_ui_toggle_menu_label(language: UiLanguage, visible: bool) -> &'static str {
    match (language, visible) {
        (UiLanguage::English, true) => "Hide Workspace Controls",
        (UiLanguage::English, false) => "Show Workspace Controls",
        (UiLanguage::Korean, true) => "작업 영역 컨트롤 숨기기",
        (UiLanguage::Korean, false) => "작업 영역 컨트롤 표시",
    }
}

fn about_dialog_title_text(_language: UiLanguage) -> String {
    about_window_title_text()
}

#[cfg(test)]
fn about_dialog_text(language: UiLanguage) -> String {
    about_notice_text(language)
}

fn tab_preset_edit_window_title_text(language: UiLanguage) -> &'static str {
    t(language, "Edit tab preset", "탭 프리셋 편집")
}

fn tab_preset_edit_message_title_text(language: UiLanguage) -> &'static str {
    t(language, "Tab preset edit", "탭 프리셋 편집")
}

fn tab_preset_edit_instruction_text(language: UiLanguage, program_count: usize) -> String {
    if program_count == 0 {
        return t(
            language,
            "Edit the preset name.",
            "프리셋 이름을 편집합니다.",
        )
        .to_owned();
    }

    match language {
        UiLanguage::English => {
            format!("Edit executable paths and arguments for {program_count} docked program(s).")
        }
        UiLanguage::Korean => {
            format!("도킹된 프로그램 {program_count}개의 실행 파일과 인수를 편집합니다.")
        }
    }
}

fn tab_preset_name_label_text(language: UiLanguage) -> &'static str {
    t(language, "Preset name", "프리셋 이름")
}

fn tab_preset_executable_label_text(language: UiLanguage) -> &'static str {
    t(language, "Executable path", "실행 파일 경로")
}

fn tab_preset_arguments_label_text(language: UiLanguage) -> &'static str {
    t(language, "Arguments", "실행 인수")
}

fn tab_preset_program_row_header_text(
    language: UiLanguage,
    index: usize,
    total: usize,
    program: &ExternalProgramSpec,
) -> String {
    let label = preset_program_label(program);
    match language {
        UiLanguage::English => format!("Program {index}/{total}: {label}"),
        UiLanguage::Korean => format!("프로그램 {index}/{total}: {label}"),
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
    message: &str,
) -> String {
    match language {
        UiLanguage::English => format!("Program {program_number}: {message}"),
        UiLanguage::Korean => format!("프로그램 {program_number}: {message}"),
    }
}

fn t(language: UiLanguage, english: &'static str, korean: &'static str) -> &'static str {
    match language {
        UiLanguage::English => english,
        UiLanguage::Korean => korean,
    }
}

fn linux_app_error_message(language: UiLanguage, error: &AppError) -> String {
    match error {
        AppError::Domain(error) => linux_domain_error_message(language, error).to_owned(),
        AppError::Window(error) => linux_window_error_message(language, error),
    }
}

fn startup_saved_workspace_skipped_status_text(
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

fn settings_load_failure_status_text(language: UiLanguage, error: &SettingsFileError) -> String {
    match language {
        UiLanguage::English => format!(
            "{} Starting with a new workspace.",
            linux_settings_error_message(language, error)
        ),
        UiLanguage::Korean => format!("{} 새 워크스페이스로 시작합니다.", error.user_message()),
    }
}

fn shutdown_settings_save_error_message(
    language: UiLanguage,
    error: &ShutdownSettingsSaveError,
) -> String {
    match error {
        ShutdownSettingsSaveError::App(error) => linux_app_error_message(language, error),
        ShutdownSettingsSaveError::Settings(error) => {
            linux_settings_error_message(language, error).to_owned()
        }
    }
}

fn shutdown_undock_summary_text(language: UiLanguage, report: &ShutdownReport) -> String {
    let _ = language;
    format!(
        "Undock: attempted {}, restored {}, missing {}, failures {}",
        report.attempted(),
        report.restored(),
        report.missing(),
        report.failures().len()
    )
}

fn log_app_error(error: &AppError) {
    eprintln!("{error}");
    if let Some(source) = error.source() {
        eprintln!("cause: {source}");
    }
}

fn log_settings_error(error: &SettingsFileError) {
    eprintln!("{error}");
    if let Some(source) = error.source() {
        eprintln!("cause: {source}");
    }
}

fn linux_settings_error_message(language: UiLanguage, error: &SettingsFileError) -> &'static str {
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

fn linux_domain_error_message(language: UiLanguage, error: &DomainError) -> &'static str {
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

fn linux_window_error_message(
    language: UiLanguage,
    error: &crate::app::WindowControlError,
) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_tooltip_text_filters_empty_titles_and_joins_lines() {
        let tooltip = tab_tooltip_text(
            ["  Editor  ", "", "Terminal", "   "]
                .into_iter()
                .map(str::to_owned),
        );

        assert_eq!(tooltip.as_deref(), Some("Editor\nTerminal"));
    }

    #[test]
    fn tab_tooltip_text_returns_none_without_titles() {
        let tooltip = tab_tooltip_text(["", "   "].into_iter().map(str::to_owned));

        assert_eq!(tooltip, None);
    }

    #[test]
    fn program_arguments_round_trip_spaces_quotes_and_backslashes() {
        let arguments = vec![
            String::from("plain"),
            String::from("two words"),
            String::from("quote\"inside"),
            String::from("path\\tail\\"),
        ];

        let formatted = format_program_arguments(&arguments);
        assert_eq!(parse_program_arguments(&formatted), Ok(arguments));
    }

    #[test]
    fn program_arguments_preserve_empty_argument_and_reject_unclosed_quote() {
        let parsed = parse_program_arguments(r#"--profile "Work A" "" "quoted \"value\"""#);
        assert_eq!(
            parsed,
            Ok(vec![
                String::from("--profile"),
                String::from("Work A"),
                String::new(),
                String::from("quoted \"value\""),
            ])
        );
        assert_eq!(
            parse_program_arguments(r#""Work A"#),
            Err(ProgramArgumentsParseError::UnterminatedQuote)
        );
    }

    #[test]
    fn visible_tab_capacity_uses_only_full_tabs() {
        assert_eq!(visible_tab_capacity_for_width(0, 4), 0);
        assert_eq!(visible_tab_capacity_for_width(GTK_TAB_WIDTH - 1, 4), 0);
        assert_eq!(visible_tab_capacity_for_width(GTK_TAB_WIDTH, 4), 1);
        assert_eq!(
            visible_tab_capacity_for_width(GTK_TAB_WIDTH * 2 + GTK_TAB_GAP, 4),
            2
        );
        assert_eq!(visible_tab_capacity_for_width(DEFAULT_WIDTH, 0), 0);
    }

    #[test]
    fn visible_tab_capacity_reserves_overflow_button_space() {
        let two_tabs_width = GTK_TAB_WIDTH * 2 + GTK_TAB_GAP;
        assert_eq!(
            visible_tab_capacity_for_tab_strip_width(two_tabs_width, 2),
            2
        );
        assert_eq!(
            visible_tab_capacity_for_tab_strip_width(two_tabs_width, 3),
            1
        );

        let two_tabs_with_overflow_width = two_tabs_width + GTK_TAB_GAP + TAB_OVERFLOW_BUTTON_WIDTH;
        assert_eq!(
            visible_tab_capacity_for_tab_strip_width(two_tabs_with_overflow_width, 3),
            2
        );
    }

    #[test]
    fn tab_overflow_range_keeps_active_tab_visible_when_not_dragging() {
        assert_eq!(linux_tab_overflow_range(5, 2, 0, Some(4), false), (3, 5));
        assert_eq!(linux_tab_overflow_range(5, 2, 3, Some(0), false), (0, 2));
        assert_eq!(linux_tab_overflow_range(5, 2, 0, Some(2), false), (1, 3));
    }

    #[test]
    fn tab_overflow_range_preserves_scroll_during_reorder_drag() {
        assert_eq!(linux_tab_overflow_range(5, 2, 0, Some(4), true), (0, 2));
        assert_eq!(linux_tab_overflow_range(5, 9, 3, Some(4), true), (0, 5));
        assert_eq!(linux_tab_overflow_range(5, 0, 3, Some(4), true), (0, 0));
    }

    #[test]
    fn tab_overflow_menu_label_matches_windows_indexed_format() {
        assert_eq!(tab_overflow_menu_label(0, "Main"), "1 Main");
        assert_eq!(tab_overflow_menu_label(9, "Work"), "10 Work");
    }

    #[test]
    fn tab_reorder_insertion_index_uses_visible_midpoints() {
        assert_eq!(
            linux_tab_reorder_insertion_index_for_x(0.0, 5, 2, 1),
            Some(1)
        );
        assert_eq!(
            linux_tab_reorder_insertion_index_for_x(f64::from(GTK_TAB_WIDTH), 5, 2, 1),
            Some(2)
        );
        assert_eq!(
            linux_tab_reorder_insertion_index_for_x(400.0, 5, 2, 1),
            Some(3)
        );
        assert_eq!(linux_tab_reorder_insertion_index_for_x(0.0, 5, 0, 1), None);
    }

    #[test]
    fn tab_reorder_scroll_moves_one_visible_tab_at_edges() {
        let width = GTK_TAB_WIDTH * 2 + GTK_TAB_GAP;
        assert_eq!(
            linux_tab_reorder_scroll_first_index(0.0, width, 5, 2, 2),
            Some(1)
        );
        assert_eq!(
            linux_tab_reorder_scroll_first_index(f64::from(width), width, 5, 2, 0),
            Some(1)
        );
        assert_eq!(
            linux_tab_reorder_scroll_first_index(0.0, width, 5, 2, 0),
            None
        );
        assert_eq!(
            linux_tab_reorder_scroll_first_index(f64::from(width), width, 5, 2, 3),
            None
        );
    }

    #[test]
    fn drop_hit_test_policy_allows_hidden_workspace_only_when_enabled() {
        assert!(drop_uses_workspace_hit_test(true, false));
        assert!(drop_uses_workspace_hit_test(true, true));
        assert!(drop_uses_workspace_hit_test(false, true));
        assert!(!drop_uses_workspace_hit_test(false, false));
    }

    #[test]
    fn splitter_overlay_policy_requires_enabled_idle_workspace_and_control() {
        assert!(splitter_overlay_should_show(
            true, false, true, false, false, true
        ));
        assert!(splitter_overlay_should_show(
            false, true, true, false, false, true
        ));
        assert!(!splitter_overlay_should_show(
            false, false, true, false, false, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, false, false, false, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, true, true, false, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, true, false, true, true
        ));
        assert!(!splitter_overlay_should_show(
            true, false, true, false, false, false
        ));
    }

    #[test]
    fn drop_candidate_marks_movement_at_threshold() -> Result<(), Box<dyn std::error::Error>> {
        let hwnd = WindowHandle::new(100)?;
        let initial_rect = Rect::new(10, 20, 110, 120)?;
        let mut candidate = LinuxDropCandidate::new(hwnd, initial_rect);

        candidate.observe_rect(Rect::new(
            10 + DROP_WINDOW_MOVE_THRESHOLD - 1,
            20,
            110 + DROP_WINDOW_MOVE_THRESHOLD - 1,
            120,
        )?);
        assert!(!candidate.moved);

        candidate.observe_rect(Rect::new(
            10 + DROP_WINDOW_MOVE_THRESHOLD,
            20,
            110 + DROP_WINDOW_MOVE_THRESHOLD,
            120,
        )?);
        assert!(candidate.moved);
        assert_eq!(candidate.hwnd, hwnd);
        Ok(())
    }

    #[test]
    fn next_tab_number_saturates_large_workspace_ids() {
        assert_eq!(next_tab_number(0), 0);
        assert_eq!(next_tab_number(u64::from(u32::MAX) + 1), u32::MAX);
    }

    #[test]
    fn next_tab_preset_name_matches_windows_sequence() {
        assert_eq!(next_tab_preset_name(0), "Tab Preset 1");
        assert_eq!(next_tab_preset_name(2), "Tab Preset 3");
    }

    #[test]
    fn localized_workspace_and_window_labels_match_state() {
        assert_eq!(
            workspace_ui_toggle_button_label(UiLanguage::English, true),
            "Hide"
        );
        assert_eq!(
            workspace_ui_toggle_menu_label(UiLanguage::Korean, false),
            "작업 영역 컨트롤 표시"
        );
        assert_eq!(new_tab_button_label(UiLanguage::English), "New");
        assert_eq!(new_tab_button_label(UiLanguage::Korean), "새 탭");
        assert_eq!(
            main_window_maximize_restore_label(UiLanguage::English, false),
            "Maximize window"
        );
        assert_eq!(
            main_window_maximize_restore_label(UiLanguage::Korean, true),
            "창 복원"
        );
        assert_eq!(
            tab_rename_prompt_label_text(UiLanguage::English),
            "Tab name:"
        );
        assert_eq!(tab_rename_prompt_label_text(UiLanguage::Korean), "탭 이름:");
        assert_eq!(
            about_dialog_title_text(UiLanguage::English),
            "About j3GridDocker"
        );
        assert_eq!(
            about_version_label_text(),
            format!("j3GridDocker {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(about_dialog_text(UiLanguage::English).contains("j3GridDocker"));
        assert!(about_dialog_text(UiLanguage::English).contains("GPL-3.0-or-later"));
        assert!(about_dialog_text(UiLanguage::English).contains("LICENSE"));
        assert!(about_dialog_text(UiLanguage::English).contains("Source Code"));
        assert!(about_dialog_text(UiLanguage::English).contains(PROJECT_URL));
    }

    #[test]
    fn docked_window_selection_status_matches_windows_guidance() {
        assert_eq!(
            docked_window_selection_status_text(UiLanguage::English),
            "Selected the docked external window region. Drag to an empty region to move it, or outside to undock it."
        );
        assert!(
            docked_window_selection_status_text(UiLanguage::Korean)
                .contains("빈 영역으로 끌면 이동")
        );
        assert!(
            docked_window_selection_status_text(UiLanguage::Korean)
                .contains("바깥으로 끌면 배치 해제")
        );
    }

    #[test]
    fn main_menu_keyboard_entry_matches_windows_native_menu_keys() {
        assert!(main_menu_keyboard_entry_key(
            gdk::Key::F10,
            gdk::ModifierType::empty()
        ));
        assert!(main_menu_keyboard_entry_key(
            gdk::Key::Alt_L,
            gdk::ModifierType::empty()
        ));
        assert!(main_menu_keyboard_entry_key(
            gdk::Key::Alt_R,
            gdk::ModifierType::empty()
        ));
        assert!(!main_menu_keyboard_entry_key(
            gdk::Key::F10,
            gdk::ModifierType::SHIFT_MASK
        ));
        assert!(!main_menu_keyboard_entry_key(
            gdk::Key::F10,
            gdk::ModifierType::CONTROL_MASK
        ));
    }

    #[test]
    fn window_close_shortcut_matches_windows_alt_f4() {
        assert!(main_window_close_shortcut_key(
            gdk::Key::F4,
            gdk::ModifierType::ALT_MASK,
            false
        ));
        assert!(main_window_close_shortcut_key(
            gdk::Key::F4,
            gdk::ModifierType::empty(),
            true
        ));
        assert!(!main_window_close_shortcut_key(
            gdk::Key::F4,
            gdk::ModifierType::empty(),
            false
        ));
        assert!(!main_window_close_shortcut_key(
            gdk::Key::F4,
            gdk::ModifierType::ALT_MASK | gdk::ModifierType::CONTROL_MASK,
            true
        ));
        assert!(!main_window_close_shortcut_key(
            gdk::Key::F10,
            gdk::ModifierType::ALT_MASK,
            true
        ));
    }

    #[test]
    fn focused_menu_button_activation_keys_open_menu_without_stealing_modified_shortcuts() {
        for key in [
            gdk::Key::Down,
            gdk::Key::Return,
            gdk::Key::KP_Enter,
            gdk::Key::space,
        ] {
            assert!(menu_button_keyboard_popup_key(
                key,
                gdk::ModifierType::empty()
            ));
        }
        assert!(!menu_button_keyboard_popup_key(
            gdk::Key::Down,
            gdk::ModifierType::SHIFT_MASK
        ));
        assert!(!menu_button_keyboard_popup_key(
            gdk::Key::Return,
            gdk::ModifierType::ALT_MASK
        ));
        assert!(!menu_button_keyboard_popup_key(
            gdk::Key::space,
            gdk::ModifierType::CONTROL_MASK
        ));
        assert!(!menu_button_keyboard_popup_key(
            gdk::Key::Down,
            gdk::ModifierType::CONTROL_MASK
        ));
        assert!(!menu_button_keyboard_popup_key(
            gdk::Key::Right,
            gdk::ModifierType::empty()
        ));
        assert!(menu_button_keyboard_popdown_key(
            gdk::Key::Escape,
            gdk::ModifierType::empty()
        ));
        assert!(!menu_button_keyboard_popdown_key(
            gdk::Key::Escape,
            gdk::ModifierType::SHIFT_MASK
        ));
    }

    #[test]
    fn main_menu_button_css_hides_gtk_default_arrow() {
        assert!(APP_CSS.contains(".j3-main-menu-button > button"));
        assert!(APP_CSS.contains(".j3-main-menu-button arrow"));
        assert!(APP_CSS.contains("-gtk-icon-size: 0"));
    }

    #[test]
    fn startup_settings_and_shutdown_status_text_matches_windows() {
        assert_eq!(
            startup_saved_workspace_skipped_status_text(UiLanguage::English, 2, 3),
            "Started with a new workspace. 2 saved tab(s) were not applied, and 3 tab preset(s) were loaded."
        );
        assert_eq!(
            startup_saved_workspace_skipped_status_text(UiLanguage::Korean, 2, 3),
            "새 워크스페이스로 시작합니다. 저장된 탭 2개는 적용하지 않았고, tab preset 3개는 불러왔습니다."
        );

        let error = SettingsFileError::FileTooLarge {
            path: std::path::PathBuf::from("settings.toml"),
            size: 5,
            max_size: 4,
        };
        assert_eq!(
            settings_load_failure_status_text(UiLanguage::English, &error),
            "Settings file size is invalid. Starting with a new workspace."
        );
        assert_eq!(
            settings_load_failure_status_text(UiLanguage::Korean, &error),
            "설정 파일 크기가 유효하지 않습니다. 새 워크스페이스로 시작합니다."
        );
        assert_eq!(
            shutdown_settings_save_error_message(
                UiLanguage::English,
                &ShutdownSettingsSaveError::Settings(error)
            ),
            "Settings file size is invalid."
        );

        let report = ShutdownReport::new(3, 2, 1, Vec::new());
        assert_eq!(
            shutdown_undock_summary_text(UiLanguage::Korean, &report),
            "Undock: attempted 3, restored 2, missing 1, failures 0"
        );
    }

    #[test]
    fn command_button_widths_match_windows_specs() {
        assert_eq!(COMMAND_BAR_LEFT_MARGIN, 8);
        assert_eq!(COMMAND_BAR_HEIGHT, 36);
        assert_eq!(COMMAND_BUTTON_WIDTHS, [112, 128, 112, 118]);
        assert_eq!(STATUS_BAR_HEIGHT, 24);
    }

    #[test]
    fn top_bar_geometry_matches_windows_specs() {
        assert_eq!(TOP_BAR_LEFT_MARGIN, 8);
        assert_eq!(TOP_BAR_BUTTON_SPACING, 8);
        assert_eq!(TAB_BAR_HEIGHT, 34);
        assert_eq!(WORKSPACE_TOGGLE_BUTTON_WIDTH, 64);
        assert_eq!(NEW_TAB_BUTTON_WIDTH, 48);
        assert_eq!(GTK_TAB_WIDTH, 132);
        assert_eq!(GTK_TAB_GAP, 4);
        assert_eq!(
            GTK_TAB_LABEL_WIDTH + GTK_TAB_BUTTON_GAP + GTK_TAB_CLOSE_BUTTON_WIDTH,
            GTK_TAB_WIDTH
        );
        assert_eq!(TAB_OVERFLOW_BUTTON_WIDTH, 28);
    }

    #[test]
    fn region_title_text_matches_windows_language_labels() {
        let region = RegionId::new(7);

        assert_eq!(
            region_title_text(UiLanguage::English, region, false),
            "Region 7"
        );
        assert_eq!(
            region_title_text(UiLanguage::English, region, true),
            "Region 7 - docked"
        );
        assert_eq!(
            region_title_text(UiLanguage::Korean, region, false),
            "영역 7"
        );
        assert_eq!(
            region_title_text(UiLanguage::Korean, region, true),
            "영역 7 - 배치됨"
        );
    }

    #[test]
    fn region_title_text_rect_matches_windows_inset() {
        let rect = Rect::new(10, 20, 100, 80).expect("valid rect");

        assert_eq!(
            region_title_text_rect(rect),
            CanvasTextRect {
                left: 18,
                top: 26,
                width: 84,
                height: 68,
            }
        );
    }

    #[test]
    fn close_other_tabs_status_includes_windows_summary_fields() {
        let target = TabId::new(2);
        let undock = LinuxUndockCounts {
            attempted: 3,
            restored: 2,
            missing: 1,
            failures: 0,
        };

        assert_eq!(
            close_other_tabs_status_text(
                UiLanguage::English,
                target,
                3,
                3,
                Some(target),
                undock,
                &[],
            ),
            "Close other tabs complete: closed 3 tab(s) except tab 2. Current active tab: 2. Undock: attempted 3, restored 2, missing 1, failures 0"
        );
        assert_eq!(
            close_other_tabs_status_text(UiLanguage::Korean, target, 3, 3, None, undock, &[],),
            "Close other tabs 완료: 탭 2 외 3개 탭을 닫았습니다. 현재 활성 탭: 없음. Undock: attempted 3, restored 2, missing 1, failures 0"
        );
    }

    #[test]
    fn tab_deletion_status_includes_active_tab_and_undock_summary() {
        let report =
            TabDeletionReport::new(TabId::new(3), Some(TabId::new(3)), Some(TabId::new(2)));

        assert_eq!(
            tab_deletion_status_text(UiLanguage::English, &report),
            "Tab 3 deleted. Current active tab: 2. Undock: attempted 0, restored 0, missing 0, failures 0"
        );
        assert_eq!(
            tab_deletion_status_text(UiLanguage::Korean, &report),
            "탭 3 삭제 완료. 현재 활성 탭: 2. Undock: attempted 0, restored 0, missing 0, failures 0"
        );
    }

    #[test]
    fn switch_tab_failure_status_matches_windows_context() {
        let context = LinuxTabSwitchStatusContext {
            target: LinuxTabStatusLabel {
                tab_id: TabId::new(2),
                name: Some("Second".to_owned()),
            },
            previous_active: Some(LinuxTabStatusLabel {
                tab_id: TabId::new(1),
                name: Some("First".to_owned()),
            }),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "창 위치 변경 실패",
            None,
        ));

        assert_eq!(
            switch_tab_failure_status_text(
                UiLanguage::English,
                &context,
                Some(TabId::new(1)),
                &error
            ),
            "Target tab window placement failed: Second (tab 2). Kept previous active tab First (tab 1). Tried to roll back by hiding the target tab window. Cause: External window position could not be changed."
        );
    }

    #[test]
    fn tab_deletion_error_status_matches_windows_context() {
        let context = LinuxTabDeletionStatusContext {
            deleted: LinuxTabStatusLabel {
                tab_id: TabId::new(1),
                name: Some("First".to_owned()),
            },
            previous_active: Some(LinuxTabStatusLabel {
                tab_id: TabId::new(1),
                name: Some("First".to_owned()),
            }),
            automatic_target: Some(LinuxTabStatusLabel {
                tab_id: TabId::new(2),
                name: Some("Second".to_owned()),
            }),
        };
        let error = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "창 위치 변경 실패",
            None,
        ));

        assert_eq!(
            tab_deletion_error_status_text(
                UiLanguage::English,
                &context,
                Some(TabId::new(1)),
                &error
            ),
            "Automatic switch after tab delete failed: delete target First (tab 1), switch target Second (tab 2). Rolled back the delete; current active tab: First (tab 1). Cause: External window position could not be changed.. Undock: not completed because of failure"
        );
    }

    #[test]
    fn tab_rename_cancel_status_matches_windows() {
        assert_eq!(
            tab_rename_cancel_status_text(UiLanguage::English, TabId::new(5)),
            "Tab 5 rename canceled."
        );
        assert_eq!(
            tab_rename_cancel_status_text(UiLanguage::Korean, TabId::new(5)),
            "탭 5 이름 변경을 취소했습니다."
        );
    }

    #[test]
    fn tab_reorder_status_matches_windows_target_and_noop() {
        assert_eq!(
            tab_reorder_status_text(
                UiLanguage::English,
                TabId::new(3),
                Some(TabId::new(1)),
                true
            ),
            "Tab order changed: tab 3 -> before tab 1"
        );
        assert_eq!(
            tab_reorder_status_text(UiLanguage::Korean, TabId::new(3), Some(TabId::new(1)), true),
            "탭 순서를 변경했습니다: 탭 3 -> 탭 1 앞"
        );
        assert_eq!(
            tab_reorder_status_text(UiLanguage::English, TabId::new(3), None, true),
            "Tab order changed: tab 3 -> last position"
        );
        assert_eq!(
            tab_reorder_status_text(UiLanguage::Korean, TabId::new(3), None, false),
            "탭 순서를 변경하지 않았습니다: 탭 3 위치가 그대로입니다."
        );
    }

    #[test]
    fn tab_operation_error_status_matches_windows() {
        let error = AppError::Domain(DomainError::TabNotFound(TabId::new(7)));

        assert_eq!(
            tab_operation_error_status_text(UiLanguage::English, TabId::new(3), "reorder", &error),
            "Tab 3 reorder failed: Requested tab could not be found."
        );
        assert_eq!(
            tab_operation_error_status_text(UiLanguage::Korean, TabId::new(3), "rename", &error),
            "탭 3 이름 변경 실패: 요청한 탭을 찾을 수 없습니다."
        );
    }

    #[test]
    fn tab_preset_status_matches_windows_save_edit_delete() {
        assert_eq!(
            tab_preset_save_missing_tab_status_text(UiLanguage::English),
            "There is no tab to save."
        );
        assert_eq!(
            tab_preset_load_missing_active_tab_status_text(UiLanguage::English),
            "There is no active tab to load the tab preset into."
        );
        assert_eq!(
            tab_preset_load_missing_tab_status_text(UiLanguage::English),
            "There is no tab to load the tab preset into."
        );
        assert_eq!(
            tab_preset_empty_status_text(UiLanguage::English, PresetAction::Load),
            "There are no saved tab presets."
        );
        assert_eq!(
            tab_preset_empty_status_text(UiLanguage::Korean, PresetAction::Edit),
            "편집할 저장된 탭 preset이 없습니다."
        );
        assert_eq!(
            tab_preset_empty_status_text(UiLanguage::English, PresetAction::Delete),
            "There are no saved tab presets to delete."
        );
        assert_eq!(
            tab_preset_save_success_status_text(UiLanguage::English, "Workbench", 2),
            "Tab preset saved: Workbench. Program(s): 2"
        );
        assert_eq!(
            tab_preset_save_success_status_text(UiLanguage::Korean, "Workbench", 2),
            "탭 preset 저장 완료: Workbench. 프로그램 2개"
        );
        assert_eq!(
            tab_preset_edit_success_status_text(UiLanguage::English, "Workbench", 1),
            "Tab preset edited: Workbench. Program(s): 1"
        );
        assert_eq!(
            tab_preset_delete_success_status_text(UiLanguage::Korean, "Workbench"),
            "탭 preset 삭제 완료: Workbench"
        );
        assert_eq!(
            tab_preset_save_cancel_status_text(UiLanguage::English),
            "Tab preset save was canceled."
        );
        assert_eq!(
            tab_preset_edit_cancel_status_text(UiLanguage::Korean),
            "탭 preset 편집을 취소했습니다."
        );
    }

    #[test]
    fn tab_preset_edit_dialog_text_matches_windows() {
        let program =
            ExternalProgramSpec::new("/usr/bin/editor", Some(String::from("Editor"))).unwrap();

        assert_eq!(
            tab_preset_edit_window_title_text(UiLanguage::English),
            "Edit tab preset"
        );
        assert_eq!(
            tab_preset_edit_message_title_text(UiLanguage::Korean),
            "탭 프리셋 편집"
        );
        assert_eq!(
            tab_preset_edit_instruction_text(UiLanguage::English, 0),
            "Edit the preset name."
        );
        assert_eq!(
            tab_preset_edit_instruction_text(UiLanguage::Korean, 3),
            "도킹된 프로그램 3개의 실행 파일과 인수를 편집합니다."
        );
        assert_eq!(
            tab_preset_name_label_text(UiLanguage::Korean),
            "프리셋 이름"
        );
        assert_eq!(
            tab_preset_executable_label_text(UiLanguage::English),
            "Executable path"
        );
        assert_eq!(
            tab_preset_arguments_label_text(UiLanguage::Korean),
            "실행 인수"
        );
        assert_eq!(
            tab_preset_program_row_header_text(UiLanguage::English, 1, 2, &program),
            "Program 1/2: Editor"
        );
        assert_eq!(
            tab_preset_name_validation_message(
                UiLanguage::English,
                DomainError::EmptyTabPresetName
            ),
            "Tab preset name cannot be empty."
        );
        assert_eq!(
            program_edit_dialog_validation_message(
                UiLanguage::English,
                2,
                "Program executable path cannot be empty.",
            ),
            "Program 2: Program executable path cannot be empty."
        );
    }

    #[test]
    fn tab_preset_failure_status_matches_windows() {
        let error = AppError::Domain(DomainError::TabPresetNotFound("Missing".to_owned()));

        assert_eq!(
            tab_preset_edit_failure_status_text(UiLanguage::English, "Missing", &error),
            "Tab preset edit failed: Missing. Cause: Requested tab preset could not be found."
        );
        assert_eq!(
            tab_preset_delete_failure_status_text(UiLanguage::Korean, "Missing", &error),
            "탭 preset 삭제 실패: Missing. 원인: 요청한 탭 preset을 찾을 수 없습니다."
        );
    }

    #[test]
    fn tab_preset_apply_status_matches_windows() {
        let target = LinuxTabStatusLabel {
            tab_id: TabId::new(2),
            name: Some("Workbench".to_owned()),
        };
        let report = LinuxTabPresetProgramRestoreReport {
            expected: 3,
            docked: 2,
            failures: vec![LinuxTabPresetProgramFailure {
                label: "Editor".to_owned(),
                message: "launch failed".to_owned(),
            }],
        };

        assert_eq!(
            tab_preset_apply_success_status_text_for_preset(
                UiLanguage::English,
                "Workbench",
                &target,
                1,
                &report,
            ),
            "Tab preset loaded: Workbench -> Workbench (tab 2). Existing undocked: 1. Programs docked 2/3; failures 1. Failed: Editor (launch failed)"
        );
        assert_eq!(
            tab_preset_apply_success_status_text_for_preset(
                UiLanguage::Korean,
                "Workbench",
                &target,
                1,
                &report,
            ),
            "탭 preset 불러오기 완료: Workbench -> Workbench (탭 2). 기존 Undock 1개. 프로그램 dock 2/3개, 실패 1개. 실패: Editor (launch failed)"
        );
    }

    #[test]
    fn tab_preset_apply_failure_status_matches_windows() {
        let target = LinuxTabStatusLabel {
            tab_id: TabId::new(2),
            name: Some("Workbench".to_owned()),
        };
        let error = AppError::Domain(DomainError::TabPresetNotFound("Missing".to_owned()));

        assert_eq!(
            tab_preset_apply_failure_status_text(UiLanguage::English, "Missing", &target, &error,),
            "Tab preset load failed: Missing -> Workbench (tab 2). Cause: Requested tab preset could not be found."
        );
    }

    #[test]
    fn placement_registration_status_matches_windows() {
        assert_eq!(
            placement_registration_status_text(
                UiLanguage::English,
                PlacementRegistration::Placed {
                    region_id: RegionId::new(1)
                },
            ),
            "External window was docked into the region."
        );
        assert_eq!(
            placement_registration_status_text(
                UiLanguage::Korean,
                PlacementRegistration::Moved {
                    from_region_id: RegionId::new(1),
                    to_region_id: RegionId::new(2),
                },
            ),
            "외부 윈도우를 다른 영역으로 이동했습니다."
        );
        assert_eq!(
            placement_registration_status_text(
                UiLanguage::English,
                PlacementRegistration::Resynced {
                    region_id: RegionId::new(1)
                },
            ),
            "External window was fitted to the current region again."
        );
    }

    #[test]
    fn drop_detach_status_distinguishes_restored_and_missing_windows() {
        assert_eq!(
            drop_detach_success_status_text(UiLanguage::English, UndockStatus::Restored),
            "External window was undocked at its current position."
        );
        assert_eq!(
            drop_detach_success_status_text(UiLanguage::Korean, UndockStatus::WindowMissing),
            "외부 윈도우가 유효하지 않아 배치 정보를 제거했습니다."
        );
    }

    #[test]
    fn drop_registration_error_status_matches_windows_context() {
        let source = RegionId::new(1);
        let target = RegionId::new(2);
        let occupied = AppError::Domain(DomainError::RegionAlreadyOccupied(target));
        let position = AppError::Window(crate::app::WindowControlError::new(
            WindowOperation::SetPosition,
            None,
            "외부 윈도우 위치를 변경할 수 없습니다.",
            None,
        ));

        assert_eq!(
            drop_registration_error_status_text(UiLanguage::English, None, target, &occupied),
            "External window dock failed: target region already has another external window."
        );
        assert_eq!(
            drop_registration_error_status_text(
                UiLanguage::Korean,
                Some(source),
                target,
                &occupied,
            ),
            "외부 윈도우 이동 실패: 대상 영역에 이미 다른 외부 윈도우가 있습니다."
        );
        assert_eq!(
            drop_registration_error_status_text(
                UiLanguage::English,
                Some(target),
                target,
                &position
            ),
            "External window refit current region failed: kept the previous region. External window position could not be changed."
        );
        assert_eq!(
            drop_registration_error_status_text(
                UiLanguage::Korean,
                Some(target),
                target,
                &position
            ),
            "외부 윈도우 현재 영역 재맞춤 실패: 기존 영역을 유지했습니다. 외부 윈도우 위치를 변경할 수 없습니다."
        );
    }

    #[test]
    fn tab_switch_status_matches_windows_success_summary() {
        let context = LinuxTabSwitchStatusContext {
            target: LinuxTabStatusLabel {
                tab_id: TabId::new(2),
                name: Some("Work".to_owned()),
            },
            previous_active: Some(LinuxTabStatusLabel {
                tab_id: TabId::new(1),
                name: Some("Main".to_owned()),
            }),
        };
        let report = TabSwitchReport::with_stale_placements(
            crate::domain::ActiveTabChange::new(Some(TabId::new(1)), TabId::new(2)),
            1,
            2,
        );

        assert_eq!(
            tab_switch_success_status_text(UiLanguage::English, &context, report),
            "Switched tab: Work (tab 2). Previous active tab: Main (tab 1). Removed 1 invalid target window(s) from placements. Removed 2 invalid previous active window(s) from placements"
        );
        assert_eq!(
            tab_switch_success_status_text(UiLanguage::Korean, &context, report),
            "탭을 전환했습니다: Work (탭 2). 이전 활성 탭: Main (탭 1). 유효하지 않은 대상 창 1개를 배치에서 제거했습니다. 유효하지 않은 이전 활성 탭 창 2개를 배치에서 제거했습니다"
        );
    }

    #[test]
    fn tab_switch_status_marks_same_tab_redisplay() {
        let context = LinuxTabSwitchStatusContext {
            target: LinuxTabStatusLabel {
                tab_id: TabId::new(2),
                name: None,
            },
            previous_active: Some(LinuxTabStatusLabel {
                tab_id: TabId::new(2),
                name: None,
            }),
        };
        let report = TabSwitchReport::new(
            crate::domain::ActiveTabChange::new(Some(TabId::new(2)), TabId::new(2)),
            0,
        );

        assert_eq!(
            tab_switch_success_status_text(UiLanguage::English, &context, report),
            "Tab redisplayed: tab 2"
        );
    }

    #[test]
    fn close_other_tabs_status_lists_failures() {
        let failures = [LinuxTabOperationFailure {
            tab_id: TabId::new(4),
            operation: "delete",
            message: "External window could not be restored.".to_owned(),
        }];

        assert_eq!(
            close_other_tabs_status_text(
                UiLanguage::English,
                TabId::new(2),
                3,
                2,
                Some(TabId::new(2)),
                LinuxUndockCounts {
                    attempted: 2,
                    restored: 1,
                    missing: 0,
                    failures: 1,
                },
                &failures,
            ),
            "Close other tabs partially failed: kept tab 2, success 2/3. Failures: tab 4 delete(External window could not be restored.). Current active tab: 2. Undock: attempted 2, restored 1, missing 0, failures 1"
        );
    }

    #[test]
    fn process_tree_stat_parser_handles_process_names_with_spaces() {
        let entry = linux_process_tree_entry_from_stat(
            42,
            "42 (worker process) S 7 1 1 0 -1 4194560 0 0 0 0",
        );

        assert_eq!(
            entry,
            Some(LinuxProcessTreeEntry {
                process_id: 42,
                parent_process_id: 7,
            })
        );
    }

    #[test]
    fn process_tree_child_index_appends_transitive_descendants() {
        let mut index = LinuxProcessTreeChildIndex::default();
        index.insert(LinuxProcessTreeEntry {
            process_id: 2,
            parent_process_id: 1,
        });
        index.insert(LinuxProcessTreeEntry {
            process_id: 3,
            parent_process_id: 2,
        });

        let mut tracked = HashSet::from([1]);
        let mut discovered = Vec::new();
        let mut stack = Vec::new();
        index.append_new_descendants(&mut tracked, &mut discovered, &mut stack);

        assert!(tracked.contains(&1));
        assert!(tracked.contains(&2));
        assert!(tracked.contains(&3));
        assert_eq!(discovered, vec![2, 3]);
    }

    #[test]
    fn preset_program_label_prefers_non_empty_title() {
        let titled =
            match ExternalProgramSpec::new("/usr/bin/editor", Some(String::from("  Editor  "))) {
                Ok(spec) => spec,
                Err(error) => panic!("valid program spec rejected: {error}"),
            };
        let untitled = match ExternalProgramSpec::new("/usr/bin/editor", Some(String::from("   ")))
        {
            Ok(spec) => spec,
            Err(error) => panic!("valid program spec rejected: {error}"),
        };

        assert_eq!(preset_program_label(&titled), "Editor");
        assert_eq!(preset_program_label(&untitled), "/usr/bin/editor");
    }
}

fn open_about_url_from_dialog(parent: &gtk::Dialog, language: UiLanguage) {
    let parent = parent.clone();
    gtk::show_uri_full(
        Some(&parent),
        PROJECT_URL,
        0,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if let Err(error) = result {
                show_about_url_error_dialog(&parent, language, &error);
            }
        },
    );
}

fn show_about_url_error_dialog(parent: &gtk::Dialog, language: UiLanguage, error: &glib::Error) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .message_type(gtk::MessageType::Info)
        .buttons(gtk::ButtonsType::Ok)
        .text(t(
            language,
            "Could not open the link in a browser.",
            "브라우저에서 링크를 열 수 없습니다.",
        ))
        .secondary_text(error.message())
        .build();
    apply_dialog_style(&dialog);
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.present();
}
