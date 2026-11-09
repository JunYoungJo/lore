use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const TRAY_ID: &str = "qmd-memory";
pub const POPOVER: &str = "tray";
const PAUSE_ITEM: &str = "pause";
const POPOVER_W: f64 = 400.0;
const POPOVER_H: f64 = 560.0;
// Right after showing, the tray click has not settled focus yet and a spurious blur arrives.
const SHOW_GRACE_MS: u64 = 350;
// The tray click that follows a blur-hide must not reopen the popover it just closed.
const REOPEN_GUARD_MS: u64 = 250;

pub struct PauseItem(pub CheckMenuItem<tauri::Wry>);

static LAST_SHOWN_MS: AtomicU64 = AtomicU64::new(0);
static LAST_HIDDEN_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or(0)
}

fn pick_monitor(window: &WebviewWindow, cursor: Option<PhysicalPosition<f64>>) -> Option<Monitor> {
    cursor
        .and_then(|cursor| {
            let (x, y) = (cursor.x as i32, cursor.y as i32);
            window.available_monitors().ok().and_then(|monitors| {
                monitors.into_iter().find(|monitor| {
                    let position = monitor.position();
                    let size = monitor.size();
                    x >= position.x && x < position.x + size.width as i32 && y >= position.y && y < position.y + size.height as i32
                })
            })
        })
        .or_else(|| window.primary_monitor().ok().flatten())
}

fn anchor_popover(window: &WebviewWindow, monitor: &Monitor, width: i32, height: i32) {
    let area = monitor.work_area();
    let right = area.position.x + area.size.width as i32;
    let bottom = area.position.y + area.size.height as i32;
    let x = (right - width - 2).max(area.position.x);
    let y = if cfg!(target_os = "macos") { area.position.y + 2 } else { (bottom - height - 2).max(area.position.y) };
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn show_popover(window: &WebviewWindow, cursor: Option<PhysicalPosition<f64>>) {
    if let Some(monitor) = pick_monitor(window, cursor) {
        let scale = monitor.scale_factor();
        anchor_popover(window, &monitor, (POPOVER_W * scale).round() as i32, (POPOVER_H * scale).round() as i32);
    }
    LAST_SHOWN_MS.store(now_ms(), Ordering::Relaxed);
    let _ = window.show();
    // outer_size() only reflects the monitor's DPI after the first show, so anchor again.
    if let (Some(monitor), Ok(size)) = (pick_monitor(window, cursor), window.outer_size()) {
        anchor_popover(window, &monitor, size.width as i32, size.height as i32);
    }
    let _ = window.set_focus();
    let _ = window.emit("popover-shown", ());
}

pub fn toggle_popover(app: &AppHandle, cursor: Option<PhysicalPosition<f64>>) {
    let Some(window) = app.get_webview_window(POPOVER) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        LAST_HIDDEN_MS.store(now_ms(), Ordering::Relaxed);
    } else if now_ms().saturating_sub(LAST_HIDDEN_MS.load(Ordering::Relaxed)) >= REOPEN_GUARD_MS {
        show_popover(&window, cursor);
    }
}

pub fn open_popover(app: &AppHandle) {
    let Some(window) = app.get_webview_window(POPOVER) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.set_focus();
    } else {
        show_popover(&window, None);
    }
}

#[cfg(windows)]
fn harden_webview(window: &WebviewWindow) {
    let _ = window.with_webview(|webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let Ok(settings) = core.Settings() else { return };
        let _ = settings.SetIsZoomControlEnabled(false);
        let _ = settings.SetAreDefaultContextMenusEnabled(false);
    });
}

#[cfg(not(windows))]
fn harden_webview(_window: &WebviewWindow) {}

// Built hidden during setup: building a window inside a command deadlocks the event loop,
// and the popover should already hold the current status when it is first opened.
pub fn create_popover(app: &AppHandle) {
    let built = WebviewWindowBuilder::new(app, POPOVER, WebviewUrl::App("index.html".into()))
        .title("qmd-memory")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(POPOVER_W, POPOVER_H)
        .build();
    match built {
        Ok(window) => {
            harden_webview(&window);
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    if now_ms().saturating_sub(LAST_SHOWN_MS.load(Ordering::Relaxed)) >= SHOW_GRACE_MS {
                        let _ = handle.hide();
                        LAST_HIDDEN_MS.store(now_ms(), Ordering::Relaxed);
                    }
                }
            });
        }
        Err(error) => eprintln!("트레이 창을 만들지 못했습니다: {error}"),
    }
}

pub fn set_paused(app: &AppHandle, paused: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings();
    settings.paused = paused;
    qmd_memory_core::settings::save(&state.paths.settings(), &settings).map_err(|error| error.to_string())?;
    state.replace_settings(settings);
    show_paused(app, paused);
    refresh_tooltip(app);
    let _ = app.emit("state-changed", ());
    Ok(())
}

pub fn show_paused(app: &AppHandle, paused: bool) {
    if let Some(item) = app.try_state::<PauseItem>() {
        let _ = item.0.set_checked(paused);
    }
}

pub fn refresh_tooltip(app: &AppHandle) {
    let state = app.state::<AppState>();
    let text = match (state.current(), state.reviewing(), state.settings().paused) {
        (Some(current), _, _) => format!("qmd-memory · 기록 중: {}", current.project),
        (None, true, _) => "qmd-memory · 검토 중이라 기록을 멈춤".to_string(),
        (None, false, true) => "qmd-memory · 일시 중지".to_string(),
        (None, false, false) => "qmd-memory · 대기 중".to_string(),
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let paused = app.state::<AppState>().settings().paused;
    let open = MenuItem::with_id(app, "open", "열기", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, PAUSE_ITEM, "일시 중지", true, paused, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &quit])?;
    let pause_item = pause.clone();
    app.manage(PauseItem(pause));
    let mut builder = TrayIconBuilder::with_id(TRAY_ID).tooltip("qmd-memory");
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => open_popover(app),
            PAUSE_ITEM => {
                let paused = pause_item.is_checked().unwrap_or(false);
                if let Err(error) = set_paused(app, paused) {
                    eprintln!("일시 중지 설정을 저장하지 못했습니다: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = event {
                toggle_popover(tray.app_handle(), Some(position));
            }
        })
        .build(app)?;
    refresh_tooltip(app);
    Ok(())
}
