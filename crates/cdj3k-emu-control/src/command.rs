//! The request grammar. A request is one line of words separated by spaces;
//! see `docs/control-api.md` for the table of commands.

use std::path::PathBuf;
use std::time::Duration;

use crate::diff::Region;

/// How long `press` and `touch` hold when the request gives no time.
pub const DEFAULT_HOLD: Duration = Duration::from_millis(200);
/// How long `wait` waits when the request gives no time.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);
/// The longest `wait` a request may ask for.
pub const MAX_WAIT: Duration = Duration::from_secs(600);
/// How long the connection waits for the window to answer any other command.
pub const ANSWER_WAIT: Duration = Duration::from_secs(10);
/// The most detents one `rotary` request may turn.
pub const MAX_DETENTS: i32 = 100;
/// How long `media attach` and `media eject` wait for the change.
pub const MEDIA_WAIT: Duration = Duration::from_secs(30);
/// How long `sh` lets a guest command run before it kills it.
pub const SH_WAIT: Duration = Duration::from_secs(30);
/// How long `sh` waits for the guest's devd to connect, as it does after a
/// restart, before the command starts.
pub const DEVD_WAIT: Duration = Duration::from_secs(20);

/// One of the deck's two screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Jog,
}

impl Screen {
    pub fn parse(s: &str) -> Option<Screen> {
        match s.to_ascii_lowercase().as_str() {
            "main" => Some(Screen::Main),
            "jog" => Some(Screen::Jog),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Screen::Main => "main",
            Screen::Jog => "jog",
        }
    }
}

/// What `release` lets go of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A button name or a raw frame bit such as `12:0x01`.
    Bit(String),
    Touch,
    /// Everything this connection holds.
    All,
}

/// What `watch` sends events about.
#[derive(Clone, Debug, PartialEq)]
pub enum Topic {
    /// Lamp changes, for the named lamps or for every lamp when the list is
    /// empty.
    Lamps(Vec<String>),
    /// Pixel changes on a screen, inside `region` when one is given.
    Lcd {
        screen: Screen,
        region: Option<Region>,
    },
    /// The emulation starting, booting and stopping.
    Emulation,
    /// New lines of a guest journal, with only the lines that contain `grep`
    /// (case-insensitive) when it is given.
    Log {
        source: LogSource,
        grep: Option<String>,
    },
}

/// Which guest journal `watch log` follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogSource {
    All,
    Kernel,
    /// A systemd unit, such as `EP122.service`.
    Unit(String),
    /// A mod's unit, `cdj3k-mod-<name>.service`.
    Mod(String),
}

impl LogSource {
    /// The guest command that follows this journal from now on.
    pub fn command(&self) -> String {
        let base = "exec journalctl -f -n 0 --no-pager -o short-monotonic";
        match self {
            LogSource::All => base.to_string(),
            LogSource::Kernel => format!("{base} -k"),
            LogSource::Unit(unit) => format!("{base} -u {unit}"),
            LogSource::Mod(name) => format!("{base} -u cdj3k-mod-{name}.service"),
        }
    }
}

/// How the jog is held: on its top plate, as for a scratch, or by its side,
/// as for a pitch bend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JogGrip {
    Top,
    Side,
}

/// What `jog` does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JogMove {
    /// Turn the platter by `degrees` (positive is clockwise) over `time`. A
    /// turn of 0 holds the top plate still for `time`.
    Turn {
        degrees: f32,
        time: Duration,
        grip: JogGrip,
    },
    /// Let go of the platter at `rps` turns per second; it coasts to rest.
    Spin { rps: f32, grip: JogGrip },
}

/// The most degrees one `jog` request may turn.
pub const MAX_JOG_DEGREES: f32 = 36_000.0;
/// The fastest `jog spin`, in turns per second.
pub const MAX_JOG_RPS: f32 = 20.0;
/// How long `jog` turns when the request gives no time.
pub const DEFAULT_JOG_TIME: Duration = Duration::from_millis(500);

/// What `media` does with the slot's USB port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaCommand {
    Status,
    /// Attach the virtual USB image at this path, on the machine that runs
    /// the window, as Storage › Mount Image… does.
    Attach(PathBuf),
    /// Eject whatever is attached, as Eject Current Media does.
    Eject,
}

/// A knob of the deck, set to a position from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    VinylSpeed,
    JogAdjust,
}

/// A position of the direction lever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lever {
    Forward,
    Reverse,
    SlipReverse,
}

/// What `mods` does to the slot's mods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModsCommand {
    List,
    /// Add the mod at this path, on the machine that runs the window, or
    /// replace the mod of the same name; then restart a running emulation.
    Push(PathBuf),
    /// Turn a mod on or off for the next boot.
    Enable(String, bool),
    Remove(String),
    /// The mod's journal from the last boot.
    Log(String),
}

/// What `wait` waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitFor {
    /// The emulation runs and the player app has drawn its first frame.
    Boot,
    /// A lamp, by name, reaches a state.
    Lamp { name: String, want: LampWant },
}

/// The state a lamp wait waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LampWant {
    /// Any value other than off, `false`, 0 or black.
    On,
    Off,
    /// This value exactly, as `lamps` shows it.
    Is(serde_json::Value),
}

impl LampWant {
    /// `on`, `off`, or a value: `full`, `medium`, `true`, `false`, a number
    /// or a colour written `r,g,b`.
    fn parse(s: &str) -> Result<Self, String> {
        let lower = s.to_ascii_lowercase();
        let value = match lower.as_str() {
            "on" => return Ok(LampWant::On),
            "off" => return Ok(LampWant::Off),
            "full" | "medium" => lower.clone().into(),
            "true" => true.into(),
            "false" => false.into(),
            _ => match s
                .split(',')
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rgb) if rgb.len() == 3 => rgb.into(),
                Ok(level) if level.len() == 1 => level[0].into(),
                _ => {
                    return Err(format!(
                        "{s:?} is not on, off, full, medium, true, false, a number or r,g,b"
                    ))
                }
            },
        };
        Ok(LampWant::Is(value))
    }
}

/// A parsed request.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Hello,
    Buttons,
    Lamps,
    State,
    /// Hold a frame bit high. `hold` of `None` holds it until `release` or
    /// until the connection closes.
    Press {
        bit: String,
        hold: Option<Duration>,
    },
    /// Force a frame bit low, such as one the idle frame sets.
    Clear {
        bit: String,
        hold: Option<Duration>,
    },
    /// Touch the main LCD at fractions of its width and height.
    Touch {
        x: f32,
        y: f32,
        hold: Option<Duration>,
    },
    Release(Target),
    /// Turn the rotary selector by detents; a positive count moves down a
    /// list.
    Rotary(i32),
    /// Move the tempo fader: 0 is full minus, 0.5 the centre, 1 full plus.
    Tempo(f32),
    Tilt,
    Lcd(Screen),
    Shot,
    Wait {
        what: WaitFor,
        timeout: Duration,
    },
    /// Send events about a topic on this connection until `unwatch`.
    Watch(Topic),
    /// Stop one watch, or every watch of this connection with `None`.
    Unwatch(Option<u32>),
    /// Run a shell command in the guest and return its output.
    Sh(String),
    Mods(ModsCommand),
    Jog(JogMove),
    Media(MediaCommand),
    /// Read the emulator settings the menu holds, or set one by name.
    Setting(Option<(String, String)>),
    /// Turn the BEAT LOOP encoder by detents.
    Encoder(i32),
    Knob(Knob, f32),
    Direction(Lever),
    Restart,
    /// Discard an ephemeral slot's eMMC writes and restart.
    Reset,
    Quit,
}

impl Command {
    /// Parse one request line.
    pub fn parse(line: &str) -> Result<Command, String> {
        let mut w = line.split_whitespace();
        let verb = w.next().ok_or("the request is empty")?.to_ascii_lowercase();
        let args: Vec<&str> = w.collect();
        let (_, parse) = VERBS
            .iter()
            .find(|(name, _)| *name == verb)
            .ok_or_else(|| format!("{verb:?} is not a command"))?;
        parse(line, &args)
    }

    /// How long the connection waits for the window to answer this command.
    pub fn answer_wait(&self) -> Duration {
        match self {
            Command::Wait { timeout, .. } => *timeout + ANSWER_WAIT,
            Command::Sh(_) => DEVD_WAIT + SH_WAIT + ANSWER_WAIT,
            Command::Jog(JogMove::Turn { time, .. }) => *time + ANSWER_WAIT,
            Command::Media(_) => MEDIA_WAIT + ANSWER_WAIT,
            _ => ANSWER_WAIT,
        }
    }
}

/// The parser of one verb. It gets the whole line and the words after the
/// verb; `sh`, `media attach` and `mods push` read the line to keep its spaces.
type VerbParser = fn(&str, &[&str]) -> Result<Command, String>;

/// Every verb and its parser.
const VERBS: &[(&str, VerbParser)] = &[
    ("hello", |_, _| Ok(Command::Hello)),
    ("buttons", |_, _| Ok(Command::Buttons)),
    ("lamps", |_, _| Ok(Command::Lamps)),
    ("state", |_, _| Ok(Command::State)),
    ("press", |_, a| parse_press(a)),
    ("hold", |_, a| {
        let bit = arg(a, 0, "hold needs a button")?.to_string();
        Ok(Command::Press { bit, hold: None })
    }),
    ("clear", |_, a| {
        let bit = arg(a, 0, "clear needs a frame bit such as 12:0x01")?.to_string();
        Ok(Command::Clear {
            bit,
            hold: hold(a.get(1), None)?,
        })
    }),
    ("touch", |_, a| parse_touch(a)),
    ("release", |_, a| parse_release(a)),
    ("rotary", |_, a| {
        let n = arg(a, 0, "rotary needs a number of detents")?;
        Ok(Command::Rotary(detents(n)?))
    }),
    ("tempo", |_, a| {
        let pos = arg(a, 0, "tempo needs a position from 0 to 1")?;
        Ok(Command::Tempo(fraction(pos)?))
    }),
    ("tilt", |_, _| Ok(Command::Tilt)),
    ("lcd", |_, a| match a.first() {
        None => Ok(Command::Lcd(Screen::Main)),
        Some(s) => Screen::parse(s)
            .map(Command::Lcd)
            .ok_or_else(|| "lcd takes main or jog".into()),
    }),
    ("shot", |_, _| Ok(Command::Shot)),
    ("wait", |_, a| parse_wait(a)),
    ("watch", |_, a| parse_topic(a).map(Command::Watch)),
    ("unwatch", |_, a| parse_unwatch(a).map(Command::Unwatch)),
    ("sh", |line, _| sh_command(line).map(Command::Sh)),
    ("mods", |line, a| parse_mods(line, a).map(Command::Mods)),
    ("jog", |_, a| parse_jog(a).map(Command::Jog)),
    ("media", |line, a| parse_media(line, a).map(Command::Media)),
    ("setting", |_, a| match a {
        [] => Ok(Command::Setting(None)),
        [name, value] => Ok(Command::Setting(Some((
            name.to_ascii_lowercase(),
            value.to_string(),
        )))),
        _ => Err("setting takes a name and a value, or nothing to list them".into()),
    }),
    ("encoder", |_, a| match a {
        [name, n] if name.eq_ignore_ascii_case("beatloop") => Ok(Command::Encoder(detents(n)?)),
        _ => Err("encoder takes beatloop and a number of detents".into()),
    }),
    ("knob", |_, a| {
        let knob = match a.first().map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("vinyl") => Knob::VinylSpeed,
            Some("jog_adjust") => Knob::JogAdjust,
            _ => return Err("knob takes vinyl or jog_adjust, then a position from 0 to 1".into()),
        };
        Ok(Command::Knob(
            knob,
            fraction(arg(a, 1, "knob needs a position from 0 to 1")?)?,
        ))
    }),
    ("direction", |_, a| {
        let lever = match a.first().map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("fwd") => Lever::Forward,
            Some("rev") => Lever::Reverse,
            Some("slip_rev") => Lever::SlipReverse,
            _ => return Err("direction takes fwd, rev or slip_rev".into()),
        };
        Ok(Command::Direction(lever))
    }),
    ("restart", |_, _| Ok(Command::Restart)),
    ("reset", |_, _| Ok(Command::Reset)),
    ("quit", |_, _| Ok(Command::Quit)),
];

fn parse_press(args: &[&str]) -> Result<Command, String> {
    Ok(Command::Press {
        bit: arg(args, 0, "press needs a button")?.to_string(),
        hold: hold(args.get(1), Some(DEFAULT_HOLD))?,
    })
}

fn parse_touch(args: &[&str]) -> Result<Command, String> {
    Ok(Command::Touch {
        x: fraction(arg(args, 0, "touch needs an x and a y")?)?,
        y: fraction(arg(args, 1, "touch needs an x and a y")?)?,
        hold: hold(args.get(2), Some(DEFAULT_HOLD))?,
    })
}

fn parse_release(args: &[&str]) -> Result<Command, String> {
    let target = match arg(args, 0, "release needs a button, touch or all")? {
        t if t.eq_ignore_ascii_case("all") => Target::All,
        t if t.eq_ignore_ascii_case("touch") => Target::Touch,
        t => Target::Bit(t.to_string()),
    };
    Ok(Command::Release(target))
}

fn arg<'a>(args: &[&'a str], i: usize, missing: &str) -> Result<&'a str, String> {
    args.get(i).copied().ok_or_else(|| missing.to_string())
}

/// A hold time in milliseconds; `0` holds until release, and an absent value
/// takes `default`.
fn hold(s: Option<&&str>, default: Option<Duration>) -> Result<Option<Duration>, String> {
    match s {
        None => Ok(default),
        Some(s) => match millis(s)? {
            0 => Ok(None),
            ms => Ok(Some(Duration::from_millis(ms))),
        },
    }
}

fn millis(s: &str) -> Result<u64, String> {
    s.parse()
        .map_err(|_| format!("{s:?} is not a whole number of milliseconds"))
}

fn fraction(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(v) if (0.0..=1.0).contains(&v) => Ok(v),
        _ => Err(format!("{s:?} is not a number from 0 to 1")),
    }
}

fn detents(s: &str) -> Result<i32, String> {
    match s.parse::<i32>() {
        Ok(n) if n != 0 && n.abs() <= MAX_DETENTS => Ok(n),
        _ => Err(format!(
            "{s:?} is not a number of detents from -{MAX_DETENTS} to {MAX_DETENTS}, other than 0"
        )),
    }
}

/// The command after `sh`, as written.
fn sh_command(line: &str) -> Result<String, String> {
    let rest = line
        .trim()
        .split_once(char::is_whitespace)
        .map(|(_, r)| r.trim());
    match rest {
        Some(r) if !r.is_empty() => Ok(r.to_string()),
        _ => Err("sh needs a command".into()),
    }
}

/// `jog <degrees> [ms] [top|side]` or `jog spin <rps> [top|side]`. The top
/// is the default.
fn parse_jog(args: &[&str]) -> Result<JogMove, String> {
    let grip = |s: Option<&&str>| match s.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("top") => Ok(JogGrip::Top),
        Some("side") => Ok(JogGrip::Side),
        Some(other) => Err(format!("{other:?} is not top or side")),
    };
    let number = |s: &str, max: f32, what: &str| match s.parse::<f32>() {
        Ok(v) if v.is_finite() && v.abs() <= max => Ok(v),
        _ => Err(format!("{s:?} is not {what} from -{max} to {max}")),
    };
    match args {
        [spin, rps, rest @ ..] if spin.eq_ignore_ascii_case("spin") && rest.len() <= 1 => {
            Ok(JogMove::Spin {
                rps: number(rps, MAX_JOG_RPS, "turns per second")?,
                grip: grip(rest.first())?,
            })
        }
        [degrees, rest @ ..] if rest.len() <= 2 => {
            let degrees = number(degrees, MAX_JOG_DEGREES, "degrees")?;
            let (time, grip_word) = jog_time(rest)?;
            Ok(JogMove::Turn {
                degrees,
                time,
                grip: grip(grip_word)?,
            })
        }
        _ => Err("jog takes <degrees> [ms] [top|side] or spin <rps> [top|side]".into()),
    }
}

/// A turn's time and the word after it, from the words after its degrees:
/// `[ms] [top|side]`. Without milliseconds the turn takes
/// [`DEFAULT_JOG_TIME`], and the first word is the grip.
fn jog_time<'a>(rest: &'a [&'a str]) -> Result<(Duration, Option<&'a &'a str>), String> {
    match rest.first().map(|s| s.parse::<u64>()) {
        Some(Ok(ms)) if ms > MAX_WAIT.as_millis() as u64 => {
            Err(format!("a turn lasts at most {} ms", MAX_WAIT.as_millis()))
        }
        Some(Ok(ms)) => Ok((Duration::from_millis(ms.max(1)), rest.get(1))),
        Some(Err(_)) if rest.len() == 1 => Ok((DEFAULT_JOG_TIME, rest.first())),
        Some(Err(_)) => Err(format!("{:?} is not milliseconds", rest[0])),
        None => Ok((DEFAULT_JOG_TIME, None)),
    }
}

/// The text after the verb and its sub-command, with its spaces kept. For
/// `media attach /tmp/My USB.img` it is `/tmp/My USB.img`.
fn text_after_subcommand(line: &str) -> Option<&str> {
    let after_verb = line.trim().split_once(char::is_whitespace)?.1;
    let rest = after_verb
        .trim_start()
        .split_once(char::is_whitespace)?
        .1
        .trim();
    (!rest.is_empty()).then_some(rest)
}

/// `media`, `media attach <path>` or `media eject`.
fn parse_media(line: &str, args: &[&str]) -> Result<MediaCommand, String> {
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("status") => Ok(MediaCommand::Status),
        Some("eject") => Ok(MediaCommand::Eject),
        Some("attach") => text_after_subcommand(line)
            .map(|p| MediaCommand::Attach(PathBuf::from(p)))
            .ok_or_else(|| "media attach needs the path of a USB image".into()),
        Some(other) => Err(format!(
            "{other:?} is not a media command: attach, eject or status"
        )),
    }
}

/// `mods [list]`, `mods push <path>`, or `mods enable|disable|remove|log
/// <name>`. A path keeps its spaces.
fn parse_mods(line: &str, args: &[&str]) -> Result<ModsCommand, String> {
    let name = || match args {
        [_, name] => Ok(name.to_string()),
        _ => Err(format!("mods {} needs one mod name", args[0])),
    };
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("list") => Ok(ModsCommand::List),
        Some("push") => text_after_subcommand(line)
            .map(|p| ModsCommand::Push(PathBuf::from(p)))
            .ok_or_else(|| "mods push needs the path of a mod folder or archive".into()),
        Some("enable") => Ok(ModsCommand::Enable(name()?, true)),
        Some("disable") => Ok(ModsCommand::Enable(name()?, false)),
        Some("remove") => Ok(ModsCommand::Remove(name()?)),
        Some("log") => Ok(ModsCommand::Log(name()?)),
        Some(other) => Err(format!(
            "{other:?} is not a mods command: list, push, enable, disable, remove or log"
        )),
    }
}

/// `watch log [kernel|unit <name>|mod <name>] [grep <text>]`.
fn parse_log(args: &[&str]) -> Result<Topic, String> {
    let (source, rest) = log_source(args)?;
    let grep = match rest {
        [] => None,
        [g, text @ ..] if g.eq_ignore_ascii_case("grep") && !text.is_empty() => {
            Some(text.join(" "))
        }
        _ => {
            return Err(
                "watch log takes kernel, unit <name> or mod <name>, then grep <text>".into(),
            )
        }
    };
    Ok(Topic::Log { source, grep })
}

/// The journal `watch log` names, and the words after it.
fn log_source<'a>(args: &'a [&'a str]) -> Result<(LogSource, &'a [&'a str]), String> {
    let rest = args.get(2..).unwrap_or_default();
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("kernel") => Ok((LogSource::Kernel, &args[1..])),
        Some("unit") => Ok((LogSource::Unit(log_name("unit", args.get(1))?), rest)),
        Some("mod") => Ok((LogSource::Mod(log_name("mod", args.get(1))?), rest)),
        _ => Ok((LogSource::All, args)),
    }
}

/// A unit or mod name for `watch log`. The guest runs it in a shell command,
/// so only the characters such names use are accepted.
fn log_name(kind: &str, name: Option<&&str>) -> Result<String, String> {
    let ok = |c: char| match kind {
        "mod" => c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c),
        _ => c.is_ascii_alphanumeric() || "@._:-".contains(c),
    };
    match name {
        Some(n) if !n.is_empty() && n.len() <= 128 && n.chars().all(ok) => Ok(n.to_string()),
        Some(n) => Err(format!("{n:?} is not a {kind} name")),
        None => Err(format!("watch log {kind} needs a name")),
    }
}

/// `watch lamps [name...]`, `watch lcd [main|jog] [x0,y0,x1,y1]`,
/// `watch emulation` or `watch log ...`.
fn parse_topic(args: &[&str]) -> Result<Topic, String> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("lamps") => Ok(Topic::Lamps(rest.iter().map(|s| s.to_string()).collect())),
        Some("lcd") => {
            let (screen, rest) = match rest.first().and_then(|s| Screen::parse(s)) {
                Some(screen) => (screen, &rest[1..]),
                None => (Screen::Main, rest),
            };
            let region = match rest {
                [] => None,
                [r] => Some(Region::parse(r)?),
                _ => return Err("watch lcd takes a screen and one region".into()),
            };
            Ok(Topic::Lcd { screen, region })
        }
        Some("emulation") if rest.is_empty() => Ok(Topic::Emulation),
        Some("emulation") => Err("watch emulation takes no filter".into()),
        Some("log") => parse_log(rest),
        _ => Err("watch takes lamps, lcd, emulation or log".into()),
    }
}

/// `unwatch <n>` or `unwatch all`.
fn parse_unwatch(args: &[&str]) -> Result<Option<u32>, String> {
    match arg(args, 0, "unwatch needs a watch number or all")? {
        a if a.eq_ignore_ascii_case("all") => Ok(None),
        a => a
            .parse()
            .map(Some)
            .map_err(|_| format!("{a:?} is not a watch number")),
    }
}

/// `wait boot [ms]` or `wait lamp <name> on|off [ms]`.
fn parse_wait(args: &[&str]) -> Result<Command, String> {
    let (what, rest) = match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("boot") => (WaitFor::Boot, &args[1..]),
        Some("lamp") => {
            let name = arg(args, 1, "wait lamp needs a lamp and a state")?.to_string();
            let want = LampWant::parse(arg(args, 2, "wait lamp needs a lamp and a state")?)?;
            (WaitFor::Lamp { name, want }, &args[3..])
        }
        _ => return Err("wait takes boot or lamp".into()),
    };
    let timeout = match rest.first() {
        None => DEFAULT_WAIT,
        Some(s) => Duration::from_millis(millis(s)?),
    };
    if timeout > MAX_WAIT {
        return Err(format!("a wait lasts at most {} s", MAX_WAIT.as_secs()));
    }
    Ok(Command::Wait { what, timeout })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn press_holds_for_the_default_a_given_time_or_until_release() {
        let press = |line| Command::parse(line).unwrap();
        assert_eq!(
            press("press PLAY"),
            Command::Press {
                bit: "PLAY".into(),
                hold: Some(DEFAULT_HOLD)
            }
        );
        assert_eq!(
            press("PRESS cue 500"),
            Command::Press {
                bit: "cue".into(),
                hold: ms(500)
            }
        );
        assert_eq!(
            press("press 12:0x01 0"),
            Command::Press {
                bit: "12:0x01".into(),
                hold: None
            }
        );
        assert_eq!(
            press("hold SLIP"),
            Command::Press {
                bit: "SLIP".into(),
                hold: None
            }
        );
    }

    #[test]
    fn clear_holds_until_release_unless_given_a_time() {
        assert_eq!(
            Command::parse("clear 12:0x01").unwrap(),
            Command::Clear {
                bit: "12:0x01".into(),
                hold: None
            }
        );
        assert_eq!(
            Command::parse("clear 12:0x01 300").unwrap(),
            Command::Clear {
                bit: "12:0x01".into(),
                hold: ms(300)
            }
        );
    }

    #[test]
    fn touch_takes_fractions_of_the_display() {
        assert_eq!(
            Command::parse("touch 0.25 1").unwrap(),
            Command::Touch {
                x: 0.25,
                y: 1.0,
                hold: Some(DEFAULT_HOLD)
            }
        );
        assert!(Command::parse("touch 500 136").is_err());
        assert!(Command::parse("touch 0.5").is_err());
    }

    #[test]
    fn release_names_a_bit_the_touch_or_everything() {
        let rel = |line| Command::parse(line).unwrap();
        assert_eq!(rel("release ALL"), Command::Release(Target::All));
        assert_eq!(rel("release touch"), Command::Release(Target::Touch));
        assert_eq!(
            rel("release PLAY"),
            Command::Release(Target::Bit("PLAY".into()))
        );
    }

    #[test]
    fn waits_take_a_condition_and_an_optional_timeout() {
        assert_eq!(
            Command::parse("wait boot").unwrap(),
            Command::Wait {
                what: WaitFor::Boot,
                timeout: DEFAULT_WAIT
            }
        );
        assert_eq!(
            Command::parse("wait lamp PLAY on 2500").unwrap(),
            Command::Wait {
                what: WaitFor::Lamp {
                    name: "PLAY".into(),
                    want: LampWant::On
                },
                timeout: Duration::from_millis(2500)
            }
        );
        let want = |s: &str| match Command::parse(&format!("wait lamp SLOT_2 {s}")).unwrap() {
            Command::Wait {
                what: WaitFor::Lamp { want, .. },
                ..
            } => want,
            other => panic!("{other:?}"),
        };
        assert_eq!(want("OFF"), LampWant::Off);
        assert_eq!(want("full"), LampWant::Is("full".into()));
        assert_eq!(
            want("136,240,255"),
            LampWant::Is(serde_json::json!([136, 240, 255]))
        );
        assert_eq!(want("7"), LampWant::Is(7.into()));
        assert!(Command::parse("wait lamp PLAY lit").is_err());
        assert!(Command::parse("wait lamp PLAY 1,2").is_err());
        assert!(Command::parse("wait boot 700000").is_err());
    }

    #[test]
    fn rotary_and_tempo_are_bounded() {
        assert_eq!(Command::parse("rotary -3").unwrap(), Command::Rotary(-3));
        assert!(Command::parse("rotary 0").is_err());
        assert!(Command::parse("rotary 101").is_err());
        assert_eq!(Command::parse("tempo 0.5").unwrap(), Command::Tempo(0.5));
        assert!(Command::parse("tempo 1.5").is_err());
    }

    #[test]
    fn lcd_defaults_to_the_main_screen() {
        assert_eq!(Command::parse("lcd").unwrap(), Command::Lcd(Screen::Main));
        assert_eq!(
            Command::parse("lcd jog").unwrap(),
            Command::Lcd(Screen::Jog)
        );
        assert!(Command::parse("lcd side").is_err());
    }

    #[test]
    fn watch_takes_a_topic_and_its_filters() {
        let w = |line| Command::parse(line).unwrap();
        let region = |s| Some(Region::parse(s).unwrap());
        assert_eq!(w("watch lamps"), Command::Watch(Topic::Lamps(vec![])));
        assert_eq!(
            w("watch lamps PLAY cue"),
            Command::Watch(Topic::Lamps(vec!["PLAY".into(), "cue".into()]))
        );
        assert_eq!(
            w("watch lcd"),
            Command::Watch(Topic::Lcd {
                screen: Screen::Main,
                region: None
            })
        );
        assert_eq!(
            w("watch lcd jog 0,0,0.5,0.5"),
            Command::Watch(Topic::Lcd {
                screen: Screen::Jog,
                region: region("0,0,0.5,0.5")
            })
        );
        assert_eq!(
            w("watch lcd 0,0,1,0.1"),
            Command::Watch(Topic::Lcd {
                screen: Screen::Main,
                region: region("0,0,1,0.1")
            })
        );
        assert_eq!(w("watch emulation"), Command::Watch(Topic::Emulation));
        assert!(Command::parse("watch emulation now").is_err());
        assert!(Command::parse("watch weather").is_err());
        assert_eq!(w("unwatch 3"), Command::Unwatch(Some(3)));
        assert_eq!(w("unwatch all"), Command::Unwatch(None));
        assert!(Command::parse("unwatch x").is_err());
    }

    #[test]
    fn jog_turns_or_spins_by_the_top_or_the_side() {
        let j = |line| match Command::parse(line).unwrap() {
            Command::Jog(m) => m,
            other => panic!("{other:?}"),
        };
        let turn = |degrees, ms, grip| JogMove::Turn {
            degrees,
            time: Duration::from_millis(ms),
            grip,
        };
        assert_eq!(j("jog 90"), turn(90.0, 500, JogGrip::Top));
        assert_eq!(j("jog -45 200"), turn(-45.0, 200, JogGrip::Top));
        assert_eq!(j("jog 30 side"), turn(30.0, 500, JogGrip::Side));
        assert_eq!(j("jog 0 1000 top"), turn(0.0, 1000, JogGrip::Top));
        assert_eq!(
            j("jog spin -2.5"),
            JogMove::Spin {
                rps: -2.5,
                grip: JogGrip::Top
            }
        );
        assert_eq!(
            j("jog spin 1 side"),
            JogMove::Spin {
                rps: 1.0,
                grip: JogGrip::Side
            }
        );
        assert!(Command::parse("jog").is_err());
        assert!(Command::parse("jog 10 fast").is_err());
        assert!(Command::parse("jog 10 x top").is_err());
        assert!(Command::parse("jog spin 100").is_err());
        assert!(Command::parse("jog 99999999").is_err());
        assert!(Command::parse("jog 10 600000").is_ok());
        assert!(Command::parse("jog 10 600001").is_err());
    }

    #[test]
    fn media_setting_and_controls_parse() {
        let c = |line| Command::parse(line).unwrap();
        assert_eq!(c("media"), Command::Media(MediaCommand::Status));
        assert_eq!(
            c("media attach /tmp/My USB.img"),
            Command::Media(MediaCommand::Attach(PathBuf::from("/tmp/My USB.img")))
        );
        assert_eq!(c("media eject"), Command::Media(MediaCommand::Eject));
        assert_eq!(c("reset"), Command::Reset);
        assert!(Command::parse("media attach").is_err());
        assert_eq!(c("setting"), Command::Setting(None));
        assert_eq!(
            c("setting Audio on"),
            Command::Setting(Some(("audio".into(), "on".into())))
        );
        assert!(Command::parse("setting audio").is_err());
        assert_eq!(c("encoder beatloop -2"), Command::Encoder(-2));
        assert_eq!(c("knob vinyl 0.25"), Command::Knob(Knob::VinylSpeed, 0.25));
        assert_eq!(c("knob jog_adjust 1"), Command::Knob(Knob::JogAdjust, 1.0));
        assert!(Command::parse("knob tempo 0.5").is_err());
        assert_eq!(
            c("direction slip_rev"),
            Command::Direction(Lever::SlipReverse)
        );
        assert!(Command::parse("direction up").is_err());
    }

    #[test]
    fn mods_commands_take_a_name_or_a_path() {
        let m = |line| match Command::parse(line).unwrap() {
            Command::Mods(c) => c,
            other => panic!("{other:?}"),
        };
        assert_eq!(m("mods"), ModsCommand::List);
        assert_eq!(m("mods list"), ModsCommand::List);
        assert_eq!(
            m("mods push /Users/me/My Mods/stems "),
            ModsCommand::Push(PathBuf::from("/Users/me/My Mods/stems"))
        );
        assert_eq!(
            m("mods disable stems"),
            ModsCommand::Enable("stems".into(), false)
        );
        assert_eq!(m("mods remove stems"), ModsCommand::Remove("stems".into()));
        assert_eq!(m("mods log stems"), ModsCommand::Log("stems".into()));
        assert!(Command::parse("mods push").is_err());
        assert!(Command::parse("mods enable").is_err());
        assert!(Command::parse("mods enable a b").is_err());
        assert!(Command::parse("mods frob").is_err());
    }

    #[test]
    fn sh_keeps_the_command_as_written() {
        assert_eq!(
            Command::parse("sh  ls -l  /tmp | head").unwrap(),
            Command::Sh("ls -l  /tmp | head".into())
        );
        assert!(Command::parse("sh").is_err());
        assert!(Command::parse("sh   ").is_err());
    }

    #[test]
    fn watch_log_takes_a_source_and_a_grep() {
        let log = |line| match Command::parse(line).unwrap() {
            Command::Watch(Topic::Log { source, grep }) => (source, grep),
            other => panic!("{other:?}"),
        };
        assert_eq!(log("watch log"), (LogSource::All, None));
        assert_eq!(log("watch log kernel"), (LogSource::Kernel, None));
        assert_eq!(
            log("watch log unit EP122.service grep load failed"),
            (
                LogSource::Unit("EP122.service".into()),
                Some("load failed".into())
            )
        );
        assert_eq!(
            log("watch log mod stems"),
            (LogSource::Mod("stems".into()), None)
        );
        assert_eq!(
            log("watch log grep xrun"),
            (LogSource::All, Some("xrun".into()))
        );
        assert!(Command::parse("watch log unit 'x;reboot'").is_err());
        assert!(Command::parse("watch log mod Stems$").is_err());
        assert!(Command::parse("watch log unit").is_err());
        assert!(Command::parse("watch log kernel grep").is_err());
        assert!(Command::parse("watch log kernel extra").is_err());
    }

    #[test]
    fn a_log_source_follows_its_journal() {
        assert_eq!(
            LogSource::Mod("stems".into()).command(),
            "exec journalctl -f -n 0 --no-pager -o short-monotonic -u cdj3k-mod-stems.service"
        );
        assert!(LogSource::Kernel.command().ends_with(" -k"));
    }

    #[test]
    fn an_unknown_or_empty_request_is_an_error() {
        assert!(Command::parse("").is_err());
        assert!(Command::parse("frobnicate").is_err());
    }
}
