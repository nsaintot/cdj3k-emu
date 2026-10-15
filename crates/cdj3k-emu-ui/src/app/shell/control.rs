//! The control API host. While Developer Mode is on, the window listens on
//! the slot's control port ([`cdj3k_emu_control`]), runs each request against
//! the panel, and lets go of what a connection held when its time is up or
//! the connection closes. The `watch` module sends the events of `watch`
//! requests. `sh` and `watch log` run commands in the guest through its devd
//! ([`cdj3k_emu_runtime::dev`]).

mod emulator;
mod mods;
mod watch;

use std::time::{Duration, Instant};

use cdj3k_emu_control::command::{DEVD_WAIT, SH_WAIT};
use cdj3k_emu_control::{
    self as control, Command, ConnId, ControlServer, Incoming, Knob, LampWant, Responder, Target,
    Topic, WaitFor,
};
use cdj3k_emu_panel::{JogBrake, Lamp, Model};
use cdj3k_emu_platform::{app_meta, menu_state, runtime_paths};
use cdj3k_emu_runtime::dev::{DevClient, Job, JobEvent};
use cdj3k_emu_streams::{jog_stream, main_stream};
use serde_json::{json, Value};

use super::{CdjShell, Screen};

/// How often `logic` runs while a timed hold, a wait or a capture is pending.
const PENDING_TICK: Duration = Duration::from_millis(10);
/// The most output `sh` returns; the rest is dropped.
const SH_OUTPUT_MAX: usize = 1 << 20;
/// How often `logic` runs while a lamp, emulation or log watch is open.
const WATCH_TICK: Duration = Duration::from_millis(30);
/// How long a window capture may take before the request fails.
const SHOT_WAIT: Duration = Duration::from_secs(5);

/// Something the control API holds on the panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    Btn((usize, u8)),
    Cleared((usize, u8)),
    Touch,
}

/// A guest command that `sh` runs, and the output it has sent.
struct PendingSh {
    job: Job,
    deadline: Instant,
    reply: Responder,
    output: String,
    truncated: bool,
}

struct PendingWait {
    what: WaitFor,
    deadline: Instant,
    reply: Responder,
}

#[derive(Default)]
pub(super) struct ControlHost {
    server: Option<ControlServer>,
    /// Whether the last bind failed. The server binds again the next time
    /// Developer Mode is turned on.
    bind_failed: bool,
    /// Holds that last until `release` or until their connection closes.
    holds: Vec<(ConnId, Hold)>,
    /// Holds with a time limit, which outlive their connection.
    timed: Vec<(Instant, Hold)>,
    waits: Vec<PendingWait>,
    /// The window capture under way, with its deadline.
    shot: Option<(Instant, Responder)>,
    watches: Vec<watch::Watch>,
    /// The number the next watch gets.
    next_watch: u32,
    /// Whether a restart was requested and the run it replaces has not
    /// stopped yet. That run does not count as booted.
    restarting: bool,
    /// The connection to the guest's devd.
    dev: Option<DevClient>,
    sh: Vec<PendingSh>,
    /// Guest commands waiting for devd to connect, with their deadline.
    sh_queued: Vec<(String, Instant, Responder)>,
    /// The connection and request whose jog turn is under way; the request is
    /// answered when the turn ends.
    jog: Option<(ConnId, Responder)>,
    /// The media change under way.
    media: Option<emulator::PendingMedia>,
}

impl ControlHost {
    /// The connection to the guest's devd, while Developer Mode is on.
    pub(super) fn dev(&self) -> Option<&DevClient> {
        self.dev.as_ref()
    }

    /// How soon `logic` has to run again, if anything is pending.
    fn tick(&self, jog_busy: bool) -> Option<Duration> {
        if jog_busy
            || !self.timed.is_empty()
            || !self.waits.is_empty()
            || self.shot.is_some()
            || !self.sh.is_empty()
            || !self.sh_queued.is_empty()
            || self.media.is_some()
        {
            Some(PENDING_TICK)
        } else if self.watches.iter().any(watch::Watch::polled) {
            Some(WATCH_TICK)
        } else {
            None
        }
    }

    /// Record `hold` for `conn`, replacing any earlier hold of the same input.
    fn track(&mut self, conn: ConnId, hold: Hold, time: Option<Duration>) {
        self.forget(hold);
        match time {
            Some(t) => self.timed.push((Instant::now() + t, hold)),
            None => self.holds.push((conn, hold)),
        }
    }

    fn forget(&mut self, hold: Hold) {
        self.holds.retain(|(_, h)| *h != hold);
        self.timed.retain(|(_, h)| *h != hold);
    }
}

type Answer = Result<Value, String>;
/// The responder and the reason when a deferred request could not start, or
/// `None` when it started.
type Refused = Option<(Responder, String)>;

fn done() -> Answer {
    Ok(control::ok().into())
}

impl CdjShell {
    /// Start or stop the server with Developer Mode, and serve its requests.
    /// `logic` calls it, so it runs while the window is hidden.
    pub(super) fn serve_control(&mut self, ctx: &egui::Context) {
        self.follow_dev_mode(ctx);
        self.follow_restart();
        self.release_due();
        while let Some(msg) = self.control.server.as_ref().and_then(ControlServer::next) {
            match msg {
                Incoming::Request {
                    conn,
                    command,
                    reply,
                } => self.on_control(ctx, conn, command, reply),
                Incoming::Closed(conn) => {
                    self.release_conn(conn);
                    self.control.watches.retain(|w| w.conn != conn);
                }
            }
        }
        self.settle_waits();
        self.settle_shot();
        self.start_queued_sh();
        self.settle_sh();
        self.settle_jog();
        self.settle_media();
        self.poll_watches();
        if let Some(tick) = self.control.tick(self.app.jog_busy()) {
            ctx.request_repaint_after(tick);
        }
    }

    fn follow_dev_mode(&mut self, ctx: &egui::Context) {
        let (on, toggled) = {
            let mut s = menu_state::lock();
            (s.dev_mode, std::mem::take(&mut s.dev_mode_toggle_requested))
        };
        if toggled {
            self.control.bind_failed = false;
            if let Err(e) =
                cdj3k_emu_storage::InstanceSettings::update(self.instance, |s| s.dev_mode = on)
            {
                eprintln!("cdj3k-emu: saving Developer Mode failed: {e}");
            }
        }
        if !on {
            if self.control.server.is_some() {
                self.release_all();
                self.control = ControlHost::default();
                eprintln!(
                    "cdj3k-emu: Slot {}: the control API stopped.",
                    self.instance
                );
            }
            return;
        }
        if self.control.server.is_some() || self.control.bind_failed {
            return;
        }
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        match ControlServer::bind(self.instance, wake) {
            Ok(server) => {
                eprintln!(
                    "cdj3k-emu: Slot {}: the control API is listening on port {}.",
                    self.instance,
                    server.port()
                );
                self.control.server = Some(server);
                self.control.dev = Some(DevClient::new(runtime_paths::instance_dir(self.instance)));
            }
            Err(e) => {
                eprintln!(
                    "cdj3k-emu: Slot {}: the control API could not listen on port {}: {e}",
                    self.instance,
                    control::port(self.instance)
                );
                self.control.bind_failed = true;
            }
        }
    }

    fn on_control(
        &mut self,
        ctx: &egui::Context,
        conn: ConnId,
        command: Command,
        reply: Responder,
    ) {
        let answer = match command {
            Command::Shot
            | Command::Wait { .. }
            | Command::Sh(_)
            | Command::Jog(_)
            | Command::Media(_) => {
                return self.on_deferred(ctx, conn, command, reply);
            }
            Command::Press { .. }
            | Command::Clear { .. }
            | Command::Touch { .. }
            | Command::Release(_)
            | Command::Rotary(_)
            | Command::Tempo(_)
            | Command::Tilt
            | Command::Encoder(_)
            | Command::Knob(..)
            | Command::Direction(_) => self.on_input(conn, command),
            Command::Watch(topic) => self.start_watch(conn, topic, &reply),
            Command::Unwatch(id) => self.unwatch(conn, id),
            other => self.on_query(other),
        };
        reply.send(answer.unwrap_or_else(control::error));
    }

    /// A request that reads the slot, edits its mods or ends its emulation.
    fn on_query(&mut self, command: Command) -> Answer {
        match command {
            Command::Hello => Ok(self.hello().into()),
            Command::Buttons => Ok(json!({ "ok": true, "buttons": self.app.button_names() })),
            Command::Lamps => Ok(json!({ "ok": true, "lamps": self.app.lamps_json() })),
            Command::State => Ok(self.state()),
            Command::Lcd(screen) => capture_lcd(self.instance, screen),
            Command::Mods(command) => self.on_mods(command),
            Command::Setting(setting) => self.on_setting(setting),
            Command::Restart => self.restart_from_api(false),
            Command::Reset => self.restart_from_api(true),
            Command::Quit => {
                self.close_window = true;
                done()
            }
            other => Err(format!("{other:?} is not handled here")),
        }
    }

    /// A request that acts on the panel's inputs.
    fn on_input(&mut self, conn: ConnId, command: Command) -> Answer {
        missing_control(self.app.model(), &command)?;
        match command {
            Command::Press { bit, hold } => return self.hold_bit(conn, &bit, hold, false),
            Command::Clear { bit, hold } => return self.hold_bit(conn, &bit, hold, true),
            Command::Release(target) => return self.release_target(conn, target),
            Command::Touch { x, y, hold } => {
                self.app.set_api_touch(Some((x, y)));
                self.control.track(conn, Hold::Touch, hold);
            }
            other => self.app.move_control(other)?,
        }
        done()
    }

    /// A request answered once something finishes: a capture, a wait, a guest
    /// command or a jog turn. A request that cannot start is answered with
    /// an error at once.
    fn on_deferred(
        &mut self,
        ctx: &egui::Context,
        conn: ConnId,
        command: Command,
        reply: Responder,
    ) {
        let refused = match command {
            Command::Shot => self.start_shot(ctx, reply),
            Command::Wait { what, timeout } => self.start_wait(what, timeout, reply),
            Command::Sh(command) => self.start_sh_job(&command, reply),
            Command::Jog(mv) => self.start_jog(conn, mv, reply),
            Command::Media(command) => self.start_media(command, reply),
            other => Some((reply, format!("{other:?} is not handled here"))),
        };
        if let Some((reply, why)) = refused {
            reply.send(control::error(why));
        }
    }

    fn start_shot(&mut self, ctx: &egui::Context, reply: Responder) -> Refused {
        if self.control.shot.is_some() {
            return Some((reply, "a window capture is already under way".into()));
        }
        self.shot.request_image(ctx);
        self.control.shot = Some((Instant::now() + SHOT_WAIT, reply));
        None
    }

    fn start_wait(&mut self, what: WaitFor, timeout: Duration, reply: Responder) -> Refused {
        if let Err(e) = lamp_of(&what) {
            return Some((reply, e));
        }
        let deadline = Instant::now() + timeout;
        self.control.waits.push(PendingWait {
            what,
            deadline,
            reply,
        });
        None
    }

    fn start_sh_job(&mut self, command: &str, reply: Responder) -> Refused {
        let devd_down = self.control.dev.as_ref().is_some_and(|d| !d.is_up());
        if devd_down && !cdj3k_emu_runtime::worker_is_finished() {
            let deadline = Instant::now() + DEVD_WAIT;
            self.control
                .sh_queued
                .push((command.into(), deadline, reply));
            return None;
        }
        self.begin_sh(command, reply)
    }

    fn begin_sh(&mut self, command: &str, reply: Responder) -> Refused {
        match self.start_sh(command) {
            Err(e) => Some((reply, e)),
            Ok(job) => {
                self.control.sh.push(PendingSh {
                    job,
                    deadline: Instant::now() + SH_WAIT,
                    reply,
                    output: String::new(),
                    truncated: false,
                });
                None
            }
        }
    }

    /// Start a jog move. A turn is answered when it ends, and a spin is
    /// answered at once.
    fn start_jog(&mut self, conn: ConnId, mv: control::JogMove, reply: Responder) -> Refused {
        if self.control.jog.is_some() {
            return Some((reply, "a jog turn is under way".into()));
        }
        self.app.start_api_jog(mv);
        if self.app.api_jog_active() {
            self.control.jog = Some((conn, reply));
        } else {
            reply.send(control::ok());
        }
        None
    }

    fn start_watch(&mut self, conn: ConnId, topic: Topic, reply: &Responder) -> Answer {
        let id = self.control.next_watch;
        let dir = runtime_paths::instance_dir(self.instance);
        let mosi = self.app.live_mosi();
        let dev = self.control.dev.as_ref();
        let w = watch::Watch::start(conn, id, reply.events(), topic, &mosi, dir, dev)?;
        self.control.next_watch += 1;
        self.control.watches.push(w);
        Ok(json!({ "ok": true, "watch": id }))
    }

    fn unwatch(&mut self, conn: ConnId, id: Option<u32>) -> Answer {
        let before = self.control.watches.len();
        self.control
            .watches
            .retain(|w| w.conn != conn || id.is_some_and(|id| w.id != id));
        match id {
            Some(id) if self.control.watches.len() == before => {
                Err(format!("this connection has no watch {id}"))
            }
            _ => done(),
        }
    }

    /// Send what changed to each polled watch, and drop the watches whose
    /// connection is gone.
    fn poll_watches(&mut self) {
        if !self.control.watches.iter().any(watch::Watch::polled) {
            return;
        }
        let mosi = self.app.live_mosi();
        let emulation = self.emulation_json();
        self.control
            .watches
            .retain_mut(|w| w.poll(&mosi, &emulation));
    }

    fn emulation_json(&self) -> Value {
        let s = menu_state::lock();
        json!({
            "model": self.model.slug(),
            "running": s.qemu_running,
            "booted": self.booted(&s),
        })
    }

    /// Whether the player app drives the panel on the current run.
    fn booted(&self, s: &menu_state::AppState) -> bool {
        s.qemu_running && self.app.panel_driven() && !self.control.restarting
    }

    /// Note a restart that is asked for, and clear it once the old run has
    /// stopped: QEMU is down or respawning, or the panel lost its driver.
    fn follow_restart(&mut self) {
        let s = menu_state::lock();
        if s.restart_requested {
            self.control.restarting = true;
        } else if self.control.restarting
            && (!s.qemu_running || s.qemu_respawning || !self.app.panel_driven())
        {
            self.control.restarting = false;
        }
    }

    fn hello(&self) -> serde_json::Map<String, Value> {
        let s = menu_state::lock();
        let mut m = control::ok();
        m.insert("protocol".into(), control::PROTOCOL.into());
        m.insert("app".into(), app_meta::APP_ID.into());
        m.insert("version".into(), app_meta::VERSION.into());
        m.insert("slot".into(), self.instance.into());
        m.insert("model".into(), self.model.slug().into());
        m.insert(
            "release".into(),
            cdj3k_emu_storage::InstanceSettings::saved_firmware_release(self.instance).into(),
        );
        m.insert("panel".into(), matches!(self.screen, Screen::Panel).into());
        m.insert("running".into(), s.qemu_running.into());
        m.insert("booted".into(), self.booted(&s).into());
        m.insert("ephemeral".into(), s.ephemeral.into());
        m.insert(
            "accel".into(),
            cdj3k_emu_platform::host::accelerator()
                .map(|a| a.name)
                .into(),
        );
        m
    }

    fn state(&self) -> Value {
        let mut m = self.hello();
        m.insert("lamps".into(), self.app.lamps_json().into());
        m.insert("inputs".into(), self.app.inputs_json());
        m.into()
    }

    fn hold_bit(
        &mut self,
        conn: ConnId,
        name: &str,
        time: Option<Duration>,
        clear: bool,
    ) -> Answer {
        let bit = self.bit_by_name(name)?;
        let hold = if clear {
            Hold::Cleared(bit)
        } else {
            Hold::Btn(bit)
        };
        self.apply(hold, true);
        self.control.track(conn, hold, time);
        Ok(json!({ "ok": true, "bit": format!("{}:{:#04x}", bit.0, bit.1) }))
    }

    /// The frame bit of the button or bit `name` on this model.
    fn bit_by_name(&self, name: &str) -> Result<(usize, u8), String> {
        self.app.btn_by_name(name).ok_or_else(|| {
            format!(
                "the {} has no button or frame bit {name:?}",
                self.model.title()
            )
        })
    }

    fn apply(&mut self, hold: Hold, on: bool) {
        match hold {
            Hold::Btn(bit) => self.app.set_api_btn(bit, on),
            Hold::Cleared(bit) => self.app.set_bit_cleared(bit, on),
            Hold::Touch if !on => self.app.set_api_touch(None),
            Hold::Touch => {}
        }
    }

    fn release(&mut self, hold: Hold) {
        self.control.forget(hold);
        self.apply(hold, false);
    }

    fn release_target(&mut self, conn: ConnId, target: Target) -> Answer {
        match target {
            Target::All => self.release_conn(conn),
            Target::Touch => self.release(Hold::Touch),
            Target::Bit(name) => {
                let bit = self.bit_by_name(&name)?;
                self.release(Hold::Btn(bit));
                self.release(Hold::Cleared(bit));
            }
        }
        done()
    }

    /// Let go of every timed hold whose time is up.
    fn release_due(&mut self) {
        let now = Instant::now();
        let due: Vec<Hold> = (self.control.timed.iter())
            .filter(|(t, _)| *t <= now)
            .map(|(_, h)| *h)
            .collect();
        for h in due {
            self.release(h);
        }
    }

    /// Release the holds without a time limit that `conn` made, and its jog
    /// turn.
    fn release_conn(&mut self, conn: ConnId) {
        if self.control.jog.as_ref().is_some_and(|(c, _)| *c == conn) {
            self.app.stop_api_jog();
            if let Some((_, reply)) = self.control.jog.take() {
                reply.send(control::error("the jog turn was released"));
            }
        }
        let held: Vec<Hold> = (self.control.holds.iter())
            .filter(|(c, _)| *c == conn)
            .map(|(_, h)| *h)
            .collect();
        for h in held {
            self.release(h);
        }
    }

    fn release_all(&mut self) {
        let all: Vec<Hold> = (self.control.holds.iter().map(|(_, h)| *h))
            .chain(self.control.timed.iter().map(|(_, h)| *h))
            .collect();
        for h in all {
            self.release(h);
        }
    }

    fn settle_waits(&mut self) {
        let now = Instant::now();
        for w in std::mem::take(&mut self.control.waits) {
            if self.wait_met(&w.what) {
                w.reply.send(control::ok());
            } else if now >= w.deadline {
                w.reply.send(control::error("the wait timed out"));
            } else {
                self.control.waits.push(w);
            }
        }
    }

    fn wait_met(&self, what: &WaitFor) -> bool {
        match what {
            WaitFor::Boot => self.booted(&menu_state::lock()),
            WaitFor::Lamp { want, .. } => {
                let Ok(Some(lamp)) = lamp_of(what) else {
                    return false;
                };
                match want {
                    LampWant::On => self.app.live_mosi().lit(lamp),
                    LampWant::Off => !self.app.live_mosi().lit(lamp),
                    LampWant::Is(value) => self.app.lamp_value(lamp).as_ref() == Some(value),
                }
            }
        }
    }

    fn settle_shot(&mut self) {
        let Some((deadline, _)) = &self.control.shot else {
            return;
        };
        let answer = match self.shot.take_image() {
            Some(img) => image_reply(window_image(&img)),
            None if Instant::now() >= *deadline => {
                Err("the window drew no frame to capture; it may be minimized".into())
            }
            None => return,
        };
        if let Some((_, reply)) = self.control.shot.take() {
            reply.send(answer.unwrap_or_else(control::error));
        }
    }

    /// Advance the API's jog turn, tick the jog while `ui` does not, and
    /// answer the turn's request once it has ended.
    fn settle_jog(&mut self) {
        self.app.step_api_jog();
        self.app.tick_jog_if_idle();
        if !self.app.api_jog_active() {
            if let Some((_, reply)) = self.control.jog.take() {
                reply.send(control::ok());
            }
        }
    }

    fn start_sh(&self, command: &str) -> Result<Job, String> {
        let dev = self
            .control
            .dev
            .as_ref()
            .ok_or("guest commands need Developer Mode")?;
        dev.start(command).map_err(|e| e.to_string())
    }

    /// Start the queued guest commands once devd is up, or refuse them when
    /// it did not connect in time.
    fn start_queued_sh(&mut self) {
        let up = self.control.dev.as_ref().is_some_and(DevClient::is_up);
        let now = Instant::now();
        for (command, deadline, reply) in std::mem::take(&mut self.control.sh_queued) {
            if up || now >= deadline {
                if let Some((reply, why)) = self.begin_sh(&command, reply) {
                    reply.send(control::error(why));
                }
            } else {
                self.control.sh_queued.push((command, deadline, reply));
            }
        }
    }

    /// Collect the output of each guest command, and answer the ones that
    /// ended or ran out of time.
    fn settle_sh(&mut self) {
        let now = Instant::now();
        for mut p in std::mem::take(&mut self.control.sh) {
            let mut end = None;
            for event in p.job.take() {
                match event {
                    JobEvent::Out(line) if p.output.len() + line.len() < SH_OUTPUT_MAX => {
                        p.output.push_str(&line);
                        p.output.push('\n');
                    }
                    JobEvent::Out(_) => p.truncated = true,
                    other => end = Some(other),
                }
            }
            let status = match end {
                Some(JobEvent::Exit(status)) => Some(status),
                Some(_) => None,
                None if now >= p.deadline => {
                    p.job.kill();
                    None
                }
                None => {
                    self.control.sh.push(p);
                    continue;
                }
            };
            let why = match (status, &end) {
                (Some(0), _) => None,
                (_, Some(end)) => Some(job_end(end)),
                (_, None) => Some(format!(
                    "the command ran longer than {} s and was killed",
                    SH_WAIT.as_secs()
                )),
            };
            p.reply.send(json!({
                "ok": why.is_none(),
                "error": why,
                "status": status,
                "output": p.output,
                "truncated": p.truncated,
            }));
        }
    }

    /// Restart the emulation. With `fresh`, also replace an ephemeral slot's
    /// overlay with an empty one.
    fn restart_from_api(&mut self, fresh: bool) -> Answer {
        if cdj3k_emu_runtime::worker_is_finished() {
            return Err("the emulation is not running".into());
        }
        let mut s = menu_state::lock();
        if fresh && !s.ephemeral {
            return Err("the slot is not ephemeral; launch it with --ephemeral".into());
        }
        s.ephemeral_reset_requested |= fresh;
        s.shade_forced = true;
        s.restart_requested = true;
        self.control.restarting = true;
        done()
    }
}

/// An error when `command` moves a control that `model` does not have.
fn missing_control(model: Model, command: &Command) -> Result<(), String> {
    let spec = model.spec();
    let control = match command {
        Command::Encoder(_) if !spec.controls.beat_loop_encoder => "BEAT LOOP encoder",
        Command::Direction(_) if !spec.controls.direction_lever => "direction lever",
        Command::Knob(Knob::VinylSpeed, _) if !spec.controls.vinyl_speed_knob => {
            "VINYL SPEED ADJUST knob"
        }
        Command::Knob(Knob::JogAdjust, _) if spec.jog_brake != JogBrake::Adjustable => {
            "JOG ADJUST knob"
        }
        _ => return Ok(()),
    };
    Err(format!("the {} has no {control}", spec.title))
}

/// Why a guest job ended, for a reply or an event.
fn job_end(event: &JobEvent) -> String {
    match event {
        JobEvent::Exit(status) => format!("the command exited with status {status}"),
        JobEvent::Error(why) => format!("the command did not start: {why}"),
        JobEvent::Lost => "the guest's devd stopped or restarted".into(),
        JobEvent::Out(_) => "the command is still running".into(),
    }
}

/// The lamp that a lamp wait names, `Ok(None)` for any other wait, or an
/// error when no lamp has that name.
fn lamp_of(what: &WaitFor) -> Result<Option<Lamp>, String> {
    match what {
        WaitFor::Lamp { name, .. } => Lamp::from_name(name)
            .map(Some)
            .ok_or_else(|| format!("{name:?} is not a lamp")),
        WaitFor::Boot => Ok(None),
    }
}

fn capture_lcd(instance: u32, screen: control::Screen) -> Answer {
    let dir = runtime_paths::instance_dir(instance);
    image_reply(capture_image(&dir.to_string_lossy(), screen)?)
}

/// Capture `screen` from the slot's runtime directory `dir`.
fn capture_image(dir: &str, screen: control::Screen) -> Result<control::Image, String> {
    let (w, h, rgba) = match screen {
        control::Screen::Main => main_stream::capture(dir)?,
        control::Screen::Jog => jog_stream::capture(dir)?,
    };
    control::Image::new(w, h, rgba)
}

fn window_image(img: &egui::ColorImage) -> control::Image {
    let rgba = img.pixels.iter().flat_map(|p| p.to_array()).collect();
    control::Image {
        width: img.size[0] as u32,
        height: img.size[1] as u32,
        rgba,
    }
}

fn image_reply(img: control::Image) -> Answer {
    Ok(json!({
        "ok": true,
        "width": img.width,
        "height": img.height,
        "png": img.to_base64_png()?,
    }))
}

#[cfg(test)]
mod tests {
    use cdj3k_emu_control::Lever;

    use super::*;

    #[test]
    fn requests_for_missing_controls_fail() {
        let encoder = Command::Encoder(1);
        let lever = Command::Direction(Lever::Reverse);
        let vinyl = Command::Knob(Knob::VinylSpeed, 0.5);
        let jog_adjust = Command::Knob(Knob::JogAdjust, 0.5);
        for model in [Model::Cdj3k, Model::Cdj3kx] {
            assert!(missing_control(model, &encoder).is_err());
            assert!(missing_control(model, &lever).is_ok());
            assert!(missing_control(model, &vinyl).is_ok());
            assert!(missing_control(model, &jog_adjust).is_ok());
        }
        assert!(missing_control(Model::Cdj1500x, &encoder).is_ok());
        assert_eq!(
            missing_control(Model::Cdj1500x, &lever),
            Err("the CDJ-1500X has no direction lever".into())
        );
        assert!(missing_control(Model::Cdj1500x, &vinyl).is_err());
        assert!(missing_control(Model::Cdj1500x, &jog_adjust).is_err());
    }
}
