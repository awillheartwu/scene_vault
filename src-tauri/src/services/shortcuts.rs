use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::webview::WebviewWindowBuilder;
use tauri::PhysicalSize;
use tauri::WebviewUrl;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::{error::AppError, models::app_settings::AppSettings, services::log_service};

pub const CLASSIFY_POPUP_LABEL: &str = "classify-popup";
pub const NOTE_POPUP_LABEL: &str = "note-popup";

/// Runtime layout state consulted by the shortcut handlers at press time so a
/// settings change takes effect without re-registering the shortcuts.
pub struct PopupWindowMode {
    pub split: AtomicBool,
}

/// Registers the classify shortcut at startup. Unlike `reconfigure`, this
/// always (re)registers even when the stored settings equal the defaults.
pub fn register(app: &AppHandle, settings: &AppSettings) -> Result<(), AppError> {
    app.manage(PopupWindowMode {
        split: AtomicBool::new(settings.split_popup_windows),
    });
    register_classify_shortcut(app, &settings.classify_shortcut)
        .and_then(|_| register_note_shortcut(app, &settings.note_shortcut))
}

/// Re-registers the classify shortcut after settings changes. The handler
/// only shows the popup when the shortcut is pressed (not released).
pub fn reconfigure(
    app: &AppHandle,
    previous: &AppSettings,
    current: &AppSettings,
) -> Result<(), AppError> {
    if previous.split_popup_windows != current.split_popup_windows {
        app.state::<PopupWindowMode>()
            .split
            .store(current.split_popup_windows, Ordering::Relaxed);
        // Keep window sizes sensible for the active layout and make sure a
        // leftover note window cannot linger when the layout is combined.
        if current.split_popup_windows {
            if let Some(window) = app.get_webview_window(CLASSIFY_POPUP_LABEL) {
                let _ = window.set_size(PhysicalSize::new(660, 640));
            }
        } else {
            if let Some(window) = app.get_webview_window(NOTE_POPUP_LABEL) {
                let _ = window.hide();
            }
            if let Some(window) = app.get_webview_window(CLASSIFY_POPUP_LABEL) {
                let _ = window.set_size(PhysicalSize::new(880, 620));
            }
        }
        // Let an already-visible popup switch its pane layout immediately.
        for label in [CLASSIFY_POPUP_LABEL, NOTE_POPUP_LABEL] {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.emit("popup:configure", ());
            }
        }
    }
    if previous.classify_shortcut != current.classify_shortcut {
        if let Ok(shortcut) = Shortcut::from_str(&previous.classify_shortcut) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        register_classify_shortcut(app, &current.classify_shortcut)?;
    }
    if previous.note_shortcut != current.note_shortcut {
        if let Ok(shortcut) = Shortcut::from_str(&previous.note_shortcut) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        register_note_shortcut(app, &current.note_shortcut)?;
    }
    Ok(())
}

fn register_classify_shortcut(app: &AppHandle, value: &str) -> Result<(), AppError> {
    log_service::info(
        "shortcut.classify",
        format!("registering shortcut: {value}"),
    );
    let shortcut = Shortcut::from_str(value).map_err(|error| {
        log_service::error(
            "shortcut.classify",
            format!("invalid shortcut {value}: {error}"),
        );
        AppError::Validation(format!("classify shortcut is invalid: {error}"))
    })?;
    log_service::debug(
        "shortcut.classify",
        format!("parsed shortcut: {shortcut:?}"),
    );
    let handler = move |app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent| {
        log_service::debug(
            "shortcut.classify",
            format!("event received: {:?}", event.state()),
        );
        if event.state() == ShortcutState::Pressed {
            show_classify_popup(app);
        }
    };
    app.global_shortcut()
        .on_shortcut(shortcut, handler)
        .map_err(|error| {
            log_service::error("shortcut.classify", format!("registration failed: {error}"));
            AppError::Validation(format!("cannot register shortcut: {error}"))
        })?;
    log_service::info("shortcut.classify", "shortcut registered");
    Ok(())
}

fn register_note_shortcut(app: &AppHandle, value: &str) -> Result<(), AppError> {
    log_service::info("shortcut.note", format!("registering shortcut: {value}"));
    let shortcut = Shortcut::from_str(value).map_err(|error| {
        log_service::error(
            "shortcut.note",
            format!("invalid shortcut {value}: {error}"),
        );
        AppError::Validation(format!("note shortcut is invalid: {error}"))
    })?;
    let handler = move |app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent| {
        if event.state() == ShortcutState::Pressed {
            show_note_popup(app);
        }
    };
    app.global_shortcut()
        .on_shortcut(shortcut, handler)
        .map_err(|error| {
            log_service::error("shortcut.note", format!("registration failed: {error}"));
            AppError::Validation(format!("cannot register note shortcut: {error}"))
        })?;
    log_service::info("shortcut.note", "shortcut registered");
    Ok(())
}

fn show_note_popup(app: &AppHandle) {
    if app.state::<PopupWindowMode>().split.load(Ordering::Relaxed) {
        // Split layout: the note shortcut raises the note-only window.
        show_workbench_window(app, NOTE_POPUP_LABEL, "Scene Vault 笔记", 560.0, 640.0);
    } else {
        // Combined layout: both shortcuts share the workbench window. Make
        // sure a note window from an earlier split session stays hidden.
        if let Some(window) = app.get_webview_window(NOTE_POPUP_LABEL) {
            let _ = window.hide();
        }
        show_classify_popup(app);
    }
}

fn show_classify_popup(app: &AppHandle) {
    if app.state::<PopupWindowMode>().split.load(Ordering::Relaxed) {
        show_workbench_window(app, CLASSIFY_POPUP_LABEL, "Scene Vault 分类", 660.0, 640.0);
        return;
    }
    show_workbench_window(app, CLASSIFY_POPUP_LABEL, "Scene Vault 分类", 880.0, 620.0);
}

fn show_workbench_window(app: &AppHandle, label: &str, title: &str, width: f64, height: f64) {
    log_service::debug("shortcut.window", format!("show requested: {label}"));
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => match WebviewWindowBuilder::new(app, label, WebviewUrl::default())
            .title(title)
            .inner_size(width, height)
            .min_inner_size(560.0, 480.0)
            .resizable(true)
            .maximizable(false)
            .minimizable(false)
            .skip_taskbar(true)
            .decorations(false)
            .center()
            .visible(false)
            .build()
        {
            Ok(window) => {
                log_service::info("shortcut.window", format!("recreated popup: {label}"));
                window
            }
            Err(error) => {
                log_service::error(
                    "shortcut.window",
                    format!("could not recreate {label}: {error}"),
                );
                return;
            }
        },
    };
    if !window.is_visible().unwrap_or(false) {
        log_service::debug("shortcut.window", format!("showing hidden popup: {label}"));
    }
    match window.show() {
        Ok(()) => log_service::debug("shortcut.window", format!("showed popup: {label}")),
        Err(error) => log_service::error(
            "shortcut.window",
            format!("could not show {label}: {error}"),
        ),
    }
    match window.set_focus() {
        Ok(()) => log_service::debug("shortcut.window", format!("focused popup: {label}")),
        Err(error) => log_service::warn(
            "shortcut.window",
            format!("could not focus {label}: {error}"),
        ),
    }
    if let Err(error) = window.emit("classify:refresh", ()) {
        log_service::warn(
            "shortcut.window",
            format!("refresh event failed for {label}: {error}"),
        );
    }
    // Popup windows are hidden rather than destroyed. Refresh the note every
    // time either shortcut raises the shared/split workbench so a project
    // selected after the popup's first mount replaces the stale empty state.
    if let Err(error) = window.emit("note:refresh", ()) {
        log_service::warn(
            "shortcut.window",
            format!("note refresh event failed for {label}: {error}"),
        );
    }
    if let Err(error) = window.emit("popup:configure", ()) {
        log_service::warn(
            "shortcut.window",
            format!("configure event failed for {label}: {error}"),
        );
    }
}
