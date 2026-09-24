//! The guest gadget's USB identity, as `guest/pc_link_bridge/gadget.c` reads
//! it back from configfs and sends it in the IDENTITY frame.
//!
//! The firmware's `usb_gadget.sh` defines it, so every macOS endpoint built
//! from it carries the identity of the model being emulated: VID/PID, strings
//! and the HID report descriptor all come from the deck, none from here.

/// One gadget's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GadgetIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
    /// `bcdDevice`, the firmware release (e.g. `0x0320` for 3.20).
    pub bcd_device: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    pub report_descriptor: Vec<u8>,
}

/// Why an IDENTITY payload was rejected.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gadget identity: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

fn hex_u16(key: &str, v: &str) -> Result<u16, ParseError> {
    let digits = v.strip_prefix("0x").unwrap_or(v);
    u16::from_str_radix(digits, 16).map_err(|_| ParseError(format!("{key}={v:?} is not hex")))
}

fn hex_bytes(v: &str) -> Result<Vec<u8>, ParseError> {
    let digits = v.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return Err(ParseError("report_desc has an odd number of digits".into()));
    }
    digits
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |c: u8| (c as char).to_digit(16);
            match (nibble(pair[0]), nibble(pair[1])) {
                (Some(hi), Some(lo)) => Ok((hi << 4 | lo) as u8),
                _ => Err(ParseError(format!(
                    "report_desc byte {:?} is not hex",
                    String::from_utf8_lossy(pair)
                ))),
            }
        })
        .collect()
}

impl GadgetIdentity {
    /// Parse the `key=value` lines of an IDENTITY payload.  Every key is
    /// required; unknown keys are ignored.
    pub fn parse(payload: &[u8]) -> Result<Self, ParseError> {
        let text =
            std::str::from_utf8(payload).map_err(|_| ParseError("payload is not UTF-8".into()))?;
        let mut get = std::collections::BTreeMap::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                get.insert(k, v);
            }
        }
        let need = |k: &str| {
            get.get(k)
                .copied()
                .ok_or_else(|| ParseError(format!("missing {k}")))
        };
        let report_descriptor = hex_bytes(need("report_desc")?)?;
        if report_descriptor.is_empty() {
            return Err(ParseError("report_desc is empty".into()));
        }
        Ok(Self {
            vendor_id: hex_u16("idVendor", need("idVendor")?)?,
            product_id: hex_u16("idProduct", need("idProduct")?)?,
            bcd_device: hex_u16("bcdDevice", need("bcdDevice")?)?,
            manufacturer: need("manufacturer")?.to_string(),
            product: need("product")?.to_string(),
            serial: need("serialnumber")?.to_string(),
            report_descriptor,
        })
    }

    /// Usage page and usage of the descriptor's top-level collection: the
    /// last Usage Page and Usage items before the first Collection.
    pub fn primary_usage(&self) -> Option<(u32, u32)> {
        let d = &self.report_descriptor;
        let (mut page, mut usage) = (None, None);
        let mut i = 0;
        while i < d.len() {
            let prefix = d[i];
            // Long items (0xFE) carry their size in the next byte.
            if prefix == 0xfe {
                i += 3 + *d.get(i + 1)? as usize;
                continue;
            }
            let size = match prefix & 0x03 {
                3 => 4,
                n => n as usize,
            };
            let data = d.get(i + 1..i + 1 + size)?;
            let value = data
                .iter()
                .rev()
                .fold(0u32, |acc, b| (acc << 8) | *b as u32);
            match prefix & 0xfc {
                0x04 => page = Some(value),           // Usage Page (global)
                0x08 => usage = Some(value),          // Usage (local)
                0xa0 => return Some((page?, usage?)), // Collection
                _ => {}
            }
            i += 1 + size;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CDJ-3000X gadget, as its firmware's usb_gadget.sh writes it.
    const CDJ3000X_PAYLOAD: &str = concat!(
        "idVendor=0x2b73\n",
        "idProduct=0x004e\n",
        "bcdDevice=0x0140\n",
        "manufacturer=AlphaTheta Corporation\n",
        "product=CDJ-3000X\n",
        "serialnumber=DJMP000004EH\n",
        "report_desc=06a0ff0901a1010902a10006a1ff090309041580257f350045ff7508960001",
        "8102090509061580257f350045ff75089600049102c0c0\n",
    );

    #[test]
    fn parses_the_cdj3000x_gadget() {
        let id = GadgetIdentity::parse(CDJ3000X_PAYLOAD.as_bytes()).unwrap();
        assert_eq!(id.vendor_id, 0x2b73);
        assert_eq!(id.product_id, 0x004e);
        assert_eq!(id.bcd_device, 0x0140);
        assert_eq!(id.manufacturer, "AlphaTheta Corporation");
        assert_eq!(id.product, "CDJ-3000X");
        assert_eq!(id.serial, "DJMP000004EH");
        assert_eq!(id.report_descriptor.len(), 54);
        assert_eq!(id.primary_usage(), Some((0xffa0, 0x01)));
    }

    #[test]
    fn a_non_ascii_report_desc_is_an_error() {
        let payload = CDJ3000X_PAYLOAD.replace("06a0ff", "\u{e9}a0ff");
        let err = GadgetIdentity::parse(payload.as_bytes()).unwrap_err();
        assert!(err.0.contains("not hex"), "{err}");
    }

    #[test]
    fn a_missing_key_is_an_error() {
        let err = GadgetIdentity::parse(b"idVendor=0x2b73\n").unwrap_err();
        assert!(err.0.contains("missing"), "{err}");
    }

    #[test]
    fn primary_usage_skips_items_before_the_collection() {
        // Usage Page (0x0c), Usage (0x01), Logical Min, Collection.
        let id = GadgetIdentity {
            vendor_id: 0,
            product_id: 0,
            bcd_device: 0,
            manufacturer: String::new(),
            product: String::new(),
            serial: String::new(),
            report_descriptor: vec![0x05, 0x0c, 0x09, 0x01, 0x15, 0x00, 0xa1, 0x01, 0xc0],
        };
        assert_eq!(id.primary_usage(), Some((0x0c, 0x01)));
    }
}
