use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Manager, PhysicalPosition, PhysicalSize};

#[derive(Clone, Serialize, Deserialize)]
struct Geometry {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    maximized: bool,
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let window = app.get_webview_window("main").expect("main window");
    let path = app.path().app_config_dir()?.join(".window-state.json");
    let saved = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| serde_json::from_value::<Geometry>(value["main"].clone()).ok())
        .filter(|g| g.width >= 760 && g.height >= 540 && g.width <= 16384 && g.height <= 16384);
    if let Some(g) = &saved {
        window.set_size(PhysicalSize::new(g.width, g.height))?;
        let on_screen = window.available_monitors()?.iter().any(|m| {
            let p = m.position();
            let s = m.size();
            g.x as i64 >= p.x as i64
                && g.x as i64 + 80 < p.x as i64 + s.width as i64
                && g.y as i64 >= p.y as i64
                && g.y as i64 + 40 < p.y as i64 + s.height as i64
        });
        if on_screen {
            window.set_position(PhysicalPosition::new(g.x, g.y))?;
        } else {
            window.center()?;
        }
    }
    let size = window.inner_size()?;
    let pos = window.outer_position()?;
    let mut initial = Geometry {
        width: size.width,
        height: size.height,
        x: pos.x,
        y: pos.y,
        maximized: false,
    };
    if let Some(g) = saved {
        initial.maximized = g.maximized;
    }
    let geometry = Arc::new(Mutex::new(initial.clone()));
    if initial.maximized {
        window.maximize()?;
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let tracked = window.clone();
    let cache = geometry.clone();
    let done = stopped.clone();
    // Sample settled native geometry. Maximize/minimize can emit intermediate move
    // events that must never replace the last normal window rectangle.
    std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(150));
            if !tracked.is_visible().unwrap_or(false) || tracked.is_minimized().unwrap_or(true) {
                continue;
            }
            if let Ok(maximized) = tracked.is_maximized() {
                let mut g = cache.lock().unwrap();
                g.maximized = maximized;
                if !maximized {
                    if let (Ok(s), Ok(p)) = (tracked.inner_size(), tracked.outer_position()) {
                        g.width = s.width;
                        g.height = s.height;
                        g.x = p.x;
                        g.y = p.y;
                    }
                }
            }
        }
    });
    window.clone().on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            stopped.store(true, Ordering::Relaxed);
            let mut g = geometry.lock().unwrap().clone();
            if !window.is_minimized().unwrap_or(true) {
                g.maximized = window.is_maximized().unwrap_or(g.maximized);
                if !g.maximized {
                    if let (Ok(s), Ok(p)) = (window.inner_size(), window.outer_position()) {
                        g.width = s.width;
                        g.height = s.height;
                        g.x = p.x;
                        g.y = p.y;
                    }
                }
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(bytes) = serde_json::to_vec_pretty(&serde_json::json!({"main":g})) {
                let _ = std::fs::write(&path, bytes);
            }
        }
    });
    Ok(())
}
