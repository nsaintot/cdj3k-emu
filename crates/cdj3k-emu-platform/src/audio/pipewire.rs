//! PipeWire output sinks, for the per-instance Audio Output picker.
//!
//! Compiled everywhere so its parsing is tested everywhere; on a host with no
//! `pw-dump` the enumeration simply finds nothing.
//!
//! Read through `pw-dump`, which every PipeWire installation ships, rather
//! than by linking libpipewire: the picker needs a list a few times a minute,
//! not a client connection, and a subprocess cannot take the app down with it.
//!
//! The guest plays 96 kHz S32. PipeWire resamples to whatever the sink is
//! running at, so the rate is reported here for the same reason it is on
//! macOS — a mismatch is audible and otherwise invisible.

use super::AudioOutDevice;

/// Every audio sink PipeWire currently has.
pub fn enumerate_output_devices() -> Vec<AudioOutDevice> {
    let Some(dump) = pw_dump() else {
        return Vec::new();
    };
    let default = default_sink_name();
    let mut out: Vec<AudioOutDevice> = parse_sinks(&dump)
        .into_iter()
        .map(|mut d| {
            d.is_default = Some(&d.uid) == default.as_ref();
            d
        })
        .collect();
    // Stable order: the picker is a menu, and a list that reshuffles between
    // openings is unusable.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.uid == b.uid);
    out
}

fn pw_dump() -> Option<String> {
    let out = std::process::Command::new("pw-dump")
        .arg("Node")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The sink the desktop currently routes to.
fn default_sink_name() -> Option<String> {
    let out = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Pull the audio sinks out of a `pw-dump` document.
///
/// Scanned rather than parsed as JSON: the three fields wanted are flat
/// strings. Each object is bounded by its `"id":` line, so a field cannot be
/// attributed to the wrong node.
fn parse_sinks(dump: &str) -> Vec<AudioOutDevice> {
    let mut out = Vec::new();
    let mut cur: Option<Partial> = None;

    for line in dump.lines() {
        let t = line.trim();
        if t.starts_with("\"id\":") {
            if let Some(done) = cur.take().and_then(Partial::finish) {
                out.push(done);
            }
            cur = Some(Partial::default());
        }
        let Some(p) = cur.as_mut() else { continue };

        if let Some(v) = field(t, "\"media.class\":") {
            p.is_sink = v == "Audio/Sink";
        } else if let Some(v) = field(t, "\"node.name\":") {
            p.node_name = Some(v.to_string());
        } else if let Some(v) = field(t, "\"node.description\":") {
            p.description = Some(v.to_string());
        } else if let Some(v) = field(t, "\"audio.rate\":") {
            p.rate = v.parse().ok();
        } else if let Some(v) = field(t, "\"clock.rate\":") {
            p.rate = p.rate.or_else(|| v.parse().ok());
        }
    }
    if let Some(done) = cur.and_then(Partial::finish) {
        out.push(done);
    }
    out
}

#[derive(Default)]
struct Partial {
    is_sink: bool,
    node_name: Option<String>,
    description: Option<String>,
    rate: Option<u32>,
}

impl Partial {
    fn finish(self) -> Option<AudioOutDevice> {
        let uid = self.node_name.filter(|_| self.is_sink)?;
        Some(AudioOutDevice {
            name: self.description.unwrap_or_else(|| uid.clone()),
            uid,
            is_default: false,
            sample_rate_hz: self.rate.unwrap_or(0),
        })
    }
}

/// The value of `"key": <value>` on one line, unquoted and without its comma.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim().trim_end_matches(',');
    Some(rest.trim_matches('"'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"
    {
      "id": 40,
      "info": {
        "props": {
          "media.class": "Audio/Source",
          "node.name": "alsa_input.pci-0000_00_1f.3.analog-stereo",
          "node.description": "Built-in Microphone"
        }
      }
    },
    {
      "id": 41,
      "info": {
        "props": {
          "media.class": "Audio/Sink",
          "node.name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
          "node.description": "Built-in Audio Analog Stereo",
          "audio.rate": 48000
        }
      }
    },
    {
      "id": 42,
      "info": {
        "props": {
          "media.class": "Audio/Sink",
          "node.name": "alsa_output.usb-Pioneer_DJ",
          "node.description": "CDJ-3000",
          "clock.rate": 96000
        }
      }
    }
    "#;

    #[test]
    fn only_sinks_are_offered() {
        let sinks = parse_sinks(DUMP);
        assert_eq!(sinks.len(), 2, "the source must not appear: {sinks:?}");
        assert!(sinks.iter().all(|s| s.uid.starts_with("alsa_output")));
    }

    #[test]
    fn the_node_name_is_the_id_and_the_description_is_the_label() {
        let sinks = parse_sinks(DUMP);
        let cdj = sinks.iter().find(|s| s.name == "CDJ-3000").unwrap();
        // out.name= takes the node name, not the description.
        assert_eq!(cdj.uid, "alsa_output.usb-Pioneer_DJ");
    }

    /// A sink's rate is read from `audio.rate`, falling back to `clock.rate`.
    #[test]
    fn the_rate_is_read_from_either_spelling() {
        let sinks = parse_sinks(DUMP);
        let by = |n: &str| sinks.iter().find(|s| s.name == n).unwrap().sample_rate_hz;
        assert_eq!(by("Built-in Audio Analog Stereo"), 48_000);
        assert_eq!(by("CDJ-3000"), 96_000);
    }

    #[test]
    fn a_field_cannot_leak_between_nodes() {
        // The source has no rate; if scanning ignored object boundaries it
        // would inherit the sink's.
        let sinks = parse_sinks(
            r#"
            {
              "id": 1,
              "props": {
                "media.class": "Audio/Sink",
                "node.name": "a",
                "audio.rate": 44100
              }
            },
            {
              "id": 2,
              "props": {
                "media.class": "Audio/Sink",
                "node.name": "b"
              }
            }
            "#,
        );
        assert_eq!(sinks.len(), 2, "{sinks:?}");
        assert_eq!(
            sinks.iter().find(|s| s.uid == "b").unwrap().sample_rate_hz,
            0,
            "the rate must not carry over from the previous node"
        );
    }

    #[test]
    fn nothing_at_all_is_not_a_panic() {
        assert!(parse_sinks("").is_empty());
        assert!(parse_sinks("not json").is_empty());
        assert!(parse_sinks(r#"{ "id": 1 }"#).is_empty());
    }
}
