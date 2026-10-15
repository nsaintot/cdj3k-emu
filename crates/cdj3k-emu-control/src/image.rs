//! RGBA images, and their PNG and base64 forms.

use std::path::Path;

use base64::Engine;

/// An 8-bit RGBA image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Image, String> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(format!(
                "{} bytes do not make a {width}x{height} RGBA image",
                rgba.len()
            ));
        }
        Ok(Image {
            width,
            height,
            rgba,
        })
    }

    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&self.rgba).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Decode a PNG of any 8- or 16-bit colour type into RGBA.
    pub fn from_png(bytes: &[u8]) -> Result<Image, String> {
        let mut dec = png::Decoder::new(bytes);
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        buf.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[r, g, b]| [r, g, b, 0xff])
                .collect(),
            png::ColorType::GrayscaleAlpha => buf
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|&[g, a]| [g, g, g, a])
                .collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 0xff]).collect(),
            png::ColorType::Indexed => return Err("an indexed PNG did not expand".into()),
        };
        Image::new(info.width, info.height, rgba)
    }

    pub fn read_png(path: &Path) -> Result<Image, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Image::from_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn write_png(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_png()?).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_base64_png(&self) -> Result<String, String> {
        Ok(base64::engine::general_purpose::STANDARD.encode(self.to_png()?))
    }

    pub fn from_base64_png(s: &str) -> Result<Image, String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|e| e.to_string())?;
        Image::from_png(&bytes)
    }

    /// The RGBA bytes of the pixel at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_survives_png_and_base64() {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 11) as u8).collect();
        let img = Image::new(3, 2, rgba).unwrap();
        let back = Image::from_base64_png(&img.to_base64_png().unwrap()).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn the_byte_count_must_match_the_size() {
        assert!(Image::new(2, 2, vec![0; 15]).is_err());
    }
}
