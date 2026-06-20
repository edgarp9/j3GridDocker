use std::io;
use std::path::Path;

#[cfg(not(windows))]
pub(super) fn replace_file_with_temp(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    std::fs::rename(temp_path, target_path)?;
    let parent = target_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(windows)]
pub(super) fn replace_file_with_temp(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
    }

    let temp_path = wide_null_path(temp_path)?;
    let target_path = wide_null_path(target_path)?;
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;

    // SAFETY: Both path buffers are NUL-terminated, contain no interior NULs,
    // and live for the duration of the call. MoveFileExW does not retain them.
    let moved = unsafe { MoveFileExW(temp_path.as_ptr(), target_path.as_ptr(), flags) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn wide_null_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains an interior NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}
