#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if let Some(exit_code) = wokrouter_desktop::run_core_operation_helper_if_requested() {
        std::process::exit(i32::from(exit_code));
    }
    if wokrouter_desktop::run().is_err() {
        eprintln!("WokRouter desktop runtime failed.");
    }
}
