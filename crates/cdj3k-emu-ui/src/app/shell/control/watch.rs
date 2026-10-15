//! The control API's watches. Each sends events about one topic to the
//! connection that asked for it, until `unwatch` or until the connection
//! closes. Lamp, emulation and log watches are polled from `logic`; an LCD
//! watch has its own thread, because comparing captures takes a few
//! milliseconds. A log watch follows a guest journal through devd and ends
//! when its job ends.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cdj3k_emu_control::diff::{self, Options};
use cdj3k_emu_control::{ConnId, EventSink, Screen, Topic};
use cdj3k_emu_panel::{Lamp, MosiFrame};
use cdj3k_emu_runtime::dev::{DevClient, Job, JobEvent};
use serde_json::{json, Value};

use super::capture_image;

/// How often an LCD watch captures its screen.
const LCD_POLL: Duration = Duration::from_millis(100);

pub(super) struct Watch {
    pub(super) conn: ConnId,
    pub(super) id: u32,
    sink: EventSink,
    kind: Kind,
}

enum Kind {
    /// The watched lamps, with the value last sent for each.
    Lamps(Vec<(Lamp, Option<Value>)>),
    /// The emulation state last sent.
    Emulation(Option<Value>),
    /// The stop flag of the LCD watch's thread.
    Lcd(Arc<AtomicBool>),
    /// The guest job that follows the journal, and the lowercase text a line
    /// must contain.
    Log { job: Job, grep: Option<String> },
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Kind::Lcd(stop) = &self.kind {
            stop.store(true, Ordering::Release);
        }
    }
}

impl Watch {
    /// Start a watch of `topic`. `mosi` names the lamps the model has, `dir`
    /// is the slot's runtime directory and `dev` runs guest commands.
    pub(super) fn start(
        conn: ConnId,
        id: u32,
        sink: EventSink,
        topic: Topic,
        mosi: &MosiFrame,
        dir: PathBuf,
        dev: Option<&DevClient>,
    ) -> Result<Watch, String> {
        let kind = match topic {
            Topic::Log { source, grep } => {
                let dev = dev.ok_or("guest commands need Developer Mode")?;
                let job = dev.start(&source.command()).map_err(|e| e.to_string())?;
                Kind::Log {
                    job,
                    grep: grep.map(|g| g.to_lowercase()),
                }
            }
            Topic::Lamps(names) => Kind::Lamps(watched_lamps(&names, mosi)?),
            Topic::Emulation => Kind::Emulation(None),
            Topic::Lcd { screen, region } => {
                let stop = Arc::new(AtomicBool::new(false));
                let opts = Options {
                    region,
                    ..Default::default()
                };
                let (sink, flag) = (sink.clone(), Arc::clone(&stop));
                thread::Builder::new()
                    .name(format!("control-watch-{id}"))
                    .spawn(move || watch_lcd(id, screen, opts, dir, sink, flag))
                    .map_err(|e| e.to_string())?;
                Kind::Lcd(stop)
            }
        };
        Ok(Watch {
            conn,
            id,
            sink,
            kind,
        })
    }

    /// Whether `logic` has to poll this watch.
    pub(super) fn polled(&self) -> bool {
        !matches!(self.kind, Kind::Lcd(_))
    }

    /// Send what changed since the last poll. Returns `false` once the
    /// connection is gone.
    pub(super) fn poll(&mut self, mosi: &MosiFrame, emulation: &Value) -> bool {
        let id = self.id;
        match &mut self.kind {
            Kind::Lamps(lamps) => lamps.iter_mut().all(|(lamp, last)| {
                let value = mosi
                    .lamp(*lamp)
                    .map_or(Value::Null, crate::app::api_input::lamp_json);
                if last.as_ref() == Some(&value) {
                    return true;
                }
                *last = Some(value.clone());
                let event =
                    json!({ "event": "lamp", "watch": id, "lamp": lamp.name(), "value": value });
                self.sink.send(event)
            }),
            Kind::Emulation(last) => {
                if last.as_ref() == Some(emulation) {
                    return true;
                }
                *last = Some(emulation.clone());
                let mut event = json!({ "event": "emulation", "watch": id });
                event
                    .as_object_mut()
                    .expect("an object")
                    .extend(emulation.as_object().cloned().unwrap_or_default());
                self.sink.send(event)
            }
            Kind::Lcd(_) => true,
            Kind::Log { job, grep } => {
                for event in job.take() {
                    let event = match event {
                        JobEvent::Out(line) => {
                            if grep
                                .as_ref()
                                .is_some_and(|g| !line.to_lowercase().contains(g))
                            {
                                continue;
                            }
                            json!({ "event": "log", "watch": id, "line": line })
                        }
                        end => {
                            let why = super::job_end(&end);
                            self.sink
                                .send(json!({ "event": "log", "watch": id, "ended": why }));
                            return false;
                        }
                    };
                    if !self.sink.send(event) {
                        return false;
                    }
                }
                true
            }
        }
    }
}

/// The lamps `names` asks for, or every lamp of the model when it is empty.
fn watched_lamps(names: &[String], mosi: &MosiFrame) -> Result<Vec<(Lamp, Option<Value>)>, String> {
    let lamps = if names.is_empty() {
        Lamp::ALL
            .iter()
            .copied()
            .filter(|l| mosi.lamp(*l).is_some())
            .collect()
    } else {
        names
            .iter()
            .map(|n| match Lamp::from_name(n) {
                Some(l) if mosi.lamp(l).is_some() => Ok(l),
                Some(_) => Err(format!("this model has no {n} lamp")),
                None => Err(format!("{n:?} is not a lamp")),
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(lamps.into_iter().map(|l| (l, None)).collect())
}

/// Capture `screen` every [`LCD_POLL`] and send an event when pixels inside
/// the region changed, until `stop` is set or the connection is gone.
fn watch_lcd(
    id: u32,
    screen: Screen,
    opts: Options,
    dir: PathBuf,
    sink: EventSink,
    stop: Arc<AtomicBool>,
) {
    let dir = dir.to_string_lossy().into_owned();
    let mut prev = None;
    while !stop.load(Ordering::Acquire) {
        thread::sleep(LCD_POLL);
        let Ok(img) = capture_image(&dir, screen) else {
            prev = None;
            continue;
        };
        if let Some(report) = prev
            .as_ref()
            .and_then(|p| diff::compare(p, &img, &opts).ok())
            .filter(|r| r.changed > 0)
        {
            let event = json!({
                "event": "lcd",
                "watch": id,
                "screen": screen.as_str(),
                "changed": report.changed,
                "bbox": report.bbox,
            });
            if !sink.send(event) {
                return;
            }
        }
        prev = Some(img);
    }
}
