use tauri::{AppHandle, Manager};

/// Toggles always-on-top for a popup window. Popups no longer default to
/// always-on-top; the header pin button is the only thing that changes it.
#[tauri::command]
pub fn set_window_always_on_top(
    app: AppHandle,
    window_label: String,
    always: bool,
) -> Result<bool, String> {
    let window = app
        .get_webview_window(&window_label)
        .ok_or_else(|| format!("window not found: {window_label}"))?;
    window
        .set_always_on_top(always)
        .map_err(|error| error.to_string())?;
    Ok(window.is_always_on_top().unwrap_or(always))
}
