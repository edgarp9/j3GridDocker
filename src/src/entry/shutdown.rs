use crate::app::{App, AppError, ShutdownReport, WindowController};
use crate::domain::WorkspaceOptions;
use crate::infra::{PreservedStartupSessionSettings, SettingsFileError, SettingsFileStore};

pub(super) enum ShutdownSettingsSaveError {
    App(AppError),
    Settings(SettingsFileError),
}

impl From<AppError> for ShutdownSettingsSaveError {
    fn from(value: AppError) -> Self {
        Self::App(value)
    }
}

impl From<SettingsFileError> for ShutdownSettingsSaveError {
    fn from(value: SettingsFileError) -> Self {
        Self::Settings(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSavePolicy {
    Enabled,
    WaitForWorkspaceChange,
    PreserveStartupSessionUntilWorkspaceChange,
    PreserveStartupSessionOptionsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSaveMode {
    FullWorkspace,
    OptionsOnlyPreservingStartupSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShutdownMode {
    Cancellable,
    Forced,
}

impl ShutdownMode {
    fn continues_after_settings_save_failure(self) -> bool {
        matches!(self, Self::Forced)
    }
}

impl SettingsSavePolicy {
    pub(super) fn save_mode(self) -> Option<SettingsSaveMode> {
        match self {
            Self::Enabled => Some(SettingsSaveMode::FullWorkspace),
            Self::PreserveStartupSessionOptionsOnly => {
                Some(SettingsSaveMode::OptionsOnlyPreservingStartupSession)
            }
            Self::WaitForWorkspaceChange | Self::PreserveStartupSessionUntilWorkspaceChange => None,
        }
    }

    #[cfg(all(test, windows))]
    pub(super) fn can_save(self) -> bool {
        self.save_mode().is_some()
    }

    pub(super) fn allow_after_workspace_change(&mut self) {
        *self = Self::Enabled;
    }

    pub(super) fn allow_after_workspace_options_change(&mut self) {
        match self {
            Self::PreserveStartupSessionUntilWorkspaceChange
            | Self::PreserveStartupSessionOptionsOnly => {
                *self = Self::PreserveStartupSessionOptionsOnly;
            }
            Self::Enabled => {}
            Self::WaitForWorkspaceChange => {}
        }
    }
}

pub(super) struct ShutdownSettingsSaver<'a, C>
where
    C: WindowController,
{
    app: &'a App<C>,
    settings_store: &'a SettingsFileStore,
    policy: SettingsSavePolicy,
    preserved_startup_session: &'a mut Option<PreservedStartupSessionSettings>,
    workspace_options: WorkspaceOptions,
}

impl<'a, C> ShutdownSettingsSaver<'a, C>
where
    C: WindowController,
{
    pub(super) fn new(
        app: &'a App<C>,
        settings_store: &'a SettingsFileStore,
        policy: SettingsSavePolicy,
        preserved_startup_session: &'a mut Option<PreservedStartupSessionSettings>,
        workspace_options: WorkspaceOptions,
    ) -> Self {
        Self {
            app,
            settings_store,
            policy,
            preserved_startup_session,
            workspace_options,
        }
    }

    pub(super) fn save(self) -> Result<(), ShutdownSettingsSaveError> {
        match self.policy.save_mode() {
            Some(SettingsSaveMode::FullWorkspace) => {
                let mut settings = self.app.settings().map_err(AppError::from)?;
                settings.set_options(self.workspace_options);
                self.settings_store.save_workspace(&settings)?;
            }
            Some(SettingsSaveMode::OptionsOnlyPreservingStartupSession) => {
                if let Some(preserved_startup_session) = self.preserved_startup_session.as_mut() {
                    self.settings_store
                        .save_workspace_options_preserving_startup_session(
                            preserved_startup_session,
                            self.workspace_options,
                        )?;
                } else {
                    self.settings_store
                        .save_workspace_options_preserving_session(self.workspace_options)?;
                }
            }
            None => {}
        }
        Ok(())
    }
}

pub(super) struct ShutdownAttemptReport {
    pub(super) settings_save_error: Option<ShutdownSettingsSaveError>,
    pub(super) report: ShutdownReport,
}

pub(super) fn shutdown_report_after_settings_save<F>(
    settings_save_result: Result<(), ShutdownSettingsSaveError>,
    mode: ShutdownMode,
    shutdown: F,
) -> Result<ShutdownAttemptReport, ShutdownSettingsSaveError>
where
    F: FnOnce() -> ShutdownReport,
{
    match settings_save_result {
        Ok(()) => Ok(ShutdownAttemptReport {
            settings_save_error: None,
            report: shutdown(),
        }),
        Err(error) if mode.continues_after_settings_save_failure() => Ok(ShutdownAttemptReport {
            settings_save_error: Some(error),
            report: shutdown(),
        }),
        Err(error) => Err(error),
    }
}

pub(super) fn shutdown_report_is_complete(report: &ShutdownReport) -> bool {
    report.failures().is_empty()
}

pub(super) fn log_undock_failures(report: &ShutdownReport) {
    for failure in report.failures() {
        eprintln!("{failure}");
    }
}
