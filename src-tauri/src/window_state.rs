//! Persists and restores the main window's geometry.
//!
//! Geometry is saved (throttled) while the window moves or resizes and always
//! on close. On start it is restored only if it would still be visible on a
//! connected monitor; otherwise the window keeps its default centred position,
//! so unplugging a monitor can't strand the window off-screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nivesh_store::{Store, WindowState};
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

/// Minimum gap between saves caused by move/resize events.
const SAVE_INTERVAL_MS: u64 = 500;

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Whether at least `min` pixels of the window's title area (its top strip)
/// lie inside some monitor, so the user can still grab and move it.
pub fn is_reachable(window: Rect, monitors: &[Rect]) -> bool {
    const MIN_VISIBLE: i64 = 100;
    let (wx0, wx1) = (
        i64::from(window.x),
        i64::from(window.x) + i64::from(window.width),
    );
    let wy = i64::from(window.y);
    monitors.iter().any(|m| {
        let (mx0, mx1) = (i64::from(m.x), i64::from(m.x) + i64::from(m.width));
        let (my0, my1) = (i64::from(m.y), i64::from(m.y) + i64::from(m.height));
        let overlap = wx1.min(mx1) - wx0.max(mx0);
        overlap >= MIN_VISIBLE && wy >= my0 && wy < my1
    })
}

/// Restores saved geometry onto `window`. Call before showing it.
pub fn restore(window: &WebviewWindow, store: &Store) {
    let saved = match store.window_state() {
        Ok(Some(saved)) => saved,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(%err, "could not read saved window state");
            return;
        }
    };
    let monitors: Vec<Rect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Rect {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
        })
        .collect();
    let rect = Rect {
        x: saved.x,
        y: saved.y,
        width: saved.width,
        height: saved.height,
    };
    if !monitors.is_empty() && is_reachable(rect, &monitors) {
        // Best effort: a failure leaves the default geometry.
        let _ = window.set_size(PhysicalSize::new(saved.width, saved.height));
        let _ = window.set_position(PhysicalPosition::new(saved.x, saved.y));
    }
    if saved.maximized {
        let _ = window.maximize();
    }
}

/// Saves geometry as the window changes.
pub fn track(window: &WebviewWindow, store: Arc<Store>) {
    let target = window.clone();
    let last_save = AtomicU64::new(0);
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            let now = now_ms();
            let prev = last_save.load(Ordering::Relaxed);
            if now.saturating_sub(prev) >= SAVE_INTERVAL_MS {
                last_save.store(now, Ordering::Relaxed);
                save(&target, &store);
            }
        }
        WindowEvent::CloseRequested { .. } => save(&target, &store),
        _ => {}
    });
}

fn save(window: &WebviewWindow, store: &Store) {
    // A minimised window reports meaningless geometry.
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let previous = store.window_state().ok().flatten();
    let state = if maximized {
        // Keep the un-maximised geometry so un-maximising after a restart
        // returns to where the user left it.
        match previous {
            Some(p) => WindowState {
                maximized: true,
                ..p
            },
            None => match current_geometry(window, true) {
                Some(s) => s,
                None => return,
            },
        }
    } else {
        match current_geometry(window, false) {
            Some(s) => s,
            None => return,
        }
    };
    if let Err(err) = store.set_window_state(state) {
        tracing::warn!(%err, "could not save window state");
    }
}

fn current_geometry(window: &WebviewWindow, maximized: bool) -> Option<WindowState> {
    let pos = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    (size.width > 0 && size.height > 0).then_some(WindowState {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const MON2: Rect = Rect {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1440,
    };

    fn win(x: i32, y: i32) -> Rect {
        Rect {
            x,
            y,
            width: 1440,
            height: 900,
        }
    }

    #[test]
    fn on_screen_windows_are_reachable() {
        assert!(is_reachable(win(100, 50), &[MON]));
        assert!(is_reachable(win(-1340, 50), &[MON])); // 100px still showing
        assert!(is_reachable(win(1800, 50), &[MON, MON2]));
    }

    #[test]
    fn off_screen_windows_are_not() {
        assert!(!is_reachable(win(-1400, 50), &[MON])); // only 40px showing
        assert!(!is_reachable(win(1900, 50), &[MON])); // 20px showing
        assert!(!is_reachable(win(100, 5000), &[MON])); // title bar below screen
        assert!(!is_reachable(win(100, -200), &[MON])); // title bar above screen
        assert!(!is_reachable(win(100, 50), &[])); // no monitors
    }

    #[test]
    fn unplugged_second_monitor() {
        let on_second = win(2500, 100);
        assert!(is_reachable(on_second, &[MON, MON2]));
        assert!(!is_reachable(on_second, &[MON]));
    }
}
