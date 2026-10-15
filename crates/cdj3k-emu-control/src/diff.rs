//! Pixel comparison of two captures of the same size.
//!
//! A pixel differs when any channel differs by more than the threshold.
//! Masked regions are left out, for parts of a screen that always move, such
//! as the waveform, the playhead or the clock. A region of interest leaves out
//! everything outside it.

use std::path::Path;

use serde_json::json;

use crate::image::Image;

/// A rectangle in fractions of the image, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Region {
    /// Parse `x0,y0,x1,y1` or `x0 y0 x1 y1`.
    pub fn parse(s: &str) -> Result<Region, String> {
        let v: Vec<f32> = s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .map(|w| w.parse::<f32>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("{s:?} is not four numbers"))?;
        let [x0, y0, x1, y1] = v[..] else {
            return Err(format!("{s:?} is not four numbers"));
        };
        let unit = |n: f32| (0.0..=1.0).contains(&n);
        if !(unit(x0) && unit(y0) && unit(x1) && unit(y1)) || x0 >= x1 || y0 >= y1 {
            return Err(format!(
                "{s:?} is not a region: x0 < x1 and y0 < y1, all from 0 to 1"
            ));
        }
        Ok(Region { x0, y0, x1, y1 })
    }

    fn contains(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        let fx = (x as f32 + 0.5) / w as f32;
        let fy = (y as f32 + 0.5) / h as f32;
        fx >= self.x0 && fx < self.x1 && fy >= self.y0 && fy < self.y1
    }
}

/// Read a mask file: one region per line as `x0 y0 x1 y1`, with `#`
/// comments and blank lines skipped.
pub fn read_masks(path: &Path) -> Result<Vec<Region>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .enumerate()
        .filter_map(|(n, line)| {
            let line = line.split('#').next().unwrap_or("").trim();
            (!line.is_empty()).then(|| {
                Region::parse(line).map_err(|e| format!("{} line {}: {e}", path.display(), n + 1))
            })
        })
        .collect()
}

/// The mask file that goes with a reference image: `ref.png` takes
/// `ref.mask` when it exists.
pub fn sidecar_masks(reference: &Path) -> Result<Vec<Region>, String> {
    let path = reference.with_extension("mask");
    if path.is_file() {
        read_masks(&path)
    } else {
        Ok(Vec::new())
    }
}

/// Which pixels [`compare`] looks at, and how far apart they may be.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The largest channel difference that still counts as equal.
    pub threshold: u8,
    pub masks: Vec<Region>,
    /// When set, only the pixels inside it are compared.
    pub region: Option<Region>,
}

impl Options {
    fn masked(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        self.region.is_some_and(|r| !r.contains(x, y, w, h))
            || self.masks.iter().any(|m| m.contains(x, y, w, h))
    }

    fn masks_anything(&self) -> bool {
        self.region.is_some() || !self.masks.is_empty()
    }

    fn differs(&self, a: &[u8], b: &[u8]) -> bool {
        a.iter()
            .zip(b)
            .any(|(p, q)| p.abs_diff(*q) > self.threshold)
    }
}

/// The result of [`compare`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    /// The number of pixels compared, which leaves out the masked ones.
    pub compared: u64,
    pub changed: u64,
    /// The box around every changed pixel: x, y, width, height.
    pub bbox: Option<[u32; 4]>,
}

impl Report {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "width": self.width,
            "height": self.height,
            "compared": self.compared,
            "changed": self.changed,
            "bbox": self.bbox,
        })
    }
}

/// Compare `actual` with `reference`. The images must be the same size.
pub fn compare(reference: &Image, actual: &Image, opts: &Options) -> Result<Report, String> {
    same_size(reference, actual)?;
    let (w, h) = (actual.width, actual.height);
    let mut report = Report {
        width: w,
        height: h,
        compared: 0,
        changed: 0,
        bbox: None,
    };
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    let row_bytes = w as usize * 4;
    for y in 0..h {
        let row = y as usize * row_bytes..(y as usize + 1) * row_bytes;
        // Most rows of two captures of one screen are equal.
        if reference.rgba[row.clone()] == actual.rgba[row.clone()] {
            report.compared += if opts.masks_anything() {
                (0..w).filter(|&x| !opts.masked(x, y, w, h)).count() as u64
            } else {
                w as u64
            };
            continue;
        }
        for x in 0..w {
            if opts.masked(x, y, w, h) {
                continue;
            }
            report.compared += 1;
            let i = row.start + x as usize * 4;
            if opts.differs(&reference.rgba[i..i + 4], &actual.rgba[i..i + 4]) {
                report.changed += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if report.changed > 0 {
        report.bbox = Some([x0, y0, x1 - x0 + 1, y1 - y0 + 1]);
    }
    Ok(report)
}

/// A picture of the difference: `actual` dimmed to grey, changed pixels in
/// red and masked regions tinted blue.
pub fn diff_image(reference: &Image, actual: &Image, opts: &Options) -> Result<Image, String> {
    same_size(reference, actual)?;
    let mut out = vec![0u8; actual.rgba.len()];
    let w = actual.width as usize;
    for_each_pixel(reference, actual, opts, |x, y, kind| {
        let i = (y as usize * w + x as usize) * 4;
        let p = &actual.rgba[i..i + 4];
        let grey = ((p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10 / 3) as u8;
        let px = match kind {
            Pixel::Same => [grey, grey, grey, 0xff],
            Pixel::Masked => [grey / 2, grey / 2, grey / 2 + 0x60, 0xff],
            Pixel::Changed => [0xff, 0x10, 0x40, 0xff],
        };
        out[i..i + 4].copy_from_slice(&px);
    });
    Image::new(actual.width, actual.height, out)
}

enum Pixel {
    Same,
    Changed,
    Masked,
}

/// Call `f` with the position and the [`Pixel`] kind of every pixel.
fn for_each_pixel(
    reference: &Image,
    actual: &Image,
    opts: &Options,
    mut f: impl FnMut(u32, u32, Pixel),
) {
    let (w, h) = (actual.width, actual.height);
    for y in 0..h {
        for x in 0..w {
            if opts.masked(x, y, w, h) {
                f(x, y, Pixel::Masked);
                continue;
            }
            let i = (y as usize * w as usize + x as usize) * 4;
            let differs = opts.differs(&reference.rgba[i..i + 4], &actual.rgba[i..i + 4]);
            f(x, y, if differs { Pixel::Changed } else { Pixel::Same });
        }
    }
}

fn same_size(a: &Image, b: &Image) -> Result<(), String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "the images differ in size: {}x{} and {}x{}",
            a.width, a.height, b.width, b.height
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: u8) -> Image {
        Image::new(w, h, [v, v, v, 0xff].repeat((w * h) as usize)).unwrap()
    }

    fn set(img: &mut Image, x: u32, y: u32, v: u8) {
        let i = ((y * img.width + x) * 4) as usize;
        img.rgba[i..i + 3].fill(v);
    }

    #[test]
    fn equal_images_have_no_changed_pixels() {
        let r = compare(&flat(4, 3, 9), &flat(4, 3, 9), &Options::default()).unwrap();
        assert_eq!((r.compared, r.changed, r.bbox), (12, 0, None));
    }

    #[test]
    fn changed_pixels_are_counted_and_boxed() {
        let mut b = flat(8, 8, 0);
        set(&mut b, 2, 1, 200);
        set(&mut b, 5, 6, 200);
        let r = compare(&flat(8, 8, 0), &b, &Options::default()).unwrap();
        assert_eq!(r.changed, 2);
        assert_eq!(r.bbox, Some([2, 1, 4, 6]));
    }

    #[test]
    fn the_threshold_absorbs_small_differences() {
        let opts = Options {
            threshold: 3,
            ..Default::default()
        };
        assert_eq!(
            compare(&flat(2, 2, 10), &flat(2, 2, 13), &opts)
                .unwrap()
                .changed,
            0
        );
        assert_eq!(
            compare(&flat(2, 2, 10), &flat(2, 2, 14), &opts)
                .unwrap()
                .changed,
            4
        );
    }

    #[test]
    fn masked_pixels_are_left_out() {
        let mut b = flat(10, 10, 0);
        set(&mut b, 1, 1, 255);
        let opts = Options {
            masks: vec![Region::parse("0,0,0.5,0.5").unwrap()],
            ..Default::default()
        };
        let r = compare(&flat(10, 10, 0), &b, &opts).unwrap();
        assert_eq!((r.compared, r.changed), (75, 0));
        let pic = diff_image(&flat(10, 10, 0), &b, &opts).unwrap();
        assert_eq!(pic.pixel(0, 0), [0, 0, 0x60, 0xff]);
    }

    #[test]
    fn a_region_leaves_out_everything_outside_it() {
        let mut b = flat(10, 10, 0);
        set(&mut b, 1, 1, 255);
        set(&mut b, 8, 8, 255);
        let opts = Options {
            region: Some(Region::parse("0.5,0.5,1,1").unwrap()),
            ..Default::default()
        };
        let r = compare(&flat(10, 10, 0), &b, &opts).unwrap();
        assert_eq!((r.compared, r.changed, r.bbox), (25, 1, Some([8, 8, 1, 1])));
    }

    #[test]
    fn sizes_must_match() {
        assert!(compare(&flat(2, 2, 0), &flat(3, 2, 0), &Options::default()).is_err());
    }

    #[test]
    fn a_region_needs_four_ordered_fractions() {
        assert_eq!(
            Region::parse("0.1 0.2 0.3 0.4").unwrap(),
            Region {
                x0: 0.1,
                y0: 0.2,
                x1: 0.3,
                y1: 0.4
            }
        );
        assert!(Region::parse("0.3,0.2,0.1,0.4").is_err());
        assert!(Region::parse("0,0,1").is_err());
        assert!(Region::parse("0,0,2,1").is_err());
    }
}
