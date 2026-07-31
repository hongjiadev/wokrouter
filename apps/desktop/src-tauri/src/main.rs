#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if wokrouter_desktop::run().is_err() {
        eprintln!("WokRouter desktop runtime failed.");
    }
}
