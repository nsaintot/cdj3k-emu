//! A table on a plate: column labels over fixed-height rows that scroll, an
//! optional pinned first row, and items reordered by their drag handle.
//!
//! The table lays out the cells; the caller draws each row's content and
//! handles its clicks.

use egui::{Id, Painter, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};
use egui_dnd::{DragAxis, DragDropItem, Handle};

use super::picker::{hairline, tracked};
use super::theme::{self, Palette};

const HEAD_TRACK: f32 = 1.4;

/// A column's width: a fixed width in points (scaled by `k`), or an equal
/// share of the width the fixed columns leave.
#[derive(Clone, Copy)]
pub(in crate::app) enum Width {
    Fixed(f32),
    Fill,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct Column {
    pub label: &'static str,
    pub width: Width,
}

/// `rect` cut left to right into `widths`, scaled by `k`; the `Fill` ones
/// share the rest evenly.
pub(in crate::app) fn columns(rect: Rect, widths: &[Width], k: f32) -> Vec<Rect> {
    let fixed: f32 = widths
        .iter()
        .map(|w| match w {
            Width::Fixed(px) => px * k,
            Width::Fill => 0.0,
        })
        .sum();
    let fills = widths.iter().filter(|w| matches!(w, Width::Fill)).count();
    let fill = if fills == 0 {
        0.0
    } else {
        ((rect.width() - fixed) / fills as f32).max(0.0)
    };
    let mut x = rect.left();
    widths
        .iter()
        .map(|w| {
            let wide = match w {
                Width::Fixed(px) => px * k,
                Width::Fill => fill,
            };
            let r = Rect::from_x_y_ranges(x..=x + wide, rect.y_range());
            x += wide;
            r
        })
        .collect()
}

/// `n` boxes of `size`, right to left from `cell`'s right edge, centred on
/// its middle.
pub(in crate::app) fn from_right(cell: Rect, size: Vec2, gap: f32, n: usize) -> Vec<Rect> {
    let mut right = cell.right();
    (0..n)
        .map(|_| {
            let r = Rect::from_min_size(
                Pos2::new(right - size.x, cell.center().y - size.y * 0.5),
                size,
            );
            right = r.left() - gap;
            r
        })
        .collect()
}

/// A row being drawn.
pub(in crate::app) struct Row<'u, 'h> {
    pub ui: &'u mut Ui,
    pub rect: Rect,
    /// One per column, inside the row's side padding, the row's full height.
    pub cells: Vec<Rect>,
    /// Being dragged: drawn above the table, and its clicks are ignored.
    pub dragged: bool,
    handle: Option<Handle<'h>>,
}

impl Row<'_, '_> {
    pub fn painter(&self) -> &Painter {
        self.ui.painter()
    }

    /// The drag handle at `rect`; a pinned row has none and draws nothing.
    /// `paint` draws it and is told whether the pointer is over it or holding
    /// it.
    pub fn handle(&mut self, rect: Rect, paint: impl FnOnce(&Painter, Rect, bool)) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let resp = self
            .ui
            .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                handle.ui(ui, |ui| {
                    ui.allocate_exact_size(rect.size(), Sense::hover());
                })
            })
            .inner;
        paint(self.ui.painter(), rect, self.dragged || resp.hovered());
    }
}

/// Which row the caller is asked to draw.
pub(in crate::app) enum Line<'a, T> {
    Pinned,
    Item(&'a T),
}

pub(in crate::app) struct Table<'a> {
    pub id: &'a str,
    pub columns: &'a [Column],
    pub head_h: f32,
    pub row_h: f32,
    /// Left and right of the first and last column.
    pub pad: f32,
    pub k: f32,
}

struct Keyed<'a, T>(Id, usize, &'a T);

impl<T> DragDropItem for Keyed<'_, T> {
    fn id(&self) -> Id {
        self.0
    }
}

impl Table<'_> {
    /// Draw the table into the top of `area`, as tall as its rows or `area`.
    /// `key` identifies an item from one frame to the next. Returns a finished drag as (from,
    /// to), `to` being the item's index once moved.
    pub fn show<T>(
        &self,
        ui: &mut Ui,
        area: Rect,
        pal: &Palette,
        pinned: bool,
        items: &[T],
        key: impl Fn(&T) -> Id,
        mut draw: impl FnMut(&mut Row, Line<'_, T>),
    ) -> Option<(usize, usize)> {
        let k = self.k;
        let (head_h, row_h) = (self.head_h * k, self.row_h * k);
        let content_h = head_h + row_h * (usize::from(pinned) + items.len()) as f32;
        let table = Rect::from_min_size(
            area.min,
            Vec2::new(area.width(), content_h.min(area.height())),
        );
        let round = theme::ROUND * k;
        let p = ui.painter().clone();
        p.rect_filled(table, round, pal.plate);

        let head = Rect::from_min_size(table.min, Vec2::new(table.width(), head_h));
        p.rect_filled(
            head,
            egui::epaint::CornerRadiusF32 {
                nw: round,
                ne: round,
                sw: 0.0,
                se: 0.0,
            },
            pal.field,
        );
        hairline(&p, head.left_bottom(), head.right_bottom(), pal.line, k);
        let widths: Vec<Width> = self.columns.iter().map(|c| c.width).collect();
        let inner = |r: Rect| columns(r.shrink2(Vec2::new(self.pad * k, 0.0)), &widths, k);
        let micro = egui::FontId::proportional(theme::MICRO_FONT * k);
        for (col, cell) in self.columns.iter().zip(inner(head)) {
            tracked(
                &p,
                cell.left_center(),
                col.label,
                &micro,
                pal.dim,
                HEAD_TRACK * k,
            );
        }

        let body = Rect::from_min_max(Pos2::new(table.left(), head.bottom()), table.max);
        let width = body.width();
        // One row: its plate, the rule above it, then the caller's content.
        let mut row = |ui: &mut Ui,
                       first: bool,
                       dragged: bool,
                       handle: Option<Handle<'_>>,
                       line: Line<'_, T>| {
            let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(width, row_h));
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.set_min_size(rect.size());
                let p = ui.painter();
                p.rect_filled(rect, 0.0, pal.plate);
                if dragged {
                    p.rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(theme::HAIRLINE * k, pal.line_strong),
                        egui::StrokeKind::Inside,
                    );
                } else if !first {
                    hairline(p, rect.left_top(), rect.right_top(), pal.line, k);
                }
                let mut r = Row {
                    ui,
                    rect,
                    cells: inner(rect),
                    dragged,
                    handle,
                };
                draw(&mut r, line);
            });
        };

        let mut moved = None;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body);
            // Inset from the plate's edge and visible without hover: otherwise
            // a half-cut row is the only sign of more rows below.
            let scroll = &mut ui.spacing_mut().scroll;
            scroll.bar_outer_margin = 4.0 * k;
            scroll.dormant_handle_opacity = 0.6;
            egui::ScrollArea::vertical()
                .id_salt(self.id)
                .max_height(body.height())
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    if pinned {
                        row(ui, true, false, None, Line::Pinned);
                    }
                    let keyed = items.iter().enumerate().map(|(i, t)| Keyed(key(t), i, t));
                    let resp = egui_dnd::dnd(ui, self.id)
                        .with_drag_axis(DragAxis::Vertical)
                        .show_sized(keyed, Vec2::new(width, row_h), |ui, item, handle, state| {
                            let first = !pinned && item.1 == 0;
                            row(ui, first, state.dragged, Some(handle), Line::Item(item.2));
                        });
                    moved = resp
                        .final_update()
                        .map(|u| (u.from, if u.to > u.from { u.to - 1 } else { u.to }))
                        .filter(|(from, to)| from != to);
                });
        });
        p.rect_stroke(
            table,
            round,
            Stroke::new(theme::HAIRLINE * k, pal.line),
            egui::StrokeKind::Middle,
        );
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_columns_scale_and_fill_takes_the_rest() {
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 10.0));
        let c = columns(
            r,
            &[Width::Fixed(60.0), Width::Fill, Width::Fixed(40.0)],
            2.0,
        );
        assert_eq!((c[0].left(), c[0].right()), (0.0, 120.0));
        assert_eq!((c[1].left(), c[1].right()), (120.0, 420.0));
        assert_eq!((c[2].left(), c[2].right()), (420.0, 500.0));
    }

    #[test]
    fn fills_share_evenly() {
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 10.0));
        let c = columns(r, &[Width::Fill; 4], 1.0);
        assert!(c.iter().all(|c| c.width() == 100.0));
        assert_eq!(c[3].right(), 400.0);
    }

    #[test]
    fn boxes_from_the_right() {
        let cell = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 40.0));
        let b = from_right(cell, Vec2::new(20.0, 10.0), 5.0, 2);
        assert_eq!(
            b[0],
            Rect::from_min_size(Pos2::new(80.0, 15.0), Vec2::new(20.0, 10.0))
        );
        assert_eq!(b[1].right(), 75.0);
    }
}
