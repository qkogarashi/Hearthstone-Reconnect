#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod net;
mod process;
mod uac;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    app::run(uac::is_elevated())
}
