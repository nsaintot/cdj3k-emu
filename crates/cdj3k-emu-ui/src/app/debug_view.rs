//! The debug window's content: a read-only view of the panel and of the guest,
//! in two tabs.
//!
//! The debug window is a deferred viewport with its own update cycle, so it
//! cannot borrow [`CdjApp`]. Each main `update()` writes a [`DebugSnapshot`]
//! into a shared mutex, and the shell writes the [`GuestView`] it samples; the
//! window reads both.

mod guest;

use cdj3k_emu_panel::miso_frame::MISO_SIZE;
use cdj3k_emu_panel::{Lamp, LampState, Model, StepLed};
use egui::{Color32, RichText};

pub(crate) use guest::{GuestStats, GuestView, Memory};

use super::CdjApp;

/// The tab the debug window shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DebugTab {
    #[default]
    Panel,
    Guest,
}

/// The parts of [`CdjApp`] state that the Panel tab shows.
#[derive(Clone)]
pub struct DebugSnapshot {
    /// Whether the main and jog LCD streams are up.
    pub screens: String,
    /// The main viewport's frame rate, shown next to the debug window's own.
    pub main_fps: f32,
    pub main_shape_count: u64,
    pub model: Model,
    /// The buttons held down by the mouse, a Ctrl latch or the control API.
    pub pressed: Vec<String>,
    /// The frame bits the control API forces low.
    pub cleared: Vec<String>,
    pub lcd_touch: Option<(u16, u16)>,
    pub tempo: f32,
    pub rotary: u16,
    pub beat_loop: u16,
    pub direction: String,
    pub vinyl: u8,
    pub jog: JogView,
    pub last_miso: [u8; MISO_SIZE],
    pub led_frame: [u8; 64],
}

/// The jog's encoded state, its motion, the input driving it and its brake.
#[derive(Clone, Copy, Default)]
pub struct JogView {
    pub pos: u16,
    pub vel: u16,
    pub touch: u8,
    pub rps: f32,
    pub omega: f32,
    /// What fed the last sample: drag, scroll or none.
    pub source: &'static str,
    pub dragging: bool,
    pub grip: bool,
    /// Whether the encoder reports a hand on the top plate.
    pub hand_on_top: bool,
    pub scroll_hold_s: f32,
    pub sample_rad: f32,
    pub sample_dt: f32,
    pub sample_omega: f32,
    pub brake: f32,
    pub adjust: f32,
}

impl Default for DebugSnapshot {
    fn default() -> Self {
        Self {
            screens: String::new(),
            main_fps: 60.0,
            main_shape_count: 0,
            model: Model::default(),
            pressed: Vec::new(),
            cleared: Vec::new(),
            lcd_touch: None,
            tempo: 0.5,
            rotary: 0,
            beat_loop: 0,
            direction: String::new(),
            vinyl: 0,
            jog: JogView::default(),
            last_miso: [0; MISO_SIZE],
            led_frame: [0; 64],
        }
    }
}

impl CdjApp {
    /// A fresh [`DebugSnapshot`] of the current state.
    pub(super) fn debug_snapshot(&self) -> DebugSnapshot {
        let state = self.panel_state();
        DebugSnapshot {
            screens: self.status.clone(),
            main_fps: self.fps_smooth,
            main_shape_count: self.frame_shape_count,
            model: self.model,
            pressed: state.pressed.iter().map(|b| self.bit_name(*b)).collect(),
            cleared: state.cleared.iter().map(|b| self.bit_name(*b)).collect(),
            lcd_touch: self.lcd_touch,
            tempo: self.tempo,
            rotary: self.rotary,
            beat_loop: self.beat_loop,
            direction: format!("{:?}", self.direction),
            vinyl: self.vinyl_speed,
            jog: JogView {
                pos: self.jog_pos,
                vel: self.jog_vel,
                touch: self.jog_touch,
                rps: self.jog_rps,
                omega: self.jog_omega,
                source: self.debug.jog_source,
                dragging: self.jog_is_dragging(),
                grip: self.jog_grip_drag,
                hand_on_top: self.jog_touch_active,
                scroll_hold_s: self.jog_scroll_hold_sec,
                sample_rad: self.debug.jog_delta_rad,
                sample_dt: self.debug.jog_dt,
                sample_omega: self.debug.jog_omega_sample,
                brake: self.debug.jog_brake,
                adjust: self.jog_adjust,
            },
            last_miso: self.last_miso,
            led_frame: self.led_state.frame,
        }
    }
}

/// Draw the debug window: the tab bar, then the chosen tab.
pub(super) fn draw(
    ui: &mut egui::Ui,
    tab: &mut DebugTab,
    snap: &DebugSnapshot,
    guest: &GuestView,
    journal_filter: &mut String,
    local_fps: f32,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, DebugTab::Panel, "Panel / egui");
        ui.selectable_value(tab, DebugTab::Guest, "Guest");
    });
    ui.separator();
    match tab {
        DebugTab::Panel => {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Each block takes its own width, side by side.
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| draw_egui(ui, snap, local_fps));
                    ui.add_space(BLOCK_GAP);
                    ui.vertical(|ui| draw_inputs(ui, snap));
                    ui.add_space(BLOCK_GAP);
                    ui.vertical(|ui| {
                        egui::CollapsingHeader::new("Jog")
                            .default_open(true)
                            .show(ui, |ui| draw_jog(ui, snap));
                    });
                });
                draw_frames(ui, snap);
            });
        }
        DebugTab::Guest => guest::draw(ui, guest, journal_filter),
    }
}

/// The space between blocks laid out side by side.
pub(super) const BLOCK_GAP: f32 = 28.0;

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace()
}

/// A two-column grid of name and value rows.
fn rows(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        for (name, value) in rows {
            ui.label(*name);
            ui.label(mono(value.as_str()));
            ui.end_row();
        }
    });
}

fn names(list: &[String]) -> String {
    if list.is_empty() {
        "none".into()
    } else {
        list.join(" ")
    }
}

fn draw_egui(ui: &mut egui::Ui, snap: &DebugSnapshot, local_fps: f32) {
    egui::CollapsingHeader::new("egui")
        .default_open(true)
        .show(ui, |ui| {
            rows(
                ui,
                "egui",
                &[
                    ("Screens", snap.screens.clone()),
                    ("Main window", format!("{:.1} fps", snap.main_fps)),
                    ("Debug window", format!("{local_fps:.1} fps")),
                    ("Shapes per frame", snap.main_shape_count.to_string()),
                ],
            );
        });
}

fn draw_inputs(ui: &mut egui::Ui, snap: &DebugSnapshot) {
    egui::CollapsingHeader::new("Inputs")
        .default_open(true)
        .show(ui, |ui| {
            let touch = snap.lcd_touch.map_or("none".into(), |(x, y)| {
                format!("{} {}", x & !cdj3k_emu_panel::TOUCH_DOWN, y)
            });
            rows(
                ui,
                "inputs",
                &[
                    ("Held", names(&snap.pressed)),
                    ("Forced low", names(&snap.cleared)),
                    ("LCD touch", touch),
                    ("Tempo", format!("{:.3}", snap.tempo)),
                    ("Rotary", snap.rotary.to_string()),
                    ("Beat loop", snap.beat_loop.to_string()),
                    ("Direction", snap.direction.clone()),
                    ("Vinyl speed", snap.vinyl.to_string()),
                ],
            );
        });
}

/// The lamps and the raw frames, across the window's width.
fn draw_frames(ui: &mut egui::Ui, snap: &DebugSnapshot) {
    egui::CollapsingHeader::new("Lamps")
        .default_open(true)
        .show(ui, |ui| draw_lamps(ui, snap));
    egui::CollapsingHeader::new("Raw frames")
        .default_open(false)
        .show(ui, |ui| {
            hex_dump(ui, "MISO, to the guest", &snap.last_miso);
            hex_dump(ui, "MOSI, from the guest", &snap.led_frame);
        });
}

fn draw_jog(ui: &mut egui::Ui, snap: &DebugSnapshot) {
    let jog = snap.jog;
    let vel = if jog.vel == u16::MAX {
        "stopped".to_string()
    } else {
        jog.vel.to_string()
    };
    let mut held = Vec::new();
    if jog.hand_on_top {
        held.push("hand on top".to_string());
    }
    if jog.dragging {
        held.push("dragging".into());
    }
    if jog.grip {
        held.push("by the side".into());
    }
    if jog.scroll_hold_s > 0.0 {
        held.push(format!("scroll hold {:.2} s", jog.scroll_hold_s));
    }
    rows(
        ui,
        "jog",
        &[
            ("Position", jog.pos.to_string()),
            ("Velocity word", vel),
            (
                "Touch byte",
                format!("{:#04x} {}", jog.touch, touch_meaning(jog.touch)),
            ),
            (
                "Speed",
                format!("{:+.3} turns/s ({:+.2} rad/s)", jog.rps, jog.omega),
            ),
            ("Input", names(&held)),
            ("Last sample", last_sample(&jog)),
            (
                "Brake",
                format!("{:.2} rad/s², adjust {:.2}", jog.brake, jog.adjust),
            ),
        ],
    );
}

/// The last jog input sample, or "none" before any input.
fn last_sample(jog: &JogView) -> String {
    if jog.source == "none" {
        return "none".into();
    }
    format!(
        "{} {:+.4} rad / {:.1} ms ({:+.2} rad/s)",
        jog.source,
        jog.sample_rad,
        jog.sample_dt * 1000.0,
        jog.sample_omega
    )
}

/// What the jog touch byte says.
fn touch_meaning(touch: u8) -> &'static str {
    match touch {
        0x00 => "released",
        0x03 => "held, still",
        0x04 => "at rest",
        0x07 => "held, forward",
        0x08 => "free, backward",
        0x0b => "held, backward",
        0x0c => "free, forward",
        _ => "",
    }
}

/// The narrowest a lamp cell may be; cells widen to fill each row.
const LAMP_CELL_MIN_WIDTH: f32 = 200.0;
const LAMP_CELL_HEIGHT: f32 = 22.0;
const LAMP_CELL_GAP: f32 = 6.0;
const LAMP_DOT_RADIUS: f32 = 5.0;

/// Every lamp the model has, one cell each, in rows that fill the width.
fn draw_lamps(ui: &mut egui::Ui, snap: &DebugSnapshot) {
    let mosi = snap.model.mosi_frame(snap.led_frame);
    let lamps: Vec<(Lamp, LampState)> = Lamp::ALL
        .iter()
        .filter_map(|l| Some((*l, mosi.lamp(*l)?)))
        .collect();
    let width = ui.available_width();
    let per_row =
        (((width + LAMP_CELL_GAP) / (LAMP_CELL_MIN_WIDTH + LAMP_CELL_GAP)) as usize).max(1);
    let cell_width = (width - LAMP_CELL_GAP * (per_row - 1) as f32) / per_row as f32;
    ui.spacing_mut().item_spacing = egui::vec2(LAMP_CELL_GAP, LAMP_CELL_GAP);
    for row in lamps.chunks(per_row) {
        ui.horizontal(|ui| {
            for (lamp, state) in row {
                lamp_cell(ui, *lamp, *state, cell_width);
            }
        });
    }
}

/// One lamp: an LED dot, lit in its colour or hollow when dark, the lamp's
/// name, and its value on the right.
fn lamp_cell(ui: &mut egui::Ui, lamp: Lamp, state: LampState, width: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, LAMP_CELL_HEIGHT), egui::Sense::hover());
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, visuals.faint_bg_color);
    let lit = state.drive() > 0;
    let dot = egui::pos2(rect.left() + 12.0, rect.center().y);
    if lit {
        painter.circle_filled(dot, LAMP_DOT_RADIUS, swatch(state));
    } else {
        painter.circle_stroke(
            dot,
            LAMP_DOT_RADIUS - 0.5,
            egui::Stroke::new(1.0, visuals.weak_text_color()),
        );
    }
    let font = egui::FontId::monospace(12.0);
    let name_color = if lit {
        visuals.strong_text_color()
    } else {
        visuals.weak_text_color()
    };
    painter.text(
        egui::pos2(dot.x + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        lamp.name(),
        font.clone(),
        name_color,
    );
    painter.text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        lamp_value(state),
        font,
        visuals.weak_text_color(),
    );
}

fn lamp_value(state: LampState) -> String {
    match state {
        LampState::Bit(on) => if on { "on" } else { "off" }.into(),
        LampState::Step(StepLed::Off) => "off".into(),
        LampState::Step(StepLed::Medium) => "medium".into(),
        LampState::Step(StepLed::Full) => "full".into(),
        LampState::Level(v) => v.to_string(),
        LampState::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
    }
}

/// The colour of a lamp's dot: an RGB lamp's colour scaled to full
/// brightness, or a grey as bright as the lamp's drive.
fn swatch(state: LampState) -> Color32 {
    match state {
        LampState::Rgb(r, g, b) => {
            let peak = r.max(g).max(b);
            if peak == 0 {
                return Color32::BLACK;
            }
            let scale = |c: u8| (c as u32 * 255 / peak as u32) as u8;
            Color32::from_rgb(scale(r), scale(g), scale(b))
        }
        other => Color32::from_gray(other.drive().max(24)),
    }
}

/// Show a heading, then the first 64 bytes of `frame` in four rows of 16.
fn hex_dump(ui: &mut egui::Ui, heading: &str, frame: &[u8]) {
    ui.label(RichText::new(heading).strong());
    for (row, chunk) in frame.chunks(16).take(4).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        ui.label(mono(format!("b{:02}  {}", row * 16, hex.join(" "))));
    }
}
