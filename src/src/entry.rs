#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win32;

#[cfg(target_os = "linux")]
pub use linux::{EntryError, run};
#[cfg(windows)]
pub use win32::{EntryError, run};

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported {
    use std::error::Error;
    use std::fmt;

    #[derive(Debug)]
    pub struct EntryError;

    impl EntryError {
        pub const fn user_message(&self) -> &str {
            "j3GridDocker는 현재 Windows와 Linux target만 지원합니다."
        }
    }

    impl fmt::Display for EntryError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "unsupported target")
        }
    }

    impl Error for EntryError {}

    pub fn run() -> Result<(), EntryError> {
        Err(EntryError)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::{EntryError, run};
