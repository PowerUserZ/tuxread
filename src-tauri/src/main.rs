//! `TuxRead.exe`: the app, or with `--disk-helper` the elevated disk helper (spec §5.1).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let Some(helper) = tuxread_win::helper::parse_args(&args) {
        return match tuxread_win::helper::run(&helper) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    tuxread_app::run();
    ExitCode::SUCCESS
}
