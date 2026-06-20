use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::app::{ActivationPolicy, WindowControlError, WindowController, WindowOperation};
use crate::domain::{
    DomainError, ExternalProgramSpec, LayoutNode, Rect, RegionId, SavedPlacement,
    SavedWindowRestorePolicy, SplitDirection, SplitRatio, TabId, TabPreset, TabPresetNode,
    TabSettings, UiLanguage, WindowDisplayState, WindowHandle, WindowIdentity, WindowSnapshot,
    WorkspaceOptions, WorkspaceSettings, ZOrderHint, canonicalize_tab_presets,
};

mod file_replace;

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaceholderWindowController;

impl PlaceholderWindowController {
    pub const fn new() -> Self {
        Self
    }

    fn pending(operation: WindowOperation, hwnd: Option<WindowHandle>) -> WindowControlError {
        WindowControlError::new(
            operation,
            hwnd,
            "Win32 윈도우 제어는 이 target에서 사용할 수 없습니다.",
            Some(String::from(
                "infra::PlaceholderWindowController is a non-Windows boundary placeholder; no Win32 API was called.",
            )),
        )
    }
}

impl WindowController for PlaceholderWindowController {
    fn is_valid_external_window(&mut self, hwnd: WindowHandle) -> Result<bool, WindowControlError> {
        Ok(hwnd.raw() != 0)
    }

    fn is_same_external_window(
        &mut self,
        snapshot: &WindowSnapshot,
    ) -> Result<bool, WindowControlError> {
        self.is_valid_external_window(snapshot.hwnd())
    }

    fn snapshot(&mut self, hwnd: WindowHandle) -> Result<WindowSnapshot, WindowControlError> {
        Err(Self::pending(WindowOperation::Snapshot, Some(hwnd)))
    }

    fn hide(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        Err(Self::pending(WindowOperation::Hide, Some(snapshot.hwnd())))
    }

    fn show(
        &mut self,
        snapshot: &WindowSnapshot,
        _activation: ActivationPolicy,
    ) -> Result<(), WindowControlError> {
        Err(Self::pending(WindowOperation::Show, Some(snapshot.hwnd())))
    }

    fn set_position(
        &mut self,
        snapshot: &WindowSnapshot,
        _rect: Rect,
    ) -> Result<(), WindowControlError> {
        Err(Self::pending(
            WindowOperation::SetPosition,
            Some(snapshot.hwnd()),
        ))
    }

    fn restore(&mut self, snapshot: &WindowSnapshot) -> Result<(), WindowControlError> {
        Err(Self::pending(
            WindowOperation::Restore,
            Some(snapshot.hwnd()),
        ))
    }
}

#[cfg(windows)]
pub type DefaultWindowController = Win32WindowController;

#[cfg(target_os = "linux")]
pub type DefaultWindowController = LinuxWindowController;

#[cfg(not(any(windows, target_os = "linux")))]
pub type DefaultWindowController = PlaceholderWindowController;

const SETTINGS_SCHEMA_VERSION: u32 = 1;
const SETTINGS_FILE_EXTENSION: &str = "toml";
const SETTINGS_TEMP_FILE_ATTEMPTS: u32 = 100;
const MAX_SETTINGS_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SETTINGS_LAYOUT_SPLIT_DEPTH: usize = 64;
const MAX_SETTINGS_LAYOUT_DTO_SPLIT_DEPTH: usize = MAX_SETTINGS_LAYOUT_SPLIT_DEPTH + 1;

#[derive(Debug)]
pub enum SettingsFileError {
    ExecutablePath {
        source: io::Error,
    },
    ExecutableDirectoryMissing {
        path: PathBuf,
    },
    ExecutableFileNameMissing {
        path: PathBuf,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    TomlDeserialize {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    TomlSerialize {
        path: PathBuf,
        source: Box<toml::ser::Error>,
    },
    InvalidDomain {
        path: PathBuf,
        source: Box<DomainError>,
    },
    FileTooLarge {
        path: PathBuf,
        size: u64,
        max_size: u64,
    },
    UnsupportedVersion {
        path: PathBuf,
        version: u32,
    },
}

impl SettingsFileError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::ExecutablePath { .. }
            | Self::ExecutableDirectoryMissing { .. }
            | Self::ExecutableFileNameMissing { .. } => {
                "실행 파일 기준 설정 파일 경로를 결정할 수 없습니다."
            }
            Self::Io { .. } => "설정 파일을 읽거나 쓸 수 없습니다.",
            Self::TomlDeserialize { .. } => "설정 파일 형식을 해석할 수 없습니다.",
            Self::TomlSerialize { .. } => "설정 파일을 TOML 형식으로 만들 수 없습니다.",
            Self::InvalidDomain { .. } => "설정 파일 내용이 유효하지 않습니다.",
            Self::FileTooLarge { .. } => "설정 파일 크기가 유효하지 않습니다.",
            Self::UnsupportedVersion { .. } => "지원하지 않는 설정 파일 버전입니다.",
        }
    }
}

impl fmt::Display for SettingsFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutablePath { source } => {
                write!(
                    formatter,
                    "current executable path could not be resolved: source={source}"
                )
            }
            Self::ExecutableDirectoryMissing { path } => write!(
                formatter,
                "current executable path has no parent directory: path={}",
                path.display()
            ),
            Self::ExecutableFileNameMissing { path } => write!(
                formatter,
                "current executable path has no file stem: path={}",
                path.display()
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(
                formatter,
                "settings file I/O failed: action={action}, path={}, source={source}",
                path.display()
            ),
            Self::TomlDeserialize { path, source } => write!(
                formatter,
                "settings TOML parse failed: path={}, source={source}",
                path.display()
            ),
            Self::TomlSerialize { path, source } => write!(
                formatter,
                "settings TOML serialize failed: path={}, source={source}",
                path.display()
            ),
            Self::InvalidDomain { path, source } => write!(
                formatter,
                "settings domain validation failed: path={}, source={source}",
                path.display()
            ),
            Self::FileTooLarge {
                path,
                size,
                max_size,
            } => write!(
                formatter,
                "settings file is too large: path={}, size={size}, max_size={max_size}",
                path.display()
            ),
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "unsupported settings version: path={}, version={version}",
                path.display()
            ),
        }
    }
}

impl Error for SettingsFileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ExecutablePath { source } => Some(source),
            Self::ExecutableDirectoryMissing { .. } | Self::ExecutableFileNameMissing { .. } => {
                None
            }
            Self::Io { source, .. } => Some(source),
            Self::TomlDeserialize { source, .. } => Some(source.as_ref()),
            Self::TomlSerialize { source, .. } => Some(source.as_ref()),
            Self::InvalidDomain { source, .. } => Some(source.as_ref()),
            Self::FileTooLarge { .. } => None,
            Self::UnsupportedVersion { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceTabPresetsOnlySettings {
    saved_tab_count: usize,
    tab_presets: Vec<TabPreset>,
    options: WorkspaceOptions,
    preserved_startup_session: PreservedStartupSessionSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreservedStartupSessionSettings {
    content: String,
}

impl PreservedStartupSessionSettings {
    fn new(content: String) -> Self {
        Self { content }
    }
}

impl WorkspaceTabPresetsOnlySettings {
    fn new(
        saved_tab_count: usize,
        tab_presets: Vec<TabPreset>,
        options: WorkspaceOptions,
        preserved_startup_session: PreservedStartupSessionSettings,
    ) -> Self {
        Self {
            saved_tab_count,
            tab_presets: canonicalize_tab_presets(tab_presets),
            options,
            preserved_startup_session,
        }
    }

    pub const fn saved_tab_count(&self) -> usize {
        self.saved_tab_count
    }

    pub fn tab_presets(&self) -> &[TabPreset] {
        &self.tab_presets
    }

    pub const fn options(&self) -> WorkspaceOptions {
        self.options
    }

    pub fn into_tab_presets_and_preserved_session(
        self,
    ) -> (Vec<TabPreset>, PreservedStartupSessionSettings) {
        (self.tab_presets, self.preserved_startup_session)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFileStore {
    path: PathBuf,
}

impl SettingsFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn for_current_exe() -> Result<Self, SettingsFileError> {
        SettingsFileIo::default_path().map(Self::new)
    }

    pub fn default_path() -> Result<PathBuf, SettingsFileError> {
        SettingsFileIo::default_path()
    }

    #[cfg(test)]
    fn path_for_executable(executable: &Path) -> Result<PathBuf, SettingsFileError> {
        SettingsFileIo::path_for_executable(executable)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_workspace(&self) -> Result<Option<WorkspaceSettings>, SettingsFileError> {
        let Some(content) = self.file_io().read()? else {
            return Ok(None);
        };

        self.toml_codec()
            .deserialize_workspace_settings(&content)
            .map(Some)
    }

    pub fn load_workspace_for_startup(
        &self,
    ) -> Result<Option<WorkspaceTabPresetsOnlySettings>, SettingsFileError> {
        let Some(content) = self.file_io().read()? else {
            return Ok(None);
        };

        SettingsStartupLoader::new(self.toml_codec())
            .load(content)
            .map(Some)
    }

    pub fn save_workspace(&self, settings: &WorkspaceSettings) -> Result<(), SettingsFileError> {
        let content = self.toml_codec().serialize_workspace_settings(settings)?;
        self.file_io().write(&content)
    }

    pub fn save_workspace_options_preserving_session(
        &self,
        options: WorkspaceOptions,
    ) -> Result<(), SettingsFileError> {
        let Some(content) = self.file_io().read()? else {
            return Ok(());
        };

        let content = SettingsOptionsTomlPatch::new(self.toml_codec())
            .workspace_options_content_preserving_session(&content, options)?;
        self.file_io().write(&content)
    }

    pub fn save_workspace_options_preserving_startup_session(
        &self,
        preserved_startup_session: &mut PreservedStartupSessionSettings,
        options: WorkspaceOptions,
    ) -> Result<(), SettingsFileError> {
        let content = SettingsOptionsTomlPatch::new(self.toml_codec())
            .workspace_options_content_preserving_session(
                &preserved_startup_session.content,
                options,
            )?;
        self.file_io().write(&content)?;
        preserved_startup_session.content = content;
        Ok(())
    }

    fn file_io(&self) -> SettingsFileIo<'_> {
        SettingsFileIo::new(&self.path)
    }

    fn toml_codec(&self) -> SettingsTomlCodec<'_> {
        SettingsTomlCodec::new(&self.path)
    }

    #[cfg(test)]
    fn replace_settings_file(
        temp_path: &Path,
        target_path: &Path,
    ) -> Result<(), SettingsFileError> {
        SettingsFileIo::replace_settings_file(temp_path, target_path)
    }
}

#[derive(Debug, Clone, Copy)]
struct SettingsFileIo<'a> {
    path: &'a Path,
}

impl<'a> SettingsFileIo<'a> {
    const fn new(path: &'a Path) -> Self {
        Self { path }
    }

    fn default_path() -> Result<PathBuf, SettingsFileError> {
        let executable = std::env::current_exe()
            .map_err(|source| SettingsFileError::ExecutablePath { source })?;

        Self::path_for_executable(&executable)
    }

    fn path_for_executable(executable: &Path) -> Result<PathBuf, SettingsFileError> {
        let parent = executable
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| SettingsFileError::ExecutableDirectoryMissing {
                path: executable.to_path_buf(),
            })?;
        let file_stem = executable
            .file_stem()
            .filter(|stem| !stem.is_empty())
            .ok_or_else(|| SettingsFileError::ExecutableFileNameMissing {
                path: executable.to_path_buf(),
            })?;
        let mut file_name = PathBuf::from(Path::new(file_stem));
        file_name.set_extension(SETTINGS_FILE_EXTENSION);

        Ok(parent.join(file_name))
    }

    fn read(&self) -> Result<Option<String>, SettingsFileError> {
        let metadata = match fs::metadata(self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SettingsFileError::Io {
                    action: "inspect",
                    path: self.path.to_path_buf(),
                    source,
                });
            }
        };

        if !metadata.is_file() {
            return Err(SettingsFileError::Io {
                action: "inspect",
                path: self.path.to_path_buf(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "settings file path is not a regular file",
                ),
            });
        }

        let size = metadata.len();
        if size > MAX_SETTINGS_FILE_BYTES {
            return Err(SettingsFileError::FileTooLarge {
                path: self.path.to_path_buf(),
                size,
                max_size: MAX_SETTINGS_FILE_BYTES,
            });
        }

        let file = match fs::File::open(self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SettingsFileError::Io {
                    action: "open",
                    path: self.path.to_path_buf(),
                    source,
                });
            }
        };

        let mut content = String::new();
        let mut reader = file.take(MAX_SETTINGS_FILE_BYTES + 1);
        reader
            .read_to_string(&mut content)
            .map_err(|source| SettingsFileError::Io {
                action: "read",
                path: self.path.to_path_buf(),
                source,
            })?;

        let size = content.len() as u64;
        if size > MAX_SETTINGS_FILE_BYTES {
            return Err(SettingsFileError::FileTooLarge {
                path: self.path.to_path_buf(),
                size,
                max_size: MAX_SETTINGS_FILE_BYTES,
            });
        }

        Ok(Some(content))
    }

    fn write(&self, content: &str) -> Result<(), SettingsFileError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| SettingsFileError::Io {
                action: "create_dir",
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let size = content.len() as u64;
        if size > MAX_SETTINGS_FILE_BYTES {
            return Err(SettingsFileError::FileTooLarge {
                path: self.path.to_path_buf(),
                size,
                max_size: MAX_SETTINGS_FILE_BYTES,
            });
        }

        Self::ensure_target_is_not_directory(self.path)?;

        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = self
            .path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| SettingsFileError::Io {
                action: "prepare_temp_file",
                path: self.path.to_path_buf(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "settings file path has no file name",
                ),
            })?;
        let (temp_path, mut temp_file) = Self::create_temp_file(parent, file_name, self.path)?;

        let write_result = Self::write_synced_temp_file(&mut temp_file, &temp_path, content);
        drop(temp_file);
        if let Err(error) = write_result {
            Self::cleanup_temp_file(&temp_path)?;
            return Err(error);
        }

        if let Err(error) = Self::replace_settings_file(&temp_path, self.path) {
            Self::cleanup_temp_file(&temp_path)?;
            return Err(error);
        }

        Ok(())
    }

    fn ensure_target_is_not_directory(path: &Path) -> Result<(), SettingsFileError> {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => Err(SettingsFileError::Io {
                action: "inspect_existing_file",
                path: path.to_path_buf(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "settings file path is a directory",
                ),
            }),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SettingsFileError::Io {
                action: "inspect_existing_file",
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    fn create_temp_file(
        parent: &Path,
        file_name: &OsStr,
        target_path: &Path,
    ) -> Result<(PathBuf, fs::File), SettingsFileError> {
        for attempt in 0..SETTINGS_TEMP_FILE_ATTEMPTS {
            let temp_path = parent.join(Self::temp_file_name(file_name, attempt));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(file) => return Ok((temp_path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(SettingsFileError::Io {
                        action: "create_temp_file",
                        path: temp_path,
                        source,
                    });
                }
            }
        }

        Err(SettingsFileError::Io {
            action: "create_temp_file",
            path: target_path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not reserve a unique settings temp file",
            ),
        })
    }

    fn temp_file_name(file_name: &OsStr, attempt: u32) -> OsString {
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}.{}.writing", std::process::id(), attempt));
        temp_name
    }

    fn write_synced_temp_file(
        temp_file: &mut fs::File,
        temp_path: &Path,
        content: &str,
    ) -> Result<(), SettingsFileError> {
        temp_file
            .write_all(content.as_bytes())
            .map_err(|source| SettingsFileError::Io {
                action: "write_temp_file",
                path: temp_path.to_path_buf(),
                source,
            })?;
        temp_file.flush().map_err(|source| SettingsFileError::Io {
            action: "flush_temp_file",
            path: temp_path.to_path_buf(),
            source,
        })?;
        temp_file
            .sync_all()
            .map_err(|source| SettingsFileError::Io {
                action: "sync_temp_file",
                path: temp_path.to_path_buf(),
                source,
            })
    }

    fn replace_settings_file(
        temp_path: &Path,
        target_path: &Path,
    ) -> Result<(), SettingsFileError> {
        file_replace::replace_file_with_temp(temp_path, target_path).map_err(|source| {
            SettingsFileError::Io {
                action: "replace_file",
                path: target_path.to_path_buf(),
                source,
            }
        })
    }

    fn cleanup_temp_file(temp_path: &Path) -> Result<(), SettingsFileError> {
        match fs::remove_file(temp_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SettingsFileError::Io {
                action: "cleanup_temp_file",
                path: temp_path.to_path_buf(),
                source,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SettingsTomlCodec<'a> {
    path: &'a Path,
}

impl<'a> SettingsTomlCodec<'a> {
    const fn new(path: &'a Path) -> Self {
        Self { path }
    }

    fn deserialize<T>(&self, content: &str) -> Result<T, SettingsFileError>
    where
        T: for<'de> Deserialize<'de>,
    {
        toml::from_str(content).map_err(|source| SettingsFileError::TomlDeserialize {
            path: self.path.to_path_buf(),
            source: Box::new(source),
        })
    }

    fn serialize_workspace_settings(
        &self,
        settings: &WorkspaceSettings,
    ) -> Result<String, SettingsFileError> {
        let dto = WorkspaceSettingsDto::from_domain(settings);
        toml::to_string_pretty(&dto).map_err(|source| SettingsFileError::TomlSerialize {
            path: self.path.to_path_buf(),
            source: Box::new(source),
        })
    }

    fn deserialize_workspace_settings(
        &self,
        content: &str,
    ) -> Result<WorkspaceSettings, SettingsFileError> {
        let dto: WorkspaceSettingsDto = self.deserialize(content)?;
        self.workspace_settings_from_dto(dto)
    }

    fn workspace_settings_from_dto(
        &self,
        dto: WorkspaceSettingsDto,
    ) -> Result<WorkspaceSettings, SettingsFileError> {
        self.ensure_supported_schema_version(dto.schema_version)?;

        dto.into_domain()
            .map_err(|source| self.invalid_domain_error(source))
    }

    fn tab_presets_only_settings_from_startup_dto(
        &self,
        dto: WorkspaceStartupSettingsDto,
        preserved_content: String,
    ) -> Result<WorkspaceTabPresetsOnlySettings, SettingsFileError> {
        let WorkspaceStartupSettingsDto {
            schema_version: _schema_version,
            active_tab_id: _active_tab_id,
            next_tab_id: _next_tab_id,
            next_region_id: _next_region_id,
            tabs: tab_count,
            tab_presets,
            options,
        } = dto;
        self.tab_presets_only_settings_from_parts(
            tab_count.len(),
            tab_presets,
            options,
            preserved_content,
        )
    }

    fn tab_presets_only_settings_from_parts(
        &self,
        saved_tab_count: usize,
        tab_presets: Vec<TabPresetDto>,
        options: WorkspaceOptionsDto,
        preserved_content: String,
    ) -> Result<WorkspaceTabPresetsOnlySettings, SettingsFileError> {
        let tab_presets = tab_presets
            .into_iter()
            .map(TabPresetDto::into_domain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| self.invalid_domain_error(source))?;
        let options = options.into_domain();

        Ok(WorkspaceTabPresetsOnlySettings::new(
            saved_tab_count,
            tab_presets,
            options,
            PreservedStartupSessionSettings::new(preserved_content),
        ))
    }

    fn ensure_supported_schema_from_root_content(
        &self,
        root_content: &str,
    ) -> Result<(), SettingsFileError> {
        let table: toml::Table = self.deserialize(root_content)?;
        let schema_version = self.schema_version_from_table(&table)?;
        self.ensure_supported_schema_version(schema_version)
    }

    fn serialized_workspace_options_table(
        &self,
        options: WorkspaceOptions,
    ) -> Result<String, SettingsFileError> {
        let body = toml::to_string_pretty(&WorkspaceOptionsDto::from_domain(options)).map_err(
            |source| SettingsFileError::TomlSerialize {
                path: self.path.to_path_buf(),
                source: Box::new(source),
            },
        )?;
        let mut content = String::with_capacity("[options]\n".len() + body.len() + 1);
        content.push_str("[options]\n");
        content.push_str(&body);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        Ok(content)
    }

    fn schema_version_from_table(&self, table: &toml::Table) -> Result<u32, SettingsFileError> {
        let Some(value) = table.get("schema_version") else {
            return Err(self.toml_deserialize_error("missing field `schema_version`"));
        };
        let Some(version) = value.as_integer() else {
            return Err(
                self.toml_deserialize_error("schema_version must be an unsigned 32-bit integer")
            );
        };

        u32::try_from(version).map_err(|_| {
            self.toml_deserialize_error("schema_version must be an unsigned 32-bit integer")
        })
    }

    fn ensure_supported_schema_version(&self, version: u32) -> Result<(), SettingsFileError> {
        if version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsFileError::UnsupportedVersion {
                path: self.path.to_path_buf(),
                version,
            });
        }

        Ok(())
    }

    fn toml_deserialize_error(&self, message: &'static str) -> SettingsFileError {
        SettingsFileError::TomlDeserialize {
            path: self.path.to_path_buf(),
            source: Box::new(<toml::de::Error as de::Error>::custom(message)),
        }
    }

    fn invalid_domain_error(&self, source: DomainError) -> SettingsFileError {
        SettingsFileError::InvalidDomain {
            path: self.path.to_path_buf(),
            source: Box::new(source),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SettingsStartupLoader<'a> {
    codec: SettingsTomlCodec<'a>,
}

impl<'a> SettingsStartupLoader<'a> {
    const fn new(codec: SettingsTomlCodec<'a>) -> Self {
        Self { codec }
    }

    fn load(&self, content: String) -> Result<WorkspaceTabPresetsOnlySettings, SettingsFileError> {
        self.tab_presets_only_workspace(content)
    }

    fn tab_presets_only_workspace(
        &self,
        content: String,
    ) -> Result<WorkspaceTabPresetsOnlySettings, SettingsFileError> {
        let startup: WorkspaceStartupSettingsDto = self.codec.deserialize(&content)?;
        self.codec
            .ensure_supported_schema_version(startup.schema_version)?;

        self.codec
            .tab_presets_only_settings_from_startup_dto(startup, content)
    }
}

#[derive(Debug, Clone, Copy)]
struct SettingsOptionsTomlPatch<'a> {
    codec: SettingsTomlCodec<'a>,
}

impl<'a> SettingsOptionsTomlPatch<'a> {
    const fn new(codec: SettingsTomlCodec<'a>) -> Self {
        Self { codec }
    }

    fn workspace_options_content_preserving_session(
        &self,
        content: &str,
        options: WorkspaceOptions,
    ) -> Result<String, SettingsFileError> {
        let document = SettingsTomlDocument::new(content);
        if let Some(section) = document.workspace_options_table_section() {
            let options_table = self.codec.serialized_workspace_options_table(options)?;
            self.codec
                .ensure_supported_schema_from_root_content(document.root_table_content())?;
            let updated = document.replace_range(section, &options_table);
            return Ok(updated);
        }

        if document.root_settings_contains_workspace_options_assignment() {
            return self.workspace_options_content_from_full_table(content, options);
        }

        let options_table = self.codec.serialized_workspace_options_table(options)?;
        self.codec
            .ensure_supported_schema_from_root_content(document.root_table_content())?;
        let updated = document.append_workspace_options_table(&options_table);
        Ok(updated)
    }

    fn workspace_options_content_from_full_table(
        &self,
        content: &str,
        options: WorkspaceOptions,
    ) -> Result<String, SettingsFileError> {
        let mut table: toml::Table = self.codec.deserialize(content)?;
        self.workspace_options_content_from_table(&mut table, options)
    }

    fn workspace_options_content_from_table(
        &self,
        table: &mut toml::Table,
        options: WorkspaceOptions,
    ) -> Result<String, SettingsFileError> {
        let schema_version = self.codec.schema_version_from_table(table)?;
        self.codec.ensure_supported_schema_version(schema_version)?;
        table.insert(
            "options".to_owned(),
            WorkspaceOptionsDto::from_domain(options).into_toml_value(),
        );
        toml::to_string_pretty(&*table).map_err(|source| SettingsFileError::TomlSerialize {
            path: self.codec.path.to_path_buf(),
            source: Box::new(source),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceSettingsDto {
    schema_version: u32,
    active_tab_id: Option<u64>,
    next_tab_id: u64,
    next_region_id: u64,
    tabs: Vec<TabSettingsDto>,
    #[serde(default)]
    tab_presets: Vec<TabPresetDto>,
    #[serde(default)]
    options: WorkspaceOptionsDto,
}

impl WorkspaceSettingsDto {
    fn from_domain(settings: &WorkspaceSettings) -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            active_tab_id: settings.active_tab_id().map(TabId::value),
            next_tab_id: settings.next_tab_id(),
            next_region_id: settings.next_region_id(),
            tabs: settings
                .tabs()
                .iter()
                .map(TabSettingsDto::from_domain)
                .collect(),
            tab_presets: settings
                .tab_presets()
                .iter()
                .map(TabPresetDto::from_domain)
                .collect(),
            options: WorkspaceOptionsDto::from_domain(settings.options()),
        }
    }

    fn into_domain(self) -> Result<WorkspaceSettings, DomainError> {
        let tabs = self
            .tabs
            .into_iter()
            .map(TabSettingsDto::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let tab_presets = self
            .tab_presets
            .into_iter()
            .map(TabPresetDto::into_domain)
            .collect::<Result<Vec<_>, _>>()?;

        WorkspaceSettings::new_with_tab_presets_and_options(
            tabs,
            self.active_tab_id.map(TabId::new),
            self.next_tab_id,
            self.next_region_id,
            tab_presets,
            self.options.into_domain(),
        )
    }
}

#[derive(Debug, Deserialize)]
struct WorkspaceStartupSettingsDto {
    schema_version: u32,
    active_tab_id: Option<u64>,
    next_tab_id: u64,
    next_region_id: u64,
    tabs: IgnoredItemCount,
    #[serde(default)]
    tab_presets: Vec<TabPresetDto>,
    #[serde(default)]
    options: WorkspaceOptionsDto,
}

#[derive(Debug, Clone, Copy)]
struct IgnoredItemCount {
    count: usize,
}

impl IgnoredItemCount {
    const fn len(self) -> usize {
        self.count
    }
}

impl<'de> Deserialize<'de> for IgnoredItemCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(IgnoredItemCountVisitor)
    }
}

struct IgnoredItemCountVisitor;

impl<'de> Visitor<'de> for IgnoredItemCountVisitor {
    type Value = IgnoredItemCount;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sequence of ignored settings items")
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        let mut count = 0usize;
        while seq.next_element::<IgnoredAny>()?.is_some() {
            count = count
                .checked_add(1)
                .ok_or_else(|| de::Error::custom("ignored item count overflowed"))?;
        }

        Ok(IgnoredItemCount { count })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceOptionsDto {
    #[serde(default)]
    dock_hidden_workspace_ui: bool,
    #[serde(default)]
    ui_language: UiLanguageDto,
}

impl WorkspaceOptionsDto {
    fn from_domain(options: WorkspaceOptions) -> Self {
        Self {
            dock_hidden_workspace_ui: options.dock_hidden_workspace_ui(),
            ui_language: UiLanguageDto::from_domain(options.ui_language()),
        }
    }

    fn into_toml_value(self) -> toml::Value {
        let mut table = toml::map::Map::new();
        table.insert(
            "dock_hidden_workspace_ui".to_owned(),
            toml::Value::Boolean(self.dock_hidden_workspace_ui),
        );
        table.insert(
            "ui_language".to_owned(),
            toml::Value::String(self.ui_language.as_str().to_owned()),
        );
        toml::Value::Table(table)
    }

    const fn into_domain(self) -> WorkspaceOptions {
        WorkspaceOptions::new_with_language(
            self.dock_hidden_workspace_ui,
            self.ui_language.into_domain(),
        )
    }
}

impl Default for WorkspaceOptionsDto {
    fn default() -> Self {
        Self {
            dock_hidden_workspace_ui: false,
            ui_language: UiLanguageDto::English,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TomlContentSection {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy)]
struct RootSettingsOptionsScan {
    has_options_assignment: bool,
}

#[derive(Debug, Clone, Copy)]
struct SettingsTomlDocument<'a> {
    content: &'a str,
}

impl<'a> SettingsTomlDocument<'a> {
    const fn new(content: &'a str) -> Self {
        Self { content }
    }

    fn root_content(&self) -> &'a str {
        let mut offset = 0;
        let mut scanner = TomlLineScanner::new();

        for line in self.content.split_inclusive('\n') {
            let line_without_ending = trim_toml_line_ending(line);
            if scanner.can_start_table_header() && toml_table_header(line_without_ending).is_some()
            {
                return &self.content[..offset];
            }
            scanner.scan_line(line_without_ending);
            offset += line.len();
        }

        self.content
    }

    fn root_table_content(&self) -> &'a str {
        let mut offset = 0;
        let mut scanner = TomlLineScanner::new();

        for line in self.content.split_inclusive('\n') {
            let line_without_ending = trim_toml_line_ending(line);
            if scanner.can_start_table_header() && toml_table_header(line_without_ending).is_some()
            {
                return &self.content[..offset];
            }
            scanner.scan_line(line_without_ending);
            offset += line.len();
        }

        self.content
    }

    fn workspace_options_table_section(&self) -> Option<TomlContentSection> {
        let mut offset = 0;
        let mut section_start = None;
        let mut scanner = TomlLineScanner::new();

        for line in self.content.split_inclusive('\n') {
            let line_without_ending = trim_toml_line_ending(line);
            if scanner.can_start_table_header()
                && let Some(header) = toml_table_header(line_without_ending)
            {
                if let Some(start) = section_start {
                    if !toml_path_is_workspace_options_or_child(header.path) {
                        return Some(TomlContentSection { start, end: offset });
                    }
                } else if !header.is_array && toml_path_is_workspace_options_table(header.path) {
                    section_start = Some(offset);
                }
            }
            scanner.scan_line(line_without_ending);
            offset += line.len();
        }

        section_start.map(|start| TomlContentSection {
            start,
            end: self.content.len(),
        })
    }

    fn root_settings_contains_workspace_options_assignment(&self) -> bool {
        Self::root_settings_options_scan(self.root_content()).has_options_assignment
    }

    fn root_settings_options_scan(content: &str) -> RootSettingsOptionsScan {
        let mut scanner = TomlLineScanner::new();
        let mut scan = RootSettingsOptionsScan {
            has_options_assignment: false,
        };

        for line in content.split_inclusive('\n') {
            let line_without_ending = trim_toml_line_ending(line);
            if scanner.can_start_table_header()
                && root_line_assigns_workspace_options(line_without_ending)
            {
                scan.has_options_assignment = true;
            }

            scanner.scan_line(line_without_ending);
        }

        scan
    }

    fn replace_range(&self, section: TomlContentSection, replacement: &str) -> String {
        let mut updated = String::with_capacity(
            self.content.len() - (section.end - section.start) + replacement.len(),
        );
        updated.push_str(&self.content[..section.start]);
        updated.push_str(replacement);
        updated.push_str(&self.content[section.end..]);
        updated
    }

    fn append_workspace_options_table(&self, options_table: &str) -> String {
        let mut updated = String::with_capacity(self.content.len() + options_table.len() + 2);
        updated.push_str(self.content);
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !updated.is_empty() && !updated.ends_with("\n\n") {
            updated.push('\n');
        }
        updated.push_str(options_table);
        updated
    }
}

#[cfg(test)]
fn find_workspace_options_table_section(content: &str) -> Option<TomlContentSection> {
    SettingsTomlDocument::new(content).workspace_options_table_section()
}

fn root_line_assigns_workspace_options(line: &str) -> bool {
    let Some((key, _value)) = line.split_once('=') else {
        return false;
    };

    toml_path_workspace_options_is_exact(key).is_some()
}

fn trim_toml_line_ending(line: &str) -> &str {
    let without_lf = line.strip_suffix('\n').unwrap_or(line);
    without_lf.strip_suffix('\r').unwrap_or(without_lf)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TomlScanState {
    Normal,
    BasicString,
    LiteralString,
    MultilineBasicString,
    MultilineLiteralString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TomlLineScanner {
    state: TomlScanState,
    nesting_depth: usize,
}

impl TomlLineScanner {
    const fn new() -> Self {
        Self {
            state: TomlScanState::Normal,
            nesting_depth: 0,
        }
    }

    fn can_start_table_header(&self) -> bool {
        self.state == TomlScanState::Normal && self.nesting_depth == 0
    }

    fn scan_line(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match self.state {
                TomlScanState::Normal => match bytes[index] {
                    b'#' => break,
                    b'"' if bytes[index..].starts_with(b"\"\"\"") => {
                        self.state = TomlScanState::MultilineBasicString;
                        index += 3;
                    }
                    b'"' => {
                        self.state = TomlScanState::BasicString;
                        index += 1;
                    }
                    b'\'' if bytes[index..].starts_with(b"'''") => {
                        self.state = TomlScanState::MultilineLiteralString;
                        index += 3;
                    }
                    b'\'' => {
                        self.state = TomlScanState::LiteralString;
                        index += 1;
                    }
                    b'[' | b'{' => {
                        self.nesting_depth = self.nesting_depth.saturating_add(1);
                        index += 1;
                    }
                    b']' | b'}' => {
                        self.nesting_depth = self.nesting_depth.saturating_sub(1);
                        index += 1;
                    }
                    _ => index += 1,
                },
                TomlScanState::BasicString => match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        self.state = TomlScanState::Normal;
                        index += 1;
                    }
                    _ => index += 1,
                },
                TomlScanState::LiteralString => match bytes[index] {
                    b'\'' => {
                        self.state = TomlScanState::Normal;
                        index += 1;
                    }
                    _ => index += 1,
                },
                TomlScanState::MultilineBasicString => {
                    if bytes[index..].starts_with(b"\"\"\"") {
                        self.state = TomlScanState::Normal;
                        index += 3;
                    } else if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else {
                        index += 1;
                    }
                }
                TomlScanState::MultilineLiteralString => {
                    if bytes[index..].starts_with(b"'''") {
                        self.state = TomlScanState::Normal;
                        index += 3;
                    } else {
                        index += 1;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TomlTableHeader<'a> {
    path: &'a str,
    is_array: bool,
}

fn toml_table_header(line: &str) -> Option<TomlTableHeader<'_>> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let (rest, closing, is_array) = if let Some(rest) = rest.strip_prefix('[') {
        (rest, "]]", true)
    } else {
        (rest, "]", false)
    };
    let end = rest.find(closing)?;
    let path = rest[..end].trim();
    if path.is_empty() {
        return None;
    }

    let after = rest[end + closing.len()..].trim_start();
    if after.is_empty() || after.starts_with('#') {
        Some(TomlTableHeader { path, is_array })
    } else {
        None
    }
}

fn toml_path_is_workspace_options_table(path: &str) -> bool {
    toml_path_workspace_options_is_exact(path).unwrap_or(false)
}

fn toml_path_is_workspace_options_or_child(path: &str) -> bool {
    toml_path_workspace_options_is_exact(path).is_some()
}

fn toml_path_workspace_options_is_exact(path: &str) -> Option<bool> {
    let mut parser = TomlPathParser::new(path);
    parser.skip_whitespace();
    let first_is_options = parser.parse_key_matches("options")?;
    parser.skip_whitespace();

    if parser.is_finished() {
        return first_is_options.then_some(true);
    }

    if !first_is_options || !parser.consume_dot() {
        return None;
    }

    parser.skip_whitespace();
    parser.parse_key()?;
    parser.skip_whitespace();

    while !parser.is_finished() {
        if !parser.consume_dot() {
            return None;
        }
        parser.skip_whitespace();
        parser.parse_key()?;
        parser.skip_whitespace();
    }

    Some(false)
}

struct TomlPathParser<'a> {
    path: &'a str,
    index: usize,
}

impl<'a> TomlPathParser<'a> {
    const fn new(path: &'a str) -> Self {
        Self { path, index: 0 }
    }

    fn is_finished(&self) -> bool {
        self.index >= self.path.len()
    }

    fn rest(&self) -> &'a str {
        &self.path[self.index..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek_char(), Some(' ' | '\t')) {
            self.index += 1;
        }
    }

    fn consume_dot(&mut self) -> bool {
        if self.rest().starts_with('.') {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn parse_key(&mut self) -> Option<()> {
        self.parse_key_inner(None).map(|_| ())
    }

    fn parse_key_matches(&mut self, expected: &str) -> Option<bool> {
        self.parse_key_inner(Some(expected))
    }

    fn parse_key_inner(&mut self, expected: Option<&str>) -> Option<bool> {
        match self.peek_char()? {
            '"' => self.parse_basic_string_key(expected),
            '\'' => self.parse_literal_string_key(expected),
            ch if is_toml_bare_key_char(ch) => self.parse_bare_key(expected),
            _ => None,
        }
    }

    fn parse_bare_key(&mut self, expected: Option<&str>) -> Option<bool> {
        let start = self.index;
        while matches!(self.peek_char(), Some(ch) if is_toml_bare_key_char(ch)) {
            self.index += 1;
        }
        if start == self.index {
            return None;
        }

        Some(match expected {
            Some(expected) => &self.path[start..self.index] == expected,
            None => true,
        })
    }

    fn parse_basic_string_key(&mut self, expected: Option<&str>) -> Option<bool> {
        self.next_char()?;
        let mut matcher = expected.map(TomlKeyMatcher::new);

        loop {
            let ch = self.next_char()?;
            let decoded = match ch {
                '"' => {
                    return Some(match matcher {
                        Some(matcher) => matcher.is_match(),
                        None => true,
                    });
                }
                '\\' => self.parse_basic_string_escape()?,
                ch if is_toml_forbidden_string_char(ch) => return None,
                ch => ch,
            };

            if let Some(matcher) = matcher.as_mut() {
                matcher.accept(decoded);
            }
        }
    }

    fn parse_literal_string_key(&mut self, expected: Option<&str>) -> Option<bool> {
        self.next_char()?;
        let start = self.index;

        loop {
            let ch = self.next_char()?;
            match ch {
                '\'' => {
                    let end = self.index - 1;
                    return Some(match expected {
                        Some(expected) => &self.path[start..end] == expected,
                        None => true,
                    });
                }
                ch if is_toml_forbidden_string_char(ch) => return None,
                _ => {}
            }
        }
    }

    fn parse_basic_string_escape(&mut self) -> Option<char> {
        match self.next_char()? {
            'b' => Some('\u{8}'),
            't' => Some('\t'),
            'n' => Some('\n'),
            'f' => Some('\u{c}'),
            'r' => Some('\r'),
            '"' => Some('"'),
            '\\' => Some('\\'),
            'u' => self.parse_unicode_escape(4),
            'U' => self.parse_unicode_escape(8),
            _ => None,
        }
    }

    fn parse_unicode_escape(&mut self, digits: usize) -> Option<char> {
        let mut value = 0;
        for _ in 0..digits {
            value = (value << 4) | self.next_char()?.to_digit(16)?;
        }
        char::from_u32(value)
    }
}

struct TomlKeyMatcher<'a> {
    remaining: &'a str,
    matched: bool,
}

impl<'a> TomlKeyMatcher<'a> {
    const fn new(expected: &'a str) -> Self {
        Self {
            remaining: expected,
            matched: true,
        }
    }

    fn accept(&mut self, ch: char) {
        if !self.matched {
            return;
        }

        let Some(expected) = self.remaining.chars().next() else {
            self.matched = false;
            return;
        };

        if expected == ch {
            self.remaining = &self.remaining[expected.len_utf8()..];
        } else {
            self.matched = false;
        }
    }

    fn is_match(&self) -> bool {
        self.matched && self.remaining.is_empty()
    }
}

fn is_toml_bare_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn is_toml_forbidden_string_char(ch: char) -> bool {
    matches!(ch, '\u{0}'..='\u{8}' | '\u{a}'..='\u{1f}' | '\u{7f}')
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UiLanguageDto {
    #[default]
    English,
    Korean,
}

impl UiLanguageDto {
    const fn from_domain(language: UiLanguage) -> Self {
        match language {
            UiLanguage::English => Self::English,
            UiLanguage::Korean => Self::Korean,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Korean => "korean",
        }
    }

    const fn into_domain(self) -> UiLanguage {
        match self {
            Self::English => UiLanguage::English,
            Self::Korean => UiLanguage::Korean,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TabSettingsDto {
    id: u64,
    name: String,
    layout: LayoutNodeDto,
    placements: Vec<SavedPlacementDto>,
}

impl TabSettingsDto {
    fn from_domain(tab: &TabSettings) -> Self {
        Self {
            id: tab.id().value(),
            name: tab.name().to_owned(),
            layout: LayoutNodeDto::from_domain(tab.layout()),
            placements: tab
                .placements()
                .iter()
                .map(SavedPlacementDto::from_domain)
                .collect(),
        }
    }

    fn into_domain(self) -> Result<TabSettings, DomainError> {
        let placements = self
            .placements
            .into_iter()
            .map(SavedPlacementDto::into_domain)
            .collect::<Result<Vec<_>, _>>()?;

        TabSettings::new(
            TabId::new(self.id),
            self.name,
            self.layout.into_domain()?,
            placements,
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TabPresetDto {
    name: String,
    root: TabPresetNodeDto,
}

impl TabPresetDto {
    fn from_domain(preset: &TabPreset) -> Self {
        Self {
            name: preset.name().to_owned(),
            root: TabPresetNodeDto::from_domain(preset.root()),
        }
    }

    fn into_domain(self) -> Result<TabPreset, DomainError> {
        TabPreset::new(self.name, self.root.into_domain()?)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ExternalProgramSpecDto {
    executable_path: String,
    #[cfg(unix)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executable_path_unix_bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

impl ExternalProgramSpecDto {
    fn from_domain(program: &ExternalProgramSpec) -> Self {
        Self {
            executable_path: program.executable_path().to_owned(),
            #[cfg(unix)]
            executable_path_unix_bytes: program.executable_path_unix_bytes().map(<[u8]>::to_vec),
            arguments: program.arguments().to_vec(),
            title: program.title().map(str::to_owned),
        }
    }

    fn into_domain(self) -> Result<ExternalProgramSpec, DomainError> {
        #[cfg(unix)]
        if let Some(executable_path) = self.executable_path_unix_bytes {
            return ExternalProgramSpec::new_with_unix_executable_path_bytes_and_arguments(
                executable_path,
                self.arguments,
                self.title,
            );
        }

        ExternalProgramSpec::new_with_arguments(self.executable_path, self.arguments, self.title)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedPlacementDto {
    region_id: u64,
    hwnd: isize,
    snapshot: WindowSnapshotDto,
    restore_policy: SavedWindowRestorePolicyDto,
}

impl SavedPlacementDto {
    fn from_domain(placement: &SavedPlacement) -> Self {
        Self {
            region_id: placement.region_id().value(),
            hwnd: placement.hwnd().raw(),
            snapshot: WindowSnapshotDto::from_domain(placement.snapshot()),
            restore_policy: SavedWindowRestorePolicyDto::from_domain(placement.restore_policy()),
        }
    }

    fn into_domain(self) -> Result<SavedPlacement, DomainError> {
        let hwnd = WindowHandle::new(self.hwnd)?;
        SavedPlacement::new(
            RegionId::new(self.region_id),
            hwnd,
            self.snapshot.into_domain()?,
            self.restore_policy.into_domain(),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowSnapshotDto {
    hwnd: isize,
    rect: RectDto,
    display_state: WindowDisplayStateDto,
    identity: Option<WindowIdentityDto>,
    owner: Option<isize>,
    z_order_hint: Option<isize>,
    style: Option<u32>,
    ex_style: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowIdentityDto {
    thread_id: u32,
    process_id: u32,
}

impl WindowIdentityDto {
    fn from_domain(identity: WindowIdentity) -> Self {
        Self {
            thread_id: identity.thread_id(),
            process_id: identity.process_id(),
        }
    }

    const fn into_domain(self) -> WindowIdentity {
        WindowIdentity::new(self.thread_id, self.process_id)
    }
}

impl WindowSnapshotDto {
    fn from_domain(snapshot: &WindowSnapshot) -> Self {
        Self {
            hwnd: snapshot.hwnd().raw(),
            rect: RectDto::from_domain(snapshot.rect()),
            display_state: WindowDisplayStateDto::from_domain(snapshot.display_state()),
            identity: snapshot.identity().map(WindowIdentityDto::from_domain),
            owner: snapshot.owner().map(WindowHandle::raw),
            z_order_hint: snapshot.z_order_hint().map(ZOrderHint::value),
            style: snapshot.style(),
            ex_style: snapshot.ex_style(),
        }
    }

    fn into_domain(self) -> Result<WindowSnapshot, DomainError> {
        let hwnd = WindowHandle::new(self.hwnd)?;
        let mut snapshot = WindowSnapshot::new(
            hwnd,
            self.rect.into_domain()?,
            self.display_state.into_domain(),
        );

        if let Some(identity) = self.identity {
            snapshot = snapshot.with_identity(identity.into_domain());
        }
        if let Some(z_order_hint) = self.z_order_hint {
            snapshot = snapshot.with_z_order_hint(ZOrderHint::new(z_order_hint));
        }
        if let Some(owner) = self.owner {
            snapshot = snapshot.with_owner(WindowHandle::new(owner)?);
        }
        if let Some(style) = self.style {
            snapshot = snapshot.with_style(style);
        }
        if let Some(ex_style) = self.ex_style {
            snapshot = snapshot.with_ex_style(ex_style);
        }

        Ok(snapshot)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RectDto {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

impl RectDto {
    fn from_domain(rect: Rect) -> Self {
        Self {
            left: rect.left(),
            top: rect.top(),
            width: rect.width(),
            height: rect.height(),
        }
    }

    fn into_domain(self) -> Result<Rect, DomainError> {
        Rect::new(self.left, self.top, self.width, self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsNodeDtoDepth {
    remaining_split_depth: usize,
}

impl SettingsNodeDtoDepth {
    const fn root() -> Self {
        Self {
            remaining_split_depth: MAX_SETTINGS_LAYOUT_DTO_SPLIT_DEPTH,
        }
    }

    fn child<E>(self) -> Result<Self, E>
    where
        E: de::Error,
    {
        Ok(Self {
            remaining_split_depth: self
                .remaining_split_depth
                .checked_sub(1)
                .ok_or_else(layout_depth_exceeded_error)?,
        })
    }

    fn ensure_split_allowed<E>(self) -> Result<(), E>
    where
        E: de::Error,
    {
        if self.remaining_split_depth == 0 {
            return Err(layout_depth_exceeded_error());
        }

        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LayoutNodeDto {
    Region {
        id: u64,
    },
    Split {
        direction: SplitDirectionDto,
        ratio: f64,
        first: Box<LayoutNodeDto>,
        second: Box<LayoutNodeDto>,
    },
}

impl<'de> Deserialize<'de> for LayoutNodeDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        LayoutNodeDtoSeed {
            depth: SettingsNodeDtoDepth::root(),
        }
        .deserialize(deserializer)
    }
}

struct LayoutNodeDtoSeed {
    depth: SettingsNodeDtoDepth,
}

impl<'de> DeserializeSeed<'de> for LayoutNodeDtoSeed {
    type Value = LayoutNodeDto;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(LayoutNodeDtoVisitor { depth: self.depth })
    }
}

struct LayoutNodeDtoVisitor {
    depth: SettingsNodeDtoDepth,
}

impl<'de> Visitor<'de> for LayoutNodeDtoVisitor {
    type Value = LayoutNodeDto;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a settings layout node")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut kind = None;
        let mut id = None;
        let mut direction = None;
        let mut ratio = None;
        let mut first = None;
        let mut second = None;

        while let Some(field) = map.next_key()? {
            match field {
                LayoutNodeField::Kind => {
                    if kind.is_some() {
                        return Err(de::Error::duplicate_field("kind"));
                    }
                    kind = Some(map.next_value()?);
                }
                LayoutNodeField::Id => {
                    if id.is_some() {
                        return Err(de::Error::duplicate_field("id"));
                    }
                    id = Some(map.next_value()?);
                }
                LayoutNodeField::Direction => {
                    if direction.is_some() {
                        return Err(de::Error::duplicate_field("direction"));
                    }
                    direction = Some(map.next_value()?);
                }
                LayoutNodeField::Ratio => {
                    if ratio.is_some() {
                        return Err(de::Error::duplicate_field("ratio"));
                    }
                    ratio = Some(map.next_value()?);
                }
                LayoutNodeField::First => {
                    if first.is_some() {
                        return Err(de::Error::duplicate_field("first"));
                    }
                    let depth = self.depth.child()?;
                    first = Some(map.next_value_seed(LayoutNodeDtoSeed { depth })?);
                }
                LayoutNodeField::Second => {
                    if second.is_some() {
                        return Err(de::Error::duplicate_field("second"));
                    }
                    let depth = self.depth.child()?;
                    second = Some(map.next_value_seed(LayoutNodeDtoSeed { depth })?);
                }
                LayoutNodeField::Ignore => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        match kind.ok_or_else(|| de::Error::missing_field("kind"))? {
            LayoutNodeKindDto::Region => Ok(LayoutNodeDto::Region {
                id: id.ok_or_else(|| de::Error::missing_field("id"))?,
            }),
            LayoutNodeKindDto::Split => {
                self.depth.ensure_split_allowed()?;
                Ok(LayoutNodeDto::Split {
                    direction: direction.ok_or_else(|| de::Error::missing_field("direction"))?,
                    ratio: ratio.ok_or_else(|| de::Error::missing_field("ratio"))?,
                    first: Box::new(first.ok_or_else(|| de::Error::missing_field("first"))?),
                    second: Box::new(second.ok_or_else(|| de::Error::missing_field("second"))?),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LayoutNodeKindDto {
    Region,
    Split,
}

enum LayoutNodeField {
    Kind,
    Id,
    Direction,
    Ratio,
    First,
    Second,
    Ignore,
}

impl<'de> Deserialize<'de> for LayoutNodeField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_identifier(LayoutNodeFieldVisitor)
    }
}

struct LayoutNodeFieldVisitor;

impl<'de> Visitor<'de> for LayoutNodeFieldVisitor {
    type Value = LayoutNodeField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a layout node field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value {
            "kind" => LayoutNodeField::Kind,
            "id" => LayoutNodeField::Id,
            "direction" => LayoutNodeField::Direction,
            "ratio" => LayoutNodeField::Ratio,
            "first" => LayoutNodeField::First,
            "second" => LayoutNodeField::Second,
            _ => LayoutNodeField::Ignore,
        })
    }
}

fn layout_depth_exceeded_error<E>() -> E
where
    E: de::Error,
{
    E::custom(format_args!(
        "settings layout split depth exceeds maximum: max_depth={MAX_SETTINGS_LAYOUT_SPLIT_DEPTH}"
    ))
}

impl LayoutNodeDto {
    fn from_domain(layout: &LayoutNode) -> Self {
        match layout {
            LayoutNode::Region { id } => Self::Region { id: id.value() },
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => Self::Split {
                direction: SplitDirectionDto::from_domain(*direction),
                ratio: ratio.value(),
                first: Box::new(Self::from_domain(first)),
                second: Box::new(Self::from_domain(second)),
            },
        }
    }

    fn into_domain(self) -> Result<LayoutNode, DomainError> {
        self.into_domain_with_depth(MAX_SETTINGS_LAYOUT_SPLIT_DEPTH)
    }

    fn into_domain_with_depth(
        self,
        remaining_split_depth: usize,
    ) -> Result<LayoutNode, DomainError> {
        match self {
            Self::Region { id } => Ok(LayoutNode::single_region(RegionId::new(id))),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if remaining_split_depth == 0 {
                    return Err(DomainError::LayoutDepthExceeded {
                        max_depth: MAX_SETTINGS_LAYOUT_SPLIT_DEPTH,
                    });
                }
                let child_depth = remaining_split_depth - 1;

                Ok(LayoutNode::Split {
                    direction: direction.into_domain(),
                    ratio: SplitRatio::new(ratio)?,
                    first: Box::new(first.into_domain_with_depth(child_depth)?),
                    second: Box::new(second.into_domain_with_depth(child_depth)?),
                })
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TabPresetNodeDto {
    Region {
        #[serde(skip_serializing_if = "Option::is_none")]
        program: Option<ExternalProgramSpecDto>,
    },
    Split {
        direction: SplitDirectionDto,
        ratio: f64,
        first: Box<TabPresetNodeDto>,
        second: Box<TabPresetNodeDto>,
    },
}

impl<'de> Deserialize<'de> for TabPresetNodeDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        TabPresetNodeDtoSeed {
            depth: SettingsNodeDtoDepth::root(),
        }
        .deserialize(deserializer)
    }
}

struct TabPresetNodeDtoSeed {
    depth: SettingsNodeDtoDepth,
}

impl<'de> DeserializeSeed<'de> for TabPresetNodeDtoSeed {
    type Value = TabPresetNodeDto;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(TabPresetNodeDtoVisitor { depth: self.depth })
    }
}

struct TabPresetNodeDtoVisitor {
    depth: SettingsNodeDtoDepth,
}

impl<'de> Visitor<'de> for TabPresetNodeDtoVisitor {
    type Value = TabPresetNodeDto;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a settings tab preset node")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut kind = None;
        let mut program = None;
        let mut direction = None;
        let mut ratio = None;
        let mut first = None;
        let mut second = None;

        while let Some(field) = map.next_key()? {
            match field {
                TabPresetNodeField::Kind => {
                    if kind.is_some() {
                        return Err(de::Error::duplicate_field("kind"));
                    }
                    kind = Some(map.next_value()?);
                }
                TabPresetNodeField::Program => {
                    if program.is_some() {
                        return Err(de::Error::duplicate_field("program"));
                    }
                    program = Some(map.next_value()?);
                }
                TabPresetNodeField::Direction => {
                    if direction.is_some() {
                        return Err(de::Error::duplicate_field("direction"));
                    }
                    direction = Some(map.next_value()?);
                }
                TabPresetNodeField::Ratio => {
                    if ratio.is_some() {
                        return Err(de::Error::duplicate_field("ratio"));
                    }
                    ratio = Some(map.next_value()?);
                }
                TabPresetNodeField::First => {
                    if first.is_some() {
                        return Err(de::Error::duplicate_field("first"));
                    }
                    let depth = self.depth.child()?;
                    first = Some(map.next_value_seed(TabPresetNodeDtoSeed { depth })?);
                }
                TabPresetNodeField::Second => {
                    if second.is_some() {
                        return Err(de::Error::duplicate_field("second"));
                    }
                    let depth = self.depth.child()?;
                    second = Some(map.next_value_seed(TabPresetNodeDtoSeed { depth })?);
                }
                TabPresetNodeField::Ignore => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }

        match kind.ok_or_else(|| de::Error::missing_field("kind"))? {
            TabPresetNodeKindDto::Region => Ok(TabPresetNodeDto::Region {
                program: program.unwrap_or(None),
            }),
            TabPresetNodeKindDto::Split => {
                self.depth.ensure_split_allowed()?;
                Ok(TabPresetNodeDto::Split {
                    direction: direction.ok_or_else(|| de::Error::missing_field("direction"))?,
                    ratio: ratio.ok_or_else(|| de::Error::missing_field("ratio"))?,
                    first: Box::new(first.ok_or_else(|| de::Error::missing_field("first"))?),
                    second: Box::new(second.ok_or_else(|| de::Error::missing_field("second"))?),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TabPresetNodeKindDto {
    Region,
    Split,
}

enum TabPresetNodeField {
    Kind,
    Program,
    Direction,
    Ratio,
    First,
    Second,
    Ignore,
}

impl<'de> Deserialize<'de> for TabPresetNodeField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_identifier(TabPresetNodeFieldVisitor)
    }
}

struct TabPresetNodeFieldVisitor;

impl<'de> Visitor<'de> for TabPresetNodeFieldVisitor {
    type Value = TabPresetNodeField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a tab preset node field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value {
            "kind" => TabPresetNodeField::Kind,
            "program" => TabPresetNodeField::Program,
            "direction" => TabPresetNodeField::Direction,
            "ratio" => TabPresetNodeField::Ratio,
            "first" => TabPresetNodeField::First,
            "second" => TabPresetNodeField::Second,
            _ => TabPresetNodeField::Ignore,
        })
    }
}

impl TabPresetNodeDto {
    fn from_domain(root: &TabPresetNode) -> Self {
        match root {
            TabPresetNode::Region { program } => Self::Region {
                program: program.as_ref().map(ExternalProgramSpecDto::from_domain),
            },
            TabPresetNode::Split {
                direction,
                ratio,
                first,
                second,
            } => Self::Split {
                direction: SplitDirectionDto::from_domain(*direction),
                ratio: ratio.value(),
                first: Box::new(Self::from_domain(first)),
                second: Box::new(Self::from_domain(second)),
            },
        }
    }

    fn into_domain(self) -> Result<TabPresetNode, DomainError> {
        self.into_domain_with_depth(MAX_SETTINGS_LAYOUT_SPLIT_DEPTH)
    }

    fn into_domain_with_depth(
        self,
        remaining_split_depth: usize,
    ) -> Result<TabPresetNode, DomainError> {
        match self {
            Self::Region { program } => Ok(TabPresetNode::Region {
                program: program
                    .map(ExternalProgramSpecDto::into_domain)
                    .transpose()?,
            }),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if remaining_split_depth == 0 {
                    return Err(DomainError::LayoutDepthExceeded {
                        max_depth: MAX_SETTINGS_LAYOUT_SPLIT_DEPTH,
                    });
                }
                let child_depth = remaining_split_depth - 1;

                Ok(TabPresetNode::Split {
                    direction: direction.into_domain(),
                    ratio: SplitRatio::new(ratio)?,
                    first: Box::new(first.into_domain_with_depth(child_depth)?),
                    second: Box::new(second.into_domain_with_depth(child_depth)?),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SplitDirectionDto {
    Vertical,
    Horizontal,
}

impl SplitDirectionDto {
    const fn from_domain(direction: SplitDirection) -> Self {
        match direction {
            SplitDirection::Vertical => Self::Vertical,
            SplitDirection::Horizontal => Self::Horizontal,
        }
    }

    const fn into_domain(self) -> SplitDirection {
        match self {
            Self::Vertical => SplitDirection::Vertical,
            Self::Horizontal => SplitDirection::Horizontal,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WindowDisplayStateDto {
    Hidden,
    Normal,
    Minimized,
    Maximized,
}

impl WindowDisplayStateDto {
    const fn from_domain(display_state: WindowDisplayState) -> Self {
        match display_state {
            WindowDisplayState::Hidden => Self::Hidden,
            WindowDisplayState::Normal => Self::Normal,
            WindowDisplayState::Minimized => Self::Minimized,
            WindowDisplayState::Maximized => Self::Maximized,
        }
    }

    const fn into_domain(self) -> WindowDisplayState {
        match self {
            Self::Hidden => WindowDisplayState::Hidden,
            Self::Normal => WindowDisplayState::Normal,
            Self::Minimized => WindowDisplayState::Minimized,
            Self::Maximized => WindowDisplayState::Maximized,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedWindowRestorePolicyDto {
    SessionOnlyNoAutoRestore,
}

impl SavedWindowRestorePolicyDto {
    const fn from_domain(policy: SavedWindowRestorePolicy) -> Self {
        match policy {
            SavedWindowRestorePolicy::SessionOnlyNoAutoRestore => Self::SessionOnlyNoAutoRestore,
        }
    }

    const fn into_domain(self) -> SavedWindowRestorePolicy {
        match self {
            Self::SessionOnlyNoAutoRestore => SavedWindowRestorePolicy::SessionOnlyNoAutoRestore,
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_settings_path() -> Result<PathBuf, Box<dyn Error>> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?;
        Ok(std::env::temp_dir().join(format!(
            "j3grid-docker-settings-test-{}-{}.toml",
            std::process::id(),
            timestamp.as_nanos()
        )))
    }

    fn sample_settings() -> Result<WorkspaceSettings, DomainError> {
        let tab_id = TabId::new(10);
        let left = RegionId::new(20);
        let right = RegionId::new(21);
        let hwnd = WindowHandle::new(500)?;
        let snapshot = WindowSnapshot::new(
            hwnd,
            Rect::new(10, 20, 300, 200)?,
            WindowDisplayState::Normal,
        )
        .with_z_order_hint(ZOrderHint::new(499))
        .with_style(0x1000)
        .with_ex_style(0x2000);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.4)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::single_region(right)),
        };
        let placement = SavedPlacement::new(
            right,
            hwnd,
            snapshot,
            SavedWindowRestorePolicy::SessionOnlyNoAutoRestore,
        )?;

        WorkspaceSettings::new(
            vec![TabSettings::new(
                tab_id,
                "Persisted",
                layout,
                vec![placement],
            )?],
            Some(tab_id),
            11,
            22,
        )
    }

    fn nested_layout_dto(split_depth: usize) -> LayoutNodeDto {
        let mut next_region_id = 1;
        let mut layout = LayoutNodeDto::Region { id: next_region_id };
        next_region_id += 1;

        for _ in 0..split_depth {
            layout = LayoutNodeDto::Split {
                direction: SplitDirectionDto::Vertical,
                ratio: 0.5,
                first: Box::new(layout),
                second: Box::new(LayoutNodeDto::Region { id: next_region_id }),
            };
            next_region_id += 1;
        }

        layout
    }

    fn settings_content_with_layout(layout: LayoutNodeDto) -> Result<String, toml::ser::Error> {
        let dto = WorkspaceSettingsDto {
            schema_version: SETTINGS_SCHEMA_VERSION,
            active_tab_id: Some(1),
            next_tab_id: 2,
            next_region_id: MAX_SETTINGS_LAYOUT_SPLIT_DEPTH as u64 + 2,
            tabs: vec![TabSettingsDto {
                id: 1,
                name: String::from("Depth"),
                layout,
                placements: Vec::new(),
            }],
            tab_presets: Vec::new(),
            options: WorkspaceOptionsDto::default(),
        };

        toml::to_string_pretty(&dto)
    }

    #[test]
    fn settings_file_startup_load_skips_saved_tabs_even_when_legacy_restore_flag_is_present()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "Restored"
placements = []
restore_previous_session = false

[tabs.layout]
kind = "region"
id = 1
"#,
        )?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;

        assert_eq!(tab_presets_only.saved_tab_count(), 1);
        assert!(tab_presets_only.tab_presets().is_empty());

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    fn sample_tab_preset() -> Result<TabPreset, DomainError> {
        TabPreset::new(
            "Program Grid",
            TabPresetNode::Split {
                direction: SplitDirection::Vertical,
                ratio: SplitRatio::new(0.5)?,
                first: Box::new(TabPresetNode::Region {
                    program: Some(ExternalProgramSpec::new_with_arguments(
                        r"C:\Tools\editor.exe",
                        ["--profile", "Work A"],
                        Some(String::from("Editor")),
                    )?),
                }),
                second: Box::new(TabPresetNode::Region { program: None }),
            },
        )
    }

    fn sample_settings_with_tab_presets() -> Result<WorkspaceSettings, DomainError> {
        let settings = sample_settings()?;

        WorkspaceSettings::new_with_tab_presets_and_options(
            settings.tabs().to_vec(),
            settings.active_tab_id(),
            settings.next_tab_id(),
            settings.next_region_id(),
            vec![sample_tab_preset()?],
            WorkspaceOptions::default(),
        )
    }

    fn settings_temp_file_paths(path: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "settings path has no file name",
            )
        })?;
        let prefix = format!(".{}.", file_name.to_string_lossy());
        let mut temp_paths = Vec::new();

        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".writing") {
                temp_paths.push(entry.path());
            }
        }

        Ok(temp_paths)
    }

    fn assert_no_settings_temp_files(path: &Path) -> Result<(), Box<dyn Error>> {
        let temp_paths = settings_temp_file_paths(path)?;
        assert!(
            temp_paths.is_empty(),
            "settings temp files were not cleaned up: {temp_paths:?}"
        );
        Ok(())
    }

    fn content_before_marker<'a>(content: &'a str, marker: &str) -> Result<&'a str, io::Error> {
        content
            .find(marker)
            .map(|index| &content[..index])
            .ok_or_else(|| io::Error::other(format!("{marker} marker was not found")))
    }

    fn content_from_marker<'a>(content: &'a str, marker: &str) -> Result<&'a str, io::Error> {
        content
            .find(marker)
            .map(|index| &content[index..])
            .ok_or_else(|| io::Error::other(format!("{marker} marker was not found")))
    }

    #[test]
    fn settings_file_store_round_trips_workspace_settings() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings()?;

        store.save_workspace(&settings)?;
        let content = fs::read_to_string(&path)?;
        assert!(content.contains("schema_version = 1"));
        assert!(content.contains("[[tabs]]"));

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded, settings);
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_round_trips_tab_presets() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings_with_tab_presets()?;

        store.save_workspace(&settings)?;
        let content = fs::read_to_string(&path)?;
        assert!(content.contains("[[tab_presets]]"));
        assert!(content.contains("name = \"Program Grid\""));
        assert!(content.contains("executable_path"));
        assert!(content.contains("arguments = ["));
        assert!(content.contains("\"--profile\""));
        assert!(content.contains("\"Work A\""));
        assert!(content.contains("editor.exe"));

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded.tab_presets(), settings.tab_presets());
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn settings_file_store_round_trips_non_utf8_unix_preset_program_path()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let base = sample_settings()?;
        let executable_path = b"/tmp/editor-\xFF".to_vec();
        let program = ExternalProgramSpec::new_with_unix_executable_path_bytes_and_arguments(
            executable_path.clone(),
            ["--profile"],
            Some(String::from("Editor")),
        )?;
        let preset = TabPreset::new(
            "Raw Path",
            TabPresetNode::Region {
                program: Some(program),
            },
        )?;
        let settings = WorkspaceSettings::new_with_tab_presets_and_options(
            base.tabs().to_vec(),
            base.active_tab_id(),
            base.next_tab_id(),
            base.next_region_id(),
            vec![preset],
            WorkspaceOptions::default(),
        )?;

        store.save_workspace(&settings)?;
        let content = fs::read_to_string(&path)?;
        assert!(content.contains("executable_path_unix_bytes"));
        assert!(content.contains("255"));

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let loaded_program = match loaded.tab_presets()[0].root() {
            TabPresetNode::Region {
                program: Some(program),
            } => program,
            _ => return Err(io::Error::other("preset program was not loaded").into()),
        };

        assert_eq!(
            loaded_program.executable_path_unix_bytes(),
            Some(executable_path.as_slice())
        );
        assert_eq!(loaded.tab_presets(), settings.tab_presets());
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_round_trips_workspace_options() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let base = sample_settings()?;
        let settings = WorkspaceSettings::new_with_tab_presets_and_options(
            base.tabs().to_vec(),
            base.active_tab_id(),
            base.next_tab_id(),
            base.next_region_id(),
            Vec::new(),
            WorkspaceOptions::new_with_language(true, UiLanguage::Korean),
        )?;

        store.save_workspace(&settings)?;
        let content = fs::read_to_string(&path)?;
        assert!(content.contains("[options]"));
        assert!(content.contains("dock_hidden_workspace_ui = true"));
        assert!(!content.contains("restore_previous_session"));
        assert!(content.contains("ui_language = \"korean\""));

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded.options(), settings.options());
        assert_eq!(loaded, settings);
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_updates_options_without_rewriting_tabs_layout_or_placements()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings()?;
        let options = WorkspaceOptions::new_with_language(true, UiLanguage::Korean);

        store.save_workspace(&settings)?;
        let saved = fs::read_to_string(&path)?;
        let tabs_start = saved
            .find("[[tabs]]")
            .ok_or_else(|| io::Error::other("tabs section was not saved"))?;
        let options_start = saved
            .find("[options]")
            .ok_or_else(|| io::Error::other("options section was not saved"))?;
        if options_start <= tabs_start {
            return Err(
                io::Error::other("test settings unexpectedly saved options before tabs").into(),
            );
        }

        let root_section = &saved[..tabs_start];
        let tabs_section = &saved[tabs_start..options_start];
        let options_section = &saved[options_start..];
        let reordered = format!("{root_section}{options_section}{tabs_section}");
        let original_tabs = content_from_marker(&reordered, "[[tabs]]")?.to_owned();
        fs::write(&path, &reordered)?;

        store.save_workspace_options_preserving_session(options)?;

        let content = fs::read_to_string(&path)?;
        assert_eq!(
            content_from_marker(&content, "[[tabs]]")?,
            original_tabs.as_str()
        );
        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        assert_eq!(loaded.tabs(), settings.tabs());
        assert_eq!(loaded.options(), options);

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_updates_options_without_rewriting_saved_tabs()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[[tabs]]
unexpected = { nested = ["kept", 1] }

[options]
restore_previous_session = false
"#,
        )?;
        let before_content = fs::read_to_string(&path)?;
        let before_saved_session = content_before_marker(&before_content, "[options]")?.to_owned();

        store.save_workspace_options_preserving_session(WorkspaceOptions::new_with_language(
            true,
            UiLanguage::Korean,
        ))?;

        let content = fs::read_to_string(&path)?;
        assert_eq!(
            content_before_marker(&content, "[options]")?,
            before_saved_session.as_str()
        );
        let saved: toml::Value = toml::from_str(&content)?;
        let tabs = saved
            .get("tabs")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| io::Error::other("saved tabs were not preserved as an array"))?;
        let first_tab = tabs
            .first()
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("first saved tab was not preserved"))?;
        assert_eq!(
            first_tab.get("id").and_then(toml::Value::as_str),
            Some("not-a-number")
        );
        assert_eq!(
            first_tab.get("name").and_then(toml::Value::as_integer),
            Some(123)
        );
        let second_tab = tabs
            .get(1)
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("second saved tab was not preserved"))?;
        assert!(second_tab.contains_key("unexpected"));

        let options = saved
            .get("options")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("options were not saved"))?;
        assert_eq!(
            options
                .get("dock_hidden_workspace_ui")
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        assert!(!options.contains_key("restore_previous_session"));
        assert_eq!(
            options.get("ui_language").and_then(toml::Value::as_str),
            Some("korean")
        );

        let full_error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => return Err(io::Error::other("invalid tab settings were rewritten").into()),
        };
        assert!(matches!(
            full_error,
            SettingsFileError::TomlDeserialize { .. }
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_ignores_options_marker_inside_multiline_string()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
notes = """
keep this saved tab payload
[options]
restore_previous_session = true
"""
layout = "not-a-layout"

# real workspace options
[options]
restore_previous_session = false
"#,
        )?;
        let before_content = fs::read_to_string(&path)?;
        let before_saved_session =
            content_before_marker(&before_content, "# real workspace options")?.to_owned();

        store.save_workspace_options_preserving_session(WorkspaceOptions::new_with_language(
            true,
            UiLanguage::Korean,
        ))?;

        let content = fs::read_to_string(&path)?;
        assert_eq!(
            content_before_marker(&content, "# real workspace options")?,
            before_saved_session.as_str()
        );
        let saved: toml::Value = toml::from_str(&content)?;
        let tabs = saved
            .get("tabs")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| io::Error::other("saved tabs were not preserved as an array"))?;
        let first_tab = tabs
            .first()
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("first saved tab was not preserved"))?;
        let notes = first_tab
            .get("notes")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| io::Error::other("multiline tab payload was not preserved"))?;
        assert!(notes.contains("[options]\nrestore_previous_session = true"));

        let options = saved
            .get("options")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("options were not saved"))?;
        assert!(!options.contains_key("restore_previous_session"));
        assert_eq!(
            options.get("ui_language").and_then(toml::Value::as_str),
            Some("korean")
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_updates_inline_root_options_after_root_multiline_table_marker()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2
startup_notes = """
keep this root payload
[options]
restore_previous_session = true
"""
options = { restore_previous_session = false }

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"
"#,
        )?;

        store.save_workspace_options_preserving_session(WorkspaceOptions::new_with_language(
            true,
            UiLanguage::Korean,
        ))?;

        let content = fs::read_to_string(&path)?;
        let saved: toml::Value = toml::from_str(&content)?;
        let tabs = saved
            .get("tabs")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| io::Error::other("saved tabs were not preserved as an array"))?;
        let first_tab = tabs
            .first()
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("first saved tab was not preserved"))?;
        assert_eq!(
            first_tab.get("id").and_then(toml::Value::as_str),
            Some("not-a-number")
        );

        let options = saved
            .get("options")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("options were not saved"))?;
        assert!(!options.contains_key("restore_previous_session"));
        assert_eq!(
            options.get("ui_language").and_then(toml::Value::as_str),
            Some("korean")
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_updates_quoted_options_table_without_appending_duplicate()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

["options"]
restore_previous_session = false
"#,
        )?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;
        let (_tab_presets, mut preserved_startup_session) =
            tab_presets_only.into_tab_presets_and_preserved_session();
        let before_saved_session =
            content_before_marker(&preserved_startup_session.content, "[\"options\"]")?.to_owned();

        store.save_workspace_options_preserving_startup_session(
            &mut preserved_startup_session,
            WorkspaceOptions::new_with_language(true, UiLanguage::Korean),
        )?;

        let content = fs::read_to_string(&path)?;
        assert_eq!(preserved_startup_session.content, content);
        assert_eq!(
            content_before_marker(&content, "[options]")?,
            before_saved_session.as_str()
        );
        assert!(!content.contains("[\"options\"]"));
        let saved: toml::Value = toml::from_str(&content)?;
        let options = saved
            .get("options")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("options were not saved"))?;
        assert!(!options.contains_key("restore_previous_session"));
        assert_eq!(
            options.get("ui_language").and_then(toml::Value::as_str),
            Some("korean")
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_preserving_session_rejects_unsupported_schema_without_rewriting()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let unsupported_schema_version = SETTINGS_SCHEMA_VERSION + 1;
        let original = format!(
            r#"
schema_version = {unsupported_schema_version}
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[options]
restore_previous_session = false
"#
        );
        fs::write(&path, &original)?;

        let error = match store.save_workspace_options_preserving_session(
            WorkspaceOptions::new_with_language(true, UiLanguage::Korean),
        ) {
            Err(error) => error,
            Ok(()) => {
                return Err(io::Error::other("unsupported settings options were saved").into());
            }
        };

        assert!(matches!(
            error,
            SettingsFileError::UnsupportedVersion { version, .. }
                if version == unsupported_schema_version
        ));
        assert_eq!(fs::read_to_string(&path)?, original);

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_store_preserving_session_rejects_corrupt_root_schema_without_rewriting()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let original = r#"
schema_version =

[options]
restore_previous_session = false
"#;
        fs::write(&path, original)?;

        let error = match store.save_workspace_options_preserving_session(
            WorkspaceOptions::new_with_language(true, UiLanguage::Korean),
        ) {
            Err(error) => error,
            Ok(()) => return Err(io::Error::other("corrupt settings options were saved").into()),
        };

        assert!(matches!(error, SettingsFileError::TomlDeserialize { .. }));
        assert_eq!(fs::read_to_string(&path)?, original);

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn toml_workspace_options_path_detection_handles_dotted_quoted_and_invalid_paths() {
        let cases = [
            ("options", Some(true)),
            (" options ", Some(true)),
            (r#""options""#, Some(true)),
            ("'options'", Some(true)),
            (r#""\u006fptions""#, Some(true)),
            ("options.child", Some(false)),
            ("options . child", Some(false)),
            (r#""options" . "child.name""#, Some(false)),
            ("'options' . ''", Some(false)),
            ("tabs.options", None),
            ("options.", None),
            ("options..child", None),
            (r#""op\qions""#, None),
        ];

        for (path, expected) in cases {
            assert_eq!(
                toml_path_workspace_options_is_exact(path),
                expected,
                "unexpected options path detection for {path:?}"
            );
        }

        assert!(toml_path_is_workspace_options_table(r#""options""#));
        assert!(!toml_path_is_workspace_options_table("options.child"));
        assert!(toml_path_is_workspace_options_or_child("options.child"));
        assert!(!toml_path_is_workspace_options_or_child("tabs.options"));
    }

    #[test]
    fn find_workspace_options_table_section_includes_child_and_array_tables()
    -> Result<(), Box<dyn Error>> {
        let content = r#"
schema_version = 1

[options]
restore_previous_session = false

[options.ui]
language = "korean"

[[options.history]]
name = "kept"

[tabs.layout]
kind = "region"
"#;
        let start = content
            .find("[options]")
            .ok_or_else(|| io::Error::other("options section was not found"))?;
        let end = content
            .find("[tabs.layout]")
            .ok_or_else(|| io::Error::other("tabs section was not found"))?;

        assert_eq!(
            find_workspace_options_table_section(content),
            Some(TomlContentSection { start, end })
        );

        Ok(())
    }

    #[test]
    fn settings_file_store_updates_preserved_startup_session_options_without_reloading_file()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[[tabs]]
unexpected = { nested = ["kept", 1] }

[options]
restore_previous_session = false
"#,
        )?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;
        let (_tab_presets, mut preserved_startup_session) =
            tab_presets_only.into_tab_presets_and_preserved_session();
        let before_saved_session =
            content_before_marker(&preserved_startup_session.content, "[options]")?.to_owned();

        fs::write(&path, "not valid toml = [")?;
        store.save_workspace_options_preserving_startup_session(
            &mut preserved_startup_session,
            WorkspaceOptions::new_with_language(true, UiLanguage::Korean),
        )?;

        let content = fs::read_to_string(&path)?;
        assert_eq!(preserved_startup_session.content, content);
        assert_eq!(
            content_before_marker(&content, "[options]")?,
            before_saved_session.as_str()
        );
        let saved: toml::Value = toml::from_str(&content)?;
        let tabs = saved
            .get("tabs")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| io::Error::other("saved tabs were not preserved as an array"))?;
        assert_eq!(tabs.len(), 2);
        let first_tab = tabs
            .first()
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("first saved tab was not preserved"))?;
        assert_eq!(
            first_tab.get("id").and_then(toml::Value::as_str),
            Some("not-a-number")
        );
        let second_tab = tabs
            .get(1)
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("second saved tab was not preserved"))?;
        let nested = second_tab
            .get("unexpected")
            .and_then(toml::Value::as_table)
            .and_then(|unexpected| unexpected.get("nested"))
            .and_then(toml::Value::as_array)
            .ok_or_else(|| io::Error::other("nested saved tab payload was not preserved"))?;
        assert_eq!(nested.first().and_then(toml::Value::as_str), Some("kept"));
        assert_eq!(nested.get(1).and_then(toml::Value::as_integer), Some(1));
        let options = saved
            .get("options")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| io::Error::other("options were not saved"))?;
        assert!(!options.contains_key("restore_previous_session"));
        assert_eq!(
            options.get("ui_language").and_then(toml::Value::as_str),
            Some("korean")
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_startup_load_reads_presets_without_restoring_saved_workspace()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings_with_tab_presets()?;

        store.save_workspace(&settings)?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;

        assert_eq!(tab_presets_only.saved_tab_count(), settings.tabs().len());
        assert_eq!(tab_presets_only.tab_presets(), settings.tab_presets());
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_startup_load_skips_saved_tabs_and_keeps_tab_presets()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let content = r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "   "
placements = []

[tabs.layout]
kind = "region"
id = 1

[[tab_presets]]
name = "Keep"

[tab_presets.root]
kind = "region"

[options]
restore_previous_session = false
ui_language = "korean"
"#;
        fs::write(&path, content)?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;

        assert_eq!(tab_presets_only.saved_tab_count(), 1);
        assert_eq!(tab_presets_only.tab_presets().len(), 1);
        assert_eq!(tab_presets_only.tab_presets()[0].name(), "Keep");
        assert_eq!(tab_presets_only.options().ui_language(), UiLanguage::Korean);
        let (_tab_presets, preserved_startup_session) =
            tab_presets_only.into_tab_presets_and_preserved_session();
        assert_eq!(preserved_startup_session.content, content);

        let full_error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => return Err(io::Error::other("invalid tab settings were loaded").into()),
        };
        assert!(matches!(
            full_error,
            SettingsFileError::InvalidDomain {
                source,
                ..
            } if matches!(source.as_ref(), DomainError::EmptyTabName)
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_startup_load_counts_saved_tabs_without_parsing_tab_dtos()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[[tabs]]
unexpected = { nested = ["kept", 1] }

[options]
restore_previous_session = false
"#,
        )?;

        let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab_presets_only = loaded;

        assert_eq!(tab_presets_only.saved_tab_count(), 2);

        let full_error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => return Err(io::Error::other("invalid tab settings were loaded").into()),
        };
        assert!(matches!(
            full_error,
            SettingsFileError::TomlDeserialize { .. }
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_startup_load_rejects_unsupported_schema() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let unsupported_schema_version = SETTINGS_SCHEMA_VERSION + 1;
        fs::write(
            &path,
            format!(
                r#"
schema_version = {unsupported_schema_version}
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[options]
restore_previous_session = false
"#,
            ),
        )?;

        let error = match store.load_workspace_for_startup() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other("unsupported settings unexpectedly loaded").into());
            }
        };
        assert!(matches!(
            error,
            SettingsFileError::UnsupportedVersion { version, .. }
                if version == unsupported_schema_version
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_startup_load_skips_tab_dtos_with_legacy_dotted_or_inline_restore_options()
    -> Result<(), Box<dyn Error>> {
        for options_block in [
            "options.restore_previous_session = false",
            "options = { restore_previous_session = false }",
        ] {
            let path = unique_settings_path()?;
            let store = SettingsFileStore::new(path.clone());
            let content = [
                r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2

"#,
                options_block,
                r#"

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[[tabs]]
unexpected = { nested = ["kept", 1] }
"#,
            ]
            .concat();
            fs::write(&path, content)?;

            let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
            })?;
            let tab_presets_only = loaded;

            assert_eq!(tab_presets_only.saved_tab_count(), 2);

            let full_error = match store.load_workspace() {
                Err(error) => error,
                Ok(_) => return Err(io::Error::other("invalid tab settings were loaded").into()),
            };
            assert!(matches!(
                full_error,
                SettingsFileError::TomlDeserialize { .. }
            ));

            let _cleanup = fs::remove_file(&path);
        }

        Ok(())
    }

    #[test]
    fn settings_file_startup_load_ignores_root_multiline_table_marker_before_root_options()
    -> Result<(), Box<dyn Error>> {
        for options_block in [
            "options.restore_previous_session = false",
            "options = { restore_previous_session = false }",
        ] {
            let path = unique_settings_path()?;
            let store = SettingsFileStore::new(path.clone());
            let content = [
                r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 3
next_region_id = 2
startup_notes = """
keep this root payload
[options]
restore_previous_session = true
"""

"#,
                options_block,
                r#"

[[tabs]]
id = "not-a-number"
name = 123
placements = "not-an-array"
layout = "not-a-layout"

[[tabs]]
unexpected = { nested = ["kept", 1] }
"#,
            ]
            .concat();
            fs::write(&path, content)?;

            let loaded = store.load_workspace_for_startup()?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
            })?;
            let tab_presets_only = loaded;

            assert_eq!(tab_presets_only.saved_tab_count(), 2);

            let _cleanup = fs::remove_file(&path);
        }

        Ok(())
    }

    #[test]
    fn settings_file_round_trip_preserves_presets_and_defers_saved_placements()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings_with_tab_presets()?;

        store.save_workspace(&settings)?;
        let content = fs::read_to_string(&path)?;
        assert!(content.contains("[[tab_presets]]"));
        assert!(content.contains("restore_policy = \"session_only_no_auto_restore\""));
        let preset_section = content
            .split("[[tab_presets]]")
            .nth(1)
            .ok_or_else(|| io::Error::other("tab preset section was not saved"))?;
        assert!(
            !preset_section.contains("region_id =") && !preset_section.contains("id ="),
            "tab preset section must not persist region IDs: {preset_section}"
        );

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        let tab = loaded
            .tabs()
            .first()
            .ok_or_else(|| io::Error::other("loaded settings did not contain a tab"))?;
        let placement = tab
            .placements()
            .first()
            .ok_or_else(|| io::Error::other("saved placement was not loaded"))?;
        assert!(!placement.restore_policy().allows_auto_restore());

        let (state, deferred_placements) = crate::app::AppState::from_settings_layout_only(
            loaded.clone(),
            crate::domain::DEFAULT_MIN_REGION_SIZE,
        )?;

        assert_eq!(loaded, settings);
        assert_eq!(deferred_placements, 1);
        assert!(state.workspace().placements_for_tab(tab.id())?.is_empty());
        assert_eq!(state.list_tab_presets(), settings.tab_presets());
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_loads_legacy_schema_v1_without_tab_presets() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "Legacy"
placements = []

[tabs.layout]
kind = "region"
id = 1
"#,
        )?;

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded.tabs().len(), 1);
        assert!(loaded.tab_presets().is_empty());
        assert!(!loaded.options().dock_hidden_workspace_ui());
        assert_eq!(loaded.options().ui_language(), UiLanguage::English);

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_accepts_layout_at_depth_limit() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let content =
            settings_content_with_layout(nested_layout_dto(MAX_SETTINGS_LAYOUT_SPLIT_DEPTH))?;
        fs::write(&path, content)?;

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded.tabs().len(), 1);
        assert_eq!(
            loaded.tabs()[0].layout().region_ids()?.len(),
            MAX_SETTINGS_LAYOUT_SPLIT_DEPTH + 1
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_rejects_layout_exceeding_depth_limit() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let content =
            settings_content_with_layout(nested_layout_dto(MAX_SETTINGS_LAYOUT_SPLIT_DEPTH + 1))?;
        fs::write(&path, content)?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other(
                    "settings with over-depth layout unexpectedly loaded",
                )
                .into());
            }
        };

        assert!(
            matches!(
                &error,
                SettingsFileError::InvalidDomain {
                    source,
                    ..
                } if matches!(
                    source.as_ref(),
                    DomainError::LayoutDepthExceeded { max_depth }
                        if *max_depth == MAX_SETTINGS_LAYOUT_SPLIT_DEPTH
                )
            ),
            "unexpected error: {error}"
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn layout_dto_domain_conversion_rejects_layout_exceeding_depth_limit()
    -> Result<(), Box<dyn Error>> {
        let error = match nested_layout_dto(MAX_SETTINGS_LAYOUT_SPLIT_DEPTH + 1).into_domain() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other("over-depth layout unexpectedly converted").into());
            }
        };

        assert!(matches!(
            error,
            DomainError::LayoutDepthExceeded { max_depth }
                if max_depth == MAX_SETTINGS_LAYOUT_SPLIT_DEPTH
        ));

        Ok(())
    }

    #[test]
    fn settings_file_store_replaces_existing_file_with_saved_settings() -> Result<(), Box<dyn Error>>
    {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings()?;
        fs::write(&path, "schema_version =")?;

        store.save_workspace(&settings)?;

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;
        assert_eq!(loaded, settings);
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn oversized_settings_save_is_rejected_without_replacing_existing_file()
    -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let existing_content = "existing settings";
        fs::write(&path, existing_content)?;

        let oversized_name_len = usize::try_from(MAX_SETTINGS_FILE_BYTES + 1)
            .map_err(|_| io::Error::other("settings file byte limit does not fit usize"))?;
        let oversized_name = "x".repeat(oversized_name_len);
        let tab_id = TabId::new(1);
        let settings = WorkspaceSettings::new(
            vec![TabSettings::new(
                tab_id,
                oversized_name.as_str(),
                LayoutNode::single_region(RegionId::new(1)),
                Vec::new(),
            )?],
            Some(tab_id),
            2,
            2,
        )?;

        let error = match store.save_workspace(&settings) {
            Err(error) => error,
            Ok(()) => return Err(io::Error::other("oversized settings were saved").into()),
        };

        assert!(
            matches!(
                &error,
                SettingsFileError::FileTooLarge {
                    path: error_path,
                    size,
                    max_size,
                } if error_path == &path
                    && *size > MAX_SETTINGS_FILE_BYTES
                    && *max_size == MAX_SETTINGS_FILE_BYTES
            ),
            "unexpected error: {error}"
        );
        assert_eq!(error.user_message(), "설정 파일 크기가 유효하지 않습니다.");
        assert_eq!(fs::read_to_string(&path)?, existing_content);
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn replace_failure_keeps_existing_settings_file() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        fs::write(&path, "existing settings")?;
        let missing_temp_path = path.with_file_name(format!(
            ".{}.missing.writing",
            path.file_name()
                .ok_or_else(|| io::Error::other("settings path has no file name"))?
                .to_string_lossy()
        ));

        let error = match SettingsFileStore::replace_settings_file(&missing_temp_path, &path) {
            Err(error) => error,
            Ok(()) => return Err(io::Error::other("missing temp file replaced settings").into()),
        };

        assert!(matches!(
            error,
            SettingsFileError::Io {
                action: "replace_file",
                ..
            }
        ));
        assert_eq!(fs::read_to_string(&path)?, "existing settings");

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn save_failure_keeps_existing_path_and_cleans_temp_file() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        fs::create_dir(&path)?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings_with_tab_presets()?;

        let error = match store.save_workspace(&settings) {
            Err(error) => error,
            Ok(()) => {
                let _cleanup = fs::remove_dir(&path);
                return Err(io::Error::other("directory path was overwritten by settings").into());
            }
        };

        assert!(matches!(
            error,
            SettingsFileError::Io {
                action: "inspect_existing_file",
                ..
            }
        ));
        assert!(path.is_dir());
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_dir(&path);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn save_replace_failure_keeps_existing_file_and_cleans_created_temp_file()
    -> Result<(), Box<dyn Error>> {
        use std::os::windows::fs::OpenOptionsExt;

        let path = unique_settings_path()?;
        fs::write(&path, "existing settings")?;
        let locked_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)?;
        let store = SettingsFileStore::new(path.clone());
        let settings = sample_settings_with_tab_presets()?;

        let error = match store.save_workspace(&settings) {
            Err(error) => error,
            Ok(()) => {
                drop(locked_file);
                let _cleanup = fs::remove_file(&path);
                return Err(
                    io::Error::other("locked settings file was unexpectedly replaced").into(),
                );
            }
        };

        drop(locked_file);

        assert!(
            matches!(
                error,
                SettingsFileError::Io {
                    action: "replace_file",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
        assert_eq!(fs::read_to_string(&path)?, "existing settings");
        assert_no_settings_temp_files(&path)?;

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn missing_settings_file_loads_as_none() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path);

        assert_eq!(store.load_workspace()?, None);

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn special_settings_file_is_rejected_before_read() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let status = std::process::Command::new("mkfifo").arg(&path).status()?;
        if !status.success() {
            return Err(io::Error::other("mkfifo failed for settings test path").into());
        }

        let store = SettingsFileStore::new(path.clone());
        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other("special settings file unexpectedly loaded").into());
            }
        };

        assert!(
            matches!(
                &error,
                SettingsFileError::Io {
                    action: "inspect",
                    source,
                    ..
                } if source.kind() == io::ErrorKind::InvalidInput
            ),
            "unexpected error: {error}"
        );

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn default_settings_path_is_next_to_current_executable() -> Result<(), Box<dyn Error>> {
        let executable = std::env::current_exe()?;
        let path = SettingsFileStore::default_path()?;

        assert_eq!(path.parent(), executable.parent());
        assert_eq!(path.file_stem(), executable.file_stem());
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("toml")
        );

        Ok(())
    }

    #[test]
    fn settings_path_for_executable_uses_executable_file_stem() -> Result<(), Box<dyn Error>> {
        let executable = std::env::temp_dir()
            .join("j3grid-docker-test-bin")
            .join("custom-runner.exe");
        let path = SettingsFileStore::path_for_executable(&executable)?;

        assert_eq!(path, executable.with_file_name("custom-runner.toml"));

        Ok(())
    }

    #[test]
    fn corrupt_settings_file_returns_toml_error_without_deleting_file() -> Result<(), Box<dyn Error>>
    {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(&path, "schema_version =")?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other("corrupt settings unexpectedly loaded").into());
            }
        };

        assert!(matches!(error, SettingsFileError::TomlDeserialize { .. }));
        assert!(path.exists());

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn oversized_settings_file_is_rejected_before_toml_parse() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        let oversized_len = usize::try_from(MAX_SETTINGS_FILE_BYTES + 1)
            .map_err(|_| io::Error::other("settings file byte limit does not fit usize"))?;
        let mut content = Vec::with_capacity(oversized_len);
        content.extend_from_slice(b"schema_version =");
        content.resize(oversized_len, b'x');
        fs::write(&path, content)?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other("oversized settings unexpectedly loaded").into());
            }
        };

        assert!(
            matches!(
                &error,
                SettingsFileError::FileTooLarge {
                    size,
                    max_size,
                    ..
                } if *size == MAX_SETTINGS_FILE_BYTES + 1
                    && *max_size == MAX_SETTINGS_FILE_BYTES
            ),
            "unexpected error: {error}"
        );
        assert_eq!(error.user_message(), "설정 파일 크기가 유효하지 않습니다.");
        assert!(path.exists());

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_rejects_unknown_active_tab_id() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 99
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "One"
placements = []

[tabs.layout]
kind = "region"
id = 1
"#,
        )?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other(
                    "settings with unknown active tab unexpectedly loaded",
                )
                .into());
            }
        };

        assert!(matches!(
            error,
            SettingsFileError::InvalidDomain {
                source,
                ..
            } if matches!(source.as_ref(), DomainError::TabNotFound(tab_id) if *tab_id == TabId::new(99))
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_selects_first_tab_when_active_tab_id_is_missing() -> Result<(), Box<dyn Error>>
    {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
next_tab_id = 3
next_region_id = 3

[[tabs]]
id = 1
name = "One"
placements = []

[tabs.layout]
kind = "region"
id = 1

[[tabs]]
id = 2
name = "Two"
placements = []

[tabs.layout]
kind = "region"
id = 2
"#,
        )?;

        let loaded = store.load_workspace()?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "settings file was not loaded")
        })?;

        assert_eq!(loaded.active_tab_id(), Some(TabId::new(1)));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_rejects_saved_placement_with_unknown_region() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "One"

[tabs.layout]
kind = "region"
id = 1

[[tabs.placements]]
region_id = 99
hwnd = 700
restore_policy = "session_only_no_auto_restore"

[tabs.placements.snapshot]
hwnd = 700
display_state = "normal"

[tabs.placements.snapshot.rect]
left = 10
top = 20
width = 300
height = 200
"#,
        )?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(io::Error::other(
                    "settings with unknown placement region unexpectedly loaded",
                )
                .into());
            }
        };

        assert!(matches!(
            error,
            SettingsFileError::InvalidDomain {
                source,
                ..
            } if matches!(source.as_ref(), DomainError::RegionNotFound(region_id) if *region_id == RegionId::new(99))
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn settings_file_rejects_invalid_saved_hwnd() -> Result<(), Box<dyn Error>> {
        let path = unique_settings_path()?;
        let store = SettingsFileStore::new(path.clone());
        fs::write(
            &path,
            r#"
schema_version = 1
active_tab_id = 1
next_tab_id = 2
next_region_id = 2

[[tabs]]
id = 1
name = "One"

[tabs.layout]
kind = "region"
id = 1

[[tabs.placements]]
region_id = 1
hwnd = 0
restore_policy = "session_only_no_auto_restore"

[tabs.placements.snapshot]
hwnd = 0
display_state = "normal"

[tabs.placements.snapshot.rect]
left = 10
top = 20
width = 300
height = 200
"#,
        )?;

        let error = match store.load_workspace() {
            Err(error) => error,
            Ok(_) => {
                return Err(
                    io::Error::other("settings with invalid HWND unexpectedly loaded").into(),
                );
            }
        };

        assert!(matches!(
            error,
            SettingsFileError::InvalidDomain {
                source,
                ..
            } if matches!(source.as_ref(), DomainError::InvalidWindowHandle)
        ));

        let _cleanup = fs::remove_file(&path);
        Ok(())
    }
}

#[cfg(windows)]
pub use self::win32::Win32WindowController;

#[cfg(windows)]
mod win32;

#[cfg(target_os = "linux")]
pub use self::linux::{LinuxOverlayWindow, LinuxPointerState, LinuxWindowController};

#[cfg(target_os = "linux")]
mod linux;
