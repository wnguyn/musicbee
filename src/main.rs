#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    musicbee_tauri_lib::run();
}