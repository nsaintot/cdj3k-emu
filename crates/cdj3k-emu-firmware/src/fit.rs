//! Flattened Image Tree (U-Boot FIT) reader.
//!
//! Models that ship `images/boot.img` as a FIT with external data: a small
//! device tree describes `/images/{fdt,kernel,ramdisk,resource}`, each with a
//! `data-position` (or `data-offset`) and `data-size` into the file and a
//! SHA-256 `hash` node. [`image`] returns one of those payloads, its hash
//! checked.

use sha2::{Digest, Sha256};

use crate::extract::ExtractError;

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

/// The payload of `/images/<name>` in the FIT `fit`, checked against the
/// SHA-256 its `hash` (or `hash-N`) node records; an image with no SHA-256
/// hash node is refused.
pub fn image<'a>(fit: &'a [u8], name: &str) -> Result<&'a [u8], ExtractError> {
    let tree = Tree::parse(fit)?;
    let node = format!("/images/{name}");
    let prop = |p: &str| tree.prop(&node, p);
    let be32 =
        |v: &[u8]| -> Option<usize> { Some(u32::from_be_bytes(v.try_into().ok()?) as usize) };

    let data = if let Some(inline) = prop("data") {
        inline
    } else {
        let size = prop("data-size")
            .and_then(be32)
            .ok_or_else(|| missing(&node, "data-size"))?;
        // `data-position` is from the start of the file; `data-offset` from
        // the end of the tree, rounded up to four bytes.
        let start = match (prop("data-position"), prop("data-offset")) {
            (Some(p), _) => be32(p),
            (None, Some(o)) => be32(o).map(|o| tree.total_size.next_multiple_of(4) + o),
            (None, None) => None,
        }
        .ok_or_else(|| missing(&node, "data-position"))?;
        start
            .checked_add(size)
            .and_then(|end| fit.get(start..end))
            .ok_or(ExtractError::BadIso(
                "FIT image data runs past the end of the file",
            ))?
    };

    let want = tree
        .children(&node)
        .filter(|child| {
            child
                .rsplit('/')
                .next()
                .is_some_and(|n| n == "hash" || n.starts_with("hash-"))
        })
        .filter(|child| tree.prop(child, "algo") == Some(b"sha256\0"))
        .find_map(|child| tree.prop(child, "value"))
        .ok_or_else(|| missing(&node, "sha256 hash node"))?;
    if Sha256::digest(data).as_slice() != want {
        return Err(ExtractError::BadIso("FIT image does not match its sha256"));
    }
    Ok(data)
}

fn missing(node: &str, prop: &str) -> ExtractError {
    ExtractError::FileNotFound(format!("{node} has no {prop}"))
}

/// Every property of the tree, keyed by its node's path.
struct Tree<'a> {
    total_size: usize,
    props: Vec<(String, &'a str, &'a [u8])>,
}

impl<'a> Tree<'a> {
    fn parse(fit: &'a [u8]) -> Result<Self, ExtractError> {
        let bad = ExtractError::BadIso("not a flattened device tree");
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_be_bytes(fit.get(at..at + 4)?.try_into().ok()?))
        };
        if word(0) != Some(FDT_MAGIC) {
            return Err(bad);
        }
        let header = |at: usize| {
            word(at)
                .map(|w| w as usize)
                .ok_or(ExtractError::BadIso("truncated FDT header"))
        };
        let total_size = header(4)?;
        let struct_off = header(8)?;
        let strings_off = header(12)?;
        let strings = fit
            .get(strings_off..total_size.min(fit.len()))
            .ok_or(ExtractError::BadIso("FDT strings block out of range"))?;
        let cstr = |b: &'a [u8]| -> &'a str {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            std::str::from_utf8(&b[..end]).unwrap_or("")
        };

        let mut path: Vec<&str> = Vec::new();
        let mut props = Vec::new();
        let mut at = struct_off;
        loop {
            let token = word(at).ok_or(ExtractError::BadIso("FDT structure runs past the file"))?;
            at += 4;
            match token {
                FDT_BEGIN_NODE => {
                    let name = cstr(fit.get(at..).unwrap_or_default());
                    path.push(name);
                    at = (at + name.len() + 1).next_multiple_of(4);
                }
                FDT_END_NODE => {
                    path.pop();
                }
                FDT_PROP => {
                    let (len, name_off) = (header(at)?, header(at + 4)?);
                    at += 8;
                    let value = fit
                        .get(at..at + len)
                        .ok_or(ExtractError::BadIso("FDT property runs past the file"))?;
                    let name = cstr(strings.get(name_off..).unwrap_or_default());
                    // The root node's name is empty, so the path starts "/".
                    let node = if path.len() <= 1 {
                        "/".to_string()
                    } else {
                        path.join("/")
                    };
                    props.push((node, name, value));
                    at = (at + len).next_multiple_of(4);
                }
                FDT_NOP => {}
                FDT_END => break,
                _ => return Err(ExtractError::BadIso("unknown FDT token")),
            }
        }
        Ok(Self { total_size, props })
    }

    /// The paths of `node`'s direct children that carry a property.
    fn children<'s>(&'s self, node: &'s str) -> impl Iterator<Item = &'s str> + 's {
        let mut seen: Vec<&str> = Vec::new();
        self.props.iter().filter_map(move |(n, _, _)| {
            let name = n.strip_prefix(node)?.strip_prefix('/')?;
            (!name.contains('/') && !seen.contains(&n.as_str())).then(|| {
                seen.push(n);
                n.as_str()
            })
        })
    }

    fn prop(&self, node: &str, name: &str) -> Option<&'a [u8]> {
        self.props
            .iter()
            .find(|(n, p, _)| n == node && *p == name)
            .map(|(_, _, v)| *v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FIT with external data: `/images/ramdisk` at `data-position`, hashed.
    /// The hash node is named `hash_node`; empty, there is none.
    fn fit_with(payload: &[u8], hash: [u8; 32], offset_style: bool, hash_node: &str) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut name = |s: &str| {
            let off = strings.len() as u32;
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
            off
        };
        let (n_size, n_pos, n_off, n_algo, n_value) = (
            name("data-size"),
            name("data-position"),
            name("data-offset"),
            name("algo"),
            name("value"),
        );

        let mut st = Vec::new();
        let begin = |st: &mut Vec<u8>, n: &str| {
            st.extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
            st.extend_from_slice(n.as_bytes());
            st.push(0);
            st.resize(st.len().next_multiple_of(4), 0);
        };
        let prop = |st: &mut Vec<u8>, off: u32, v: &[u8]| {
            st.extend_from_slice(&FDT_PROP.to_be_bytes());
            st.extend_from_slice(&(v.len() as u32).to_be_bytes());
            st.extend_from_slice(&off.to_be_bytes());
            st.extend_from_slice(v);
            st.resize(st.len().next_multiple_of(4), 0);
        };
        let end = |st: &mut Vec<u8>| st.extend_from_slice(&FDT_END_NODE.to_be_bytes());

        const TREE: usize = 1024;
        begin(&mut st, "");
        begin(&mut st, "images");
        begin(&mut st, "ramdisk");
        prop(&mut st, n_size, &(payload.len() as u32).to_be_bytes());
        if offset_style {
            prop(&mut st, n_off, &8u32.to_be_bytes());
        } else {
            prop(&mut st, n_pos, &(TREE as u32 + 8).to_be_bytes());
        }
        if !hash_node.is_empty() {
            begin(&mut st, hash_node);
            prop(&mut st, n_value, &hash);
            prop(&mut st, n_algo, b"sha256\0");
            end(&mut st);
        }
        end(&mut st);
        end(&mut st);
        end(&mut st);
        st.extend_from_slice(&FDT_END.to_be_bytes());

        let struct_off = 40usize;
        let strings_off = struct_off + st.len();
        let mut fit = Vec::new();
        for w in [
            FDT_MAGIC,
            TREE as u32,
            struct_off as u32,
            strings_off as u32,
            0,
            17,
            16,
            0,
            strings.len() as u32,
            st.len() as u32,
        ] {
            fit.extend_from_slice(&w.to_be_bytes());
        }
        fit.extend_from_slice(&st);
        fit.extend_from_slice(&strings);
        fit.resize(TREE + 8, 0);
        fit.extend_from_slice(payload);
        fit
    }

    #[test]
    fn reads_an_external_image_by_position_and_by_offset() {
        let payload = b"\x1f\x8b\x08 rootfs.cpio";
        let hash: [u8; 32] = Sha256::digest(payload).into();
        for offset_style in [false, true] {
            let fit = fit_with(payload, hash, offset_style, "hash");
            assert_eq!(image(&fit, "ramdisk").unwrap(), payload);
        }
    }

    #[test]
    fn refuses_a_payload_that_fails_its_hash() {
        let fit = fit_with(b"payload", [0; 32], false, "hash");
        assert!(image(&fit, "ramdisk").is_err());
    }

    #[test]
    fn checks_a_numbered_hash_node_and_refuses_none() {
        let payload = b"payload";
        let hash: [u8; 32] = Sha256::digest(payload).into();
        assert_eq!(
            image(&fit_with(payload, hash, false, "hash-1"), "ramdisk").unwrap(),
            payload
        );
        assert!(image(&fit_with(payload, [0; 32], false, "hash-1"), "ramdisk").is_err());
        assert!(matches!(
            image(&fit_with(payload, hash, false, ""), "ramdisk"),
            Err(ExtractError::FileNotFound(_))
        ));
    }

    #[test]
    fn names_a_missing_image() {
        let payload = b"x";
        let fit = fit_with(payload, Sha256::digest(payload).into(), false, "hash");
        assert!(matches!(
            image(&fit, "kernel"),
            Err(ExtractError::FileNotFound(_))
        ));
        assert!(image(b"not a tree at all", "ramdisk").is_err());
    }
}
