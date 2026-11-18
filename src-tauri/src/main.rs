// Release builds on Windows should not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = nivesh_app::run() {
        eprintln!("nivesh: fatal: {err:#}");
        std::process::exit(1);
    }
}
