use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::webview::WebviewWindowBuilder;
use tauri::PhysicalSize;
use tauri::WebviewUrl;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::{error::AppError, models::app_settings::AppSettings};

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
    eprintln!("[shortcut] registering classify shortcut: {value}");
    let shortcut = Shortcut::from_str(value).map_err(|error| {
        eprintln!("[shortcut] invalid shortcut {value}: {error}");
        AppError::Validation(format!("classify shortcut is invalid: {error}"))
    })?;
    eprintln!("[shortcut] parsed: {shortcut:?}");
    let handler = move |app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent| {
        eprintln!("[shortcut] event received, state={:?}", event.state());
        if event.state() == ShortcutState::Pressed {
            show_classify_popup(app);
        }
    };
    app.global_shortcut()
        .on_shortcut(shortcut, handler)
        .map_err(|error| {
            eprintln!("[shortcut] register failed: {error}");
            AppError::Validation(format!("cannot register shortcut: {error}"))
        })?;
    eprintln!("[shortcut] registered ok");
    Ok(())
}

fn register_note_shortcut(app: &AppHandle, value: &str) -> Result<(), AppError> {
    eprintln!("[shortcut] registering note shortcut: {value}");
    let shortcut = Shortcut::from_str(value).map_err(|error| {
        eprintln!("[shortcut] invalid note shortcut {value}: {error}");
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
            eprintln!("[shortcut] note register failed: {error}");
            AppError::Validation(format!("cannot register note shortcut: {error}"))
        })?;
    eprintln!("[shortcut] note registered ok");
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
    eprintln!("[shortcut] show_workbench_window called: {label}");
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
                eprintln!("[shortcut] popup window recreated on demand");
                window
            }
            Err(error) => {
                eprintln!("[shortcut] popup window recreate failed: {error}");
                return;
            }
        },
    };
    if !window.is_visible().unwrap_or(false) {
        eprintln!("[shortcut] popup window hidden, showing");
    }
    match window.show() {
        Ok(()) => eprintln!("[shortcut] show ok"),
        Err(error) => eprintln!("[shortcut] show failed: {error}"),
    }
    match window.set_focus() {
        Ok(()) => eprintln!("[shortcut] focus ok"),
        Err(error) => eprintln!("[shortcut] focus failed: {error}"),
    }
    if let Err(error) = window.emit("classify:refresh", ()) {
        eprintln!("[shortcut] emit failed: {error}");
    }
    if let Err(error) = window.emit("popup:configure", ()) {
        eprintln!("[shortcut] configure emit failed: {error}");
    }
}
