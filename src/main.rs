#![cfg_attr(not(test), windows_subsystem = "windows")]

mod action;
mod actions;
mod app;
mod clipboard;
mod config;
mod gesture;
mod hook;
mod logging;
mod resources;
mod storage;
mod ui;

fn main() {
    if let Err(error) = app::run() {
        logging::error("启动", &error);
        app::fatal_error(&format!("{error:#}"));
    }
}
