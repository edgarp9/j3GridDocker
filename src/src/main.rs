#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

pub mod app;
pub mod domain;
pub mod entry;
pub mod infra;

use std::error::Error;

fn main() {
    if let Err(error) = entry::run() {
        eprintln!("{}", error.user_message());
        if let Some(source) = error.source() {
            eprintln!("cause: {source}");
        }
        std::process::exit(1);
    }
}
