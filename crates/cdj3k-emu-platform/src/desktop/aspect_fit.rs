//! Holding a dragged window's outer rectangle to the chassis aspect, for a
//! window system that offers the proposed rectangle to edit (`WM_SIZING`).
//!
//! Compiled everywhere so it is tested everywhere.

/// The chassis ratio, with the in-window menu strip as a base height outside
/// it. Width and height are the reference canvas at 16× so a fractional
/// canvas survives; the base height is in the window's own pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub width: u32,
    pub height: u32,
    pub base_height: u32,
}

/// A rectangle in window coordinates, `RECT`-shaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The edge or corner being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    TopLeft,
    TopRight,
    Bottom,
    BottomLeft,
    BottomRight,
}

impl Edge {
    /// From a `WM_SIZING` `wParam` (`WMSZ_LEFT` = 1 through `WMSZ_BOTTOMRIGHT` = 8).
    pub fn from_wmsz(v: usize) -> Option<Self> {
        Some(match v {
            1 => Self::Left,
            2 => Self::Right,
            3 => Self::Top,
            4 => Self::TopLeft,
            5 => Self::TopRight,
            6 => Self::Bottom,
            7 => Self::BottomLeft,
            8 => Self::BottomRight,
            _ => return None,
        })
    }
}

/// `rect` reshaped to the ratio. `chrome` is the window's non-client
/// `(width, height)`: the aspect holds for the client area, below the strip.
///
/// A horizontal drag or a corner gives the width priority and moves the
/// vertical edge opposite the anchored one; a vertical drag gives the height
/// priority and moves the right edge.
pub fn fit(edge: Edge, rect: Rect, chrome: (i32, i32), ratio: Ratio) -> Rect {
    if ratio.width == 0 || ratio.height == 0 {
        return rect;
    }
    let client_w = rect.right - rect.left - chrome.0;
    let client_h = rect.bottom - rect.top - chrome.1;
    let base = ratio.base_height as i64;
    if client_w <= 0 || i64::from(client_h) <= base {
        return rect;
    }
    let (w, h) = (i64::from(ratio.width), i64::from(ratio.height));
    match edge {
        Edge::Top | Edge::Bottom => {
            let content = i64::from(client_h) - base;
            let new_client_w = (content * w + h / 2) / h;
            Rect {
                right: rect.left + new_client_w as i32 + chrome.0,
                ..rect
            }
        }
        _ => {
            let new_client_h = (i64::from(client_w) * h + w / 2) / w + base;
            let outer_h = new_client_h as i32 + chrome.1;
            match edge {
                Edge::TopLeft | Edge::TopRight => Rect {
                    top: rect.bottom - outer_h,
                    ..rect
                },
                _ => Rect {
                    bottom: rect.top + outer_h,
                    ..rect
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4:3 under a 44 px strip, in a frame with 16x39 px of chrome.
    const RATIO: Ratio = Ratio {
        width: 64,
        height: 48,
        base_height: 44,
    };
    const CHROME: (i32, i32) = (16, 39);

    fn rect(w: i32, h: i32) -> Rect {
        Rect {
            left: 100,
            top: 50,
            right: 100 + w,
            bottom: 50 + h,
        }
    }

    fn client(r: Rect) -> (i32, i32) {
        (r.right - r.left - CHROME.0, r.bottom - r.top - CHROME.1)
    }

    #[test]
    fn a_right_drag_takes_its_height_from_the_width() {
        let got = fit(Edge::Right, rect(816, 900), CHROME, RATIO);
        assert_eq!(client(got), (800, 600 + 44));
        assert_eq!((got.left, got.top, got.right), (100, 50, 916));
    }

    #[test]
    fn a_bottom_drag_takes_its_width_from_the_height() {
        let got = fit(Edge::Bottom, rect(2000, 39 + 644), CHROME, RATIO);
        assert_eq!(client(got), (800, 644));
        assert_eq!((got.left, got.top, got.bottom), (100, 50, 50 + 683));
    }

    #[test]
    fn a_top_corner_keeps_the_bottom_edge_where_it_is() {
        let got = fit(Edge::TopLeft, rect(816, 900), CHROME, RATIO);
        assert_eq!(got.bottom, 950);
        assert_eq!(client(got), (800, 644));
    }

    #[test]
    fn a_fitted_rect_fits_as_itself() {
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom, Edge::BottomRight] {
            for w in (300..1500).step_by(37) {
                let once = fit(edge, rect(w, 777), CHROME, RATIO);
                assert_eq!(fit(edge, once, CHROME, RATIO), once, "{edge:?} {w}");
            }
        }
    }

    #[test]
    fn nothing_to_fit_leaves_the_offer_alone() {
        let r = rect(10, 10);
        assert_eq!(fit(Edge::Right, r, CHROME, RATIO), r);
        let none = Ratio { width: 0, ..RATIO };
        assert_eq!(fit(Edge::Right, rect(800, 600), CHROME, none), rect(800, 600));
    }

    #[test]
    fn wmsz_values_map_and_the_rest_do_not() {
        assert_eq!(Edge::from_wmsz(1), Some(Edge::Left));
        assert_eq!(Edge::from_wmsz(8), Some(Edge::BottomRight));
        assert_eq!(Edge::from_wmsz(0), None);
        assert_eq!(Edge::from_wmsz(9), None);
    }
}
