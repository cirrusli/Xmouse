#![cfg_attr(not(test), windows_subsystem = "windows")]

mod action;
mod actions;
mod app;
mod autostart;
mod clipboard;
mod config;
mod gesture;
mod hook;
mod logging;
mod resources;
mod stats;
mod storage;
mod ui;

fn main() {
    if let Some(result) = autostart::handle_helper_command() {
        if let Err(error) = result {
            autostart::record_helper_error(&format!("{error:#}"));
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if let Err(error) = app::run() {
        logging::error("启动", &error);
        app::fatal_error(&format!("{error:#}"));
    }
}
