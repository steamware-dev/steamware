// No console window in release builds. Debug builds keep it so panics and tracing are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    steamware_lib::run();
}
