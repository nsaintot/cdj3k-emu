//! Where a panel window with no saved frame goes: the default size, scaled
//! down to fit the monitor, centred where the picker was.
//!
//! Compiled everywhere so it is tested everywhere.

use egui::{Pos2, Rect, Vec2};

/// The share of the monitor a first panel window may take, leaving room for
/// the title bar and a taskbar or dock.
const MONITOR_SHARE: f32 = 0.85;

/// The inner size for a panel of `panel` (the chassis, aspect to keep) plus
/// the menu strip `bar`, shrunk to fit `monitor` and never below `min`.
pub fn fit_to_monitor(panel: Vec2, bar: f32, min: Vec2, monitor: Option<Vec2>) -> Vec2 {
    let scale = monitor.map_or(1.0, |m| {
        let room = m * MONITOR_SHARE - Vec2::new(0.0, bar);
        (room.x / panel.x).min(room.y / panel.y).min(1.0)
    });
    Vec2::new(panel.x * scale, panel.y * scale + bar).max(min)
}

/// The outer position that puts a window of inner size `inner` on the centre
/// of the window it replaces (`outer` and `inner_now`, its outer and inner
/// rectangles), with the same decorations around it.
pub fn centred_on(outer: Rect, inner_now: Rect, inner: Vec2) -> Pos2 {
    let decorations = outer.size() - inner_now.size();
    outer.center() - (inner + decorations) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_that_fits_keeps_its_size() {
        let size = fit_to_monitor(Vec2::new(1000.0, 800.0), 30.0, Vec2::ZERO, Some(Vec2::new(2560.0, 1440.0)));
        assert_eq!(size, Vec2::new(1000.0, 830.0));
        let size = fit_to_monitor(Vec2::new(1000.0, 800.0), 30.0, Vec2::ZERO, None);
        assert_eq!(size, Vec2::new(1000.0, 830.0));
    }

    #[test]
    fn a_panel_too_tall_shrinks_with_its_aspect() {
        let monitor = Vec2::new(1323.0, 891.0);
        let size = fit_to_monitor(Vec2::new(1000.0, 1300.0), 30.0, Vec2::ZERO, Some(monitor));
        assert!(size.y <= monitor.y * MONITOR_SHARE + 0.01, "{size:?}");
        assert!(((size.y - 30.0) / size.x - 1.3).abs() < 1e-4, "{size:?}");
    }

    #[test]
    fn never_below_the_minimum() {
        let min = Vec2::new(200.0, 290.0);
        let size = fit_to_monitor(Vec2::new(1000.0, 1300.0), 30.0, min, Some(Vec2::new(300.0, 200.0)));
        assert_eq!(size, min);
    }

    #[test]
    fn centred_on_the_old_window_with_its_decorations() {
        let outer = Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(762.0, 631.0));
        let inner = Rect::from_min_size(Pos2::new(101.0, 80.0), Vec2::new(760.0, 600.0));
        let pos = centred_on(outer, inner, Vec2::new(500.0, 400.0));
        assert_eq!(pos, Pos2::new(100.0 + (762.0 - 502.0) / 2.0, 50.0 + (631.0 - 431.0) / 2.0));
    }
}
