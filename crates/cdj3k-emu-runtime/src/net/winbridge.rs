//! The text side of Windows bridging: the request the app hands its elevated
//! copies, the files they leave, the parsing of `netsh bridge`, and what to
//! take down when a slot lets its link go.
//!
//! Nothing here touches the host, so it is checked on every platform.

use std::path::{Path, PathBuf};

use cdj3k_emu_platform::net::is_valid_mac;
use cdj3k_emu_platform::net::windows_kind::is_valid_adapter_name;

/// The hidden subcommand of the app that builds the link.
pub const HELPER_FLAG: &str = "--windows-net-helper";
/// The hidden subcommand the helper leaves running, which takes the link down
/// when the slot lets it go.
pub const WATCH_FLAG: &str = "--windows-net-watch";

/// In the instance directory: the helper's verdict.
pub const RESULT_FILE: &str = "winnet.result";

/// The connection name of the tap adapter a slot owns.
pub fn tap_name(instance_id: u32) -> String {
    format!("cdj3k-emu-{instance_id}")
}

/// `{8-4-4-4-12}` with hex digits, the form `netsh bridge` prints.
pub fn is_guid(s: &str) -> bool {
    let Some(inner) = s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return false;
    };
    let parts: Vec<&str> = inner.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Every GUID in `text`, upper-cased, in order of appearance.
pub fn guids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let tail = &rest[start..];
        // 38 bytes: braces plus 36.
        match tail.get(..38).filter(|c| is_guid(c)) {
            Some(g) => {
                out.push(g.to_ascii_uppercase());
                rest = &tail[38..];
            }
            None => rest = &tail[1..],
        }
    }
    out
}

/// Whether `netsh bridge show adapter` lists `name` as bridged; `None` when
/// it is not listed.
///
/// Rows are `IfIndex GUID <name> IsBridged Bridgeable Compatibility`, and a
/// name can hold spaces.
pub fn is_bridged(show_adapter: &str, name: &str) -> Option<bool> {
    show_adapter.lines().find_map(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 6 || !words[0].bytes().all(|b| b.is_ascii_digit()) || !is_guid(words[1]) {
            return None;
        }
        let tail = words.len() - 3;
        (words[2..tail].join(" ").eq_ignore_ascii_case(name))
            .then(|| words[tail].eq_ignore_ascii_case("yes"))
    })
}

/// The names `netsh bridge show adapter` lists as bridged.
pub fn bridged_names(show_adapter: &str) -> Vec<String> {
    show_adapter
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() < 6 || !words[0].bytes().all(|b| b.is_ascii_digit()) || !is_guid(words[1])
            {
                return None;
            }
            let tail = words.len() - 3;
            words[tail]
                .eq_ignore_ascii_case("yes")
                .then(|| words[2..tail].join(" "))
        })
        .collect()
}

/// Whether `name` is a slot's tap.
pub fn is_tap_name(name: &str) -> bool {
    name.strip_prefix("cdj3k-emu-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// What letting a slot's link go takes down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Teardown {
    /// The bridge to take `tap` out of, when it is in one.
    pub remove_from: Option<String>,
    /// The bridge to destroy: the one the app made, once no slot's tap is
    /// left in it.
    pub destroy: Option<String>,
}

/// The teardown for `tap`, given `netsh bridge show adapter`, `netsh bridge
/// list` and the bridge the app recorded making (`owned`).
pub fn teardown(show_adapter: &str, bridge_list: &str, owned: Option<&str>, tap: &str) -> Teardown {
    let bridge = guids(bridge_list).into_iter().next();
    let bridged = bridged_names(show_adapter);
    let tap_bridged = bridged.iter().any(|n| n.eq_ignore_ascii_case(tap));
    let others = bridged
        .iter()
        .any(|n| is_tap_name(n) && !n.eq_ignore_ascii_case(tap));
    let ours = bridge
        .as_deref()
        .zip(owned)
        .is_some_and(|(b, o)| b.eq_ignore_ascii_case(o.trim()));
    if ours && !others {
        return Teardown {
            remove_from: None,
            destroy: bridge,
        };
    }
    Teardown {
        remove_from: bridge.filter(|_| tap_bridged),
        destroy: None,
    }
}

/// A path that survives `cmd.exe /C` inside double quotes.
pub fn is_cmd_safe_path(p: &Path) -> bool {
    p.to_str().is_some_and(|s| {
        !s.is_empty()
            && !s
                .chars()
                .any(|c| matches!(c, '"' | '%' | '&' | '|' | '<' | '>' | '^' | '\n' | '\r'))
    })
}

/// What the elevated copies of the app are asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The instance directory, where the lease's files and the result are.
    pub dir: PathBuf,
    pub tap: String,
    pub nic: String,
    /// The tap driver's INF, when the adapter has to be made.
    pub inf: Option<PathBuf>,
    /// The app process the link is for.
    pub pid: u32,
    /// The claim this link was made under ([`super::lease`]).
    pub claim: String,
    /// The slot's MAC, which the guest wears; the tap's own is derived from
    /// it ([`tap_mac`]).
    pub mac: String,
}

impl Request {
    /// The command line for `cmd.exe /C`: the whole line in one more pair of
    /// quotes, which `cmd` strips, so every quoted part inside survives.
    pub fn cmdline(&self, exe: &Path) -> String {
        format!("\"{}\"", self.command(exe, HELPER_FLAG))
    }

    /// `exe` with `flag` and the request.
    pub fn command(&self, exe: &Path, flag: &str) -> String {
        format!("\"{}\" {flag} {}", exe.display(), self.args().map(|a| format!("\"{a}\"")).join(" "))
    }

    /// The request as the arguments after the flag.
    pub fn args(&self) -> [String; 7] {
        [
            self.dir.display().to_string(),
            self.tap.clone(),
            self.nic.clone(),
            self.inf
                .as_deref()
                .map_or_else(|| "-".to_string(), |p| p.display().to_string()),
            self.pid.to_string(),
            self.claim.clone(),
            self.mac.clone(),
        ]
    }

    /// The request in `args` (argv after the flag), or `None` when it is
    /// malformed or names anything outside the grammars.
    pub fn parse(args: &[String]) -> Option<Self> {
        let [dir, tap, nic, inf, pid, claim, mac] = args else {
            return None;
        };
        let dir = PathBuf::from(dir);
        let inf = (inf != "-").then(|| PathBuf::from(inf));
        let ok = is_cmd_safe_path(&dir)
            && inf.as_deref().is_none_or(is_cmd_safe_path)
            && is_valid_adapter_name(tap)
            && is_valid_adapter_name(nic)
            && super::lease::is_claim(claim)
            && is_valid_mac(mac);
        ok.then_some(Self {
            dir,
            tap: tap.clone(),
            nic: nic.clone(),
            inf,
            pid: pid.parse().ok()?,
            claim: claim.clone(),
            mac: mac.clone(),
        })
    }
}

/// The tap adapter's own MAC, as `NetworkAddress` takes it (12 hex digits),
/// for a slot whose guest wears `guest_mac`.
///
/// The Network Bridge takes the tap's MAC, so the host's DHCP lease follows
/// it. It is the guest's with the first octet set to 02 (locally
/// administered, unicast) and must differ from the guest's: a bridge passes
/// frames for a port's own address up to the host.
pub fn tap_mac(guest_mac: &str) -> Option<String> {
    let octets: Vec<&str> = guest_mac.split([':', '-']).collect();
    if octets.len() != 6 || !octets.iter().all(|o| o.len() == 2 && o.bytes().all(|b| b.is_ascii_hexdigit())) {
        return None;
    }
    // 06 is the next locally administered unicast prefix, for a guest that
    // already wears 02.
    let prefix = if octets[0].eq_ignore_ascii_case("02") { "06" } else { "02" };
    Some(format!("{prefix}{}", octets[1..].concat().to_ascii_uppercase()))
}

/// What the elevated copy leaves in `winnet.result`.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The tap and the NIC are both in the bridge.
    Done,
    /// The tap adapter could not be made.
    TapFailed(String),
    /// `netsh` refused the bridge.
    BridgeFailed(String),
}

impl Outcome {
    pub fn encode(&self) -> String {
        match self {
            Self::Done => "ok\n".to_string(),
            Self::TapFailed(m) => format!("tap\n{m}"),
            Self::BridgeFailed(m) => format!("bridge\n{m}"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (head, body) = text.split_once('\n').unwrap_or((text, ""));
        let body = body.trim();
        match head.trim() {
            "ok" => Some(Self::Done),
            "tap" => Some(Self::TapFailed(body.to_string())),
            "bridge" => Some(Self::BridgeFailed(body.to_string())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: &str = "{12345678-ABCD-EF01-2345-6789ABCDEF03}";

    /// `netsh bridge show adapter` after `create` joined only the tap.
    const SHOW_ADAPTER: &str = "
----------------------------------------------------------------------------------------------------------------
 IfIndex GUID                                    Adapter Name                 IsBridged Bridgeable Compatibility
----------------------------------------------------------------------------------------------------------------
 24      {2C14D3C3-9EAB-4D7E-A9BB-154524B24B51}  cdj3k-emu-1                     Yes    Yes         disabled
 7       {6E0F2882-28E8-488D-AC8B-A41540BA2DE7}  Ethernet                         No    Yes         disabled
 9       {6E0F2882-28E8-488D-AC8B-A41540BA2DE8}  Ethernet 2                       No    No          disabled
----------------------------------------------------------------------------------------------------------------
";

    #[test]
    fn reads_bridge_membership_by_adapter_name() {
        assert_eq!(is_bridged(SHOW_ADAPTER, "cdj3k-emu-1"), Some(true));
        assert_eq!(is_bridged(SHOW_ADAPTER, "ethernet"), Some(false));
        assert_eq!(is_bridged(SHOW_ADAPTER, "Ethernet 2"), Some(false));
        assert_eq!(is_bridged(SHOW_ADAPTER, "Wi-Fi"), None);
    }

    const G2: &str = "{aaaaaaaa-0000-1111-2222-333333333333}";

    #[test]
    fn guids_are_found_in_netsh_output() {
        let text = format!("Bridges:\r\n  {G1}\r\n  junk {{not-a-guid}} {G2}\r\n");
        assert_eq!(guids(&text), vec![G1.to_string(), G2.to_ascii_uppercase()]);
        assert!(guids("none here {").is_empty());
        assert!(is_guid(G1));
        assert!(!is_guid("{1234}"));
        assert!(!is_guid("12345678-ABCD-EF01-2345-6789ABCDEF03"));
    }

    fn request() -> Request {
        Request {
            dir: PathBuf::from(r"C:\Users\Jo Doe\AppData\Local\cdj3k-emu\instance-0"),
            tap: "cdj3k-emu-0".into(),
            nic: "Ethernet 2".into(),
            inf: Some(PathBuf::from(r"C:\Program Files\cdj3k-emu\tap-windows6\OemVista.inf")),
            pid: 4356,
            claim: "4356-1790933216403".into(),
            mac: "fe:a7:61:67:79:84".into(),
        }
    }

    /// `cmd /C` strips the outer pair of quotes; what is left is the command.
    #[test]
    fn the_command_line_is_wrapped_for_cmd() {
        let line = request().cmdline(Path::new(r"C:\Program Files\cdj3k-emu\cdj3k-emu.exe"));
        assert!(line.starts_with("\"\"C:\\Program Files\\cdj3k-emu\\cdj3k-emu.exe\" --windows-net-helper "));
        assert!(line.ends_with("OemVista.inf\" \"4356\" \"4356-1790933216403\" \"fe:a7:61:67:79:84\"\""), "{line}");
        assert!(line.contains("\"Ethernet 2\""), "{line}");
    }

    #[test]
    fn a_request_parses_back_and_refuses_hostile_parts() {
        let r = request();
        let args: Vec<String> = r.args().into();
        assert_eq!(Request::parse(&args), Some(r));

        let with = |i: usize, v: &str| {
            let mut a = args.clone();
            a[i] = v.into();
            a
        };
        assert!(Request::parse(&with(1, "x\" & calc & \"")).is_none());
        assert!(Request::parse(&with(2, "eth0; id")).is_none());
        assert!(Request::parse(&with(0, r"C:\%TEMP%")).is_none());
        assert!(Request::parse(&with(4, "-1")).is_none());
        assert!(Request::parse(&with(5, "4356 & calc")).is_none());
        assert!(Request::parse(&with(6, "fe:a7:61:67:79:84 & calc")).is_none());
        assert!(Request::parse(&args[..6]).is_none());
        assert_eq!(Request::parse(&with(3, "-")).unwrap().inf, None);
    }

    #[test]
    fn the_tap_mac_is_the_guests_made_local() {
        assert_eq!(tap_mac("fe:a7:61:67:79:84").as_deref(), Some("02A761677984"));
        assert_eq!(tap_mac("02:a7:61:67:79:84").as_deref(), Some("06A761677984"));
        assert_eq!(tap_mac("FE-A7-61-67-79-84").as_deref(), Some("02A761677984"));
        assert_eq!(tap_mac("fe:a7:61"), None);
        assert_eq!(tap_mac("fe:a7:61:67:79:zz"), None);
    }

    #[test]
    fn an_outcome_round_trips() {
        for o in [
            Outcome::Done,
            Outcome::TapFailed("no driver".into()),
            Outcome::BridgeFailed("Not supported".into()),
        ] {
            assert_eq!(Outcome::parse(&o.encode()), Some(o));
        }
        assert_eq!(Outcome::parse(""), None);
        assert_eq!(Outcome::parse("maybe\nx"), None);
    }

    /// Two slots' taps and the NIC in the bridge the app made.
    const TWO_SLOTS: &str = "
 5       {4C5896F1-EDEF-4A91-AFCE-3C46E5993C32}  cdj3k-emu-1                     Yes    Yes         disabled
 8       {6E0F2882-28E8-488D-AC8B-A41540BA2DE7}  Ethernet                        Yes    Yes         disabled
 11      {95287737-7784-4932-BAF9-E5F158D949B4}  cdj3k-emu-2                     Yes    Yes         disabled
";

    #[test]
    fn bridged_names_are_the_yes_rows() {
        assert_eq!(bridged_names(TWO_SLOTS), ["cdj3k-emu-1", "Ethernet", "cdj3k-emu-2"]);
        assert_eq!(bridged_names(SHOW_ADAPTER), ["cdj3k-emu-1"]);
    }

    #[test]
    fn the_last_slot_out_destroys_the_bridge_the_app_made() {
        let list = format!("Bridges:\n  {G1}\n");
        let one_left = TWO_SLOTS.replace(
            "cdj3k-emu-2                     Yes",
            "cdj3k-emu-2                      No",
        );
        assert_eq!(
            teardown(TWO_SLOTS, &list, Some(G1), "cdj3k-emu-2"),
            Teardown { remove_from: Some(G1.into()), destroy: None }
        );
        assert_eq!(
            teardown(&one_left, &list, Some(G1), "cdj3k-emu-1"),
            Teardown { remove_from: None, destroy: Some(G1.into()) }
        );
    }

    #[test]
    fn a_bridge_the_app_did_not_make_is_only_left() {
        let list = format!("Bridges:\n  {G1}\n");
        assert_eq!(
            teardown(TWO_SLOTS, &list, None, "cdj3k-emu-1"),
            Teardown { remove_from: Some(G1.into()), destroy: None }
        );
        assert_eq!(
            teardown(TWO_SLOTS, &list, Some(G2), "cdj3k-emu-1"),
            Teardown { remove_from: Some(G1.into()), destroy: None }
        );
        assert_eq!(
            teardown(TWO_SLOTS, &list, None, "cdj3k-emu-3"),
            Teardown { remove_from: None, destroy: None }
        );
        assert_eq!(
            teardown("", "", Some(G1), "cdj3k-emu-1"),
            Teardown { remove_from: None, destroy: None }
        );
    }

    #[test]
    fn tap_names_have_a_fixed_shape() {
        assert!(is_tap_name("cdj3k-emu-12"));
        assert!(!is_tap_name("cdj3k-emu-"));
        assert!(!is_tap_name("Ethernet"));
    }

    #[test]
    fn the_tap_is_named_for_its_slot() {
        assert_eq!(tap_name(0), "cdj3k-emu-0");
        assert_eq!(tap_name(3), "cdj3k-emu-3");
        assert!(is_valid_adapter_name(&tap_name(12)));
    }
}
