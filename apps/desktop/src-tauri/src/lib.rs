//! WokRouter desktop shell.

mod control;

use control::{DaemonStatusDto, DesktopControl};

struct DesktopState {
    control: DesktopControl,
}

#[tauri::command]
async fn daemon_status(state: tauri::State<'_, DesktopState>) -> Result<DaemonStatusDto, String> {
    state
        .control
        .status()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_daemon(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    state
        .control
        .start()
        .await
        .map_err(|error| error.to_string())
}

pub fn run() {
    let control = DesktopControl::discover().expect("desktop control initialization failed");
    tauri::Builder::default()
        .manage(DesktopState { control })
        .invoke_handler(tauri::generate_handler![daemon_status, start_daemon])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed");
}
