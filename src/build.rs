use std::error::Error;
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if should_compile_windows_resource() {
            let mut resource = winresource::WindowsResource::new();
            resource.set_icon("icon.ico");
            resource.compile()?;
        } else {
            println!(
                "cargo:warning=skipping Windows icon resource: no resource compiler is available for this cross build"
            );
        }
    }

    Ok(())
}

fn should_compile_windows_resource() -> bool {
    let host = std::env::var("HOST").unwrap_or_default();
    let target = std::env::var("TARGET").unwrap_or_default();
    if host == target {
        return true;
    }

    std::env::var_os("WINDRES").is_some()
        || command_exists("windres")
        || command_exists("x86_64-w64-mingw32-windres")
        || command_exists("llvm-windres")
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}
