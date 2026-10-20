//! Shrinking a decoded frame to a thumbnail, off the GTK thread: to 8-bit
//! RGBA (straight alpha, as gdk-pixbuf wants it), averaged down in boxes.

use gtk::gdk;

use crate::decode::RawFrame;

/// `frame` fitted in `size` × `size` (never enlarged), as RGBA rows with no
/// padding; `None` for the float formats (HDR), which get no thumbnail.
pub fn thumbnail(frame: &RawFrame, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let layout = Layout::of(frame.format)?;
    let scale = (f64::from(size) / f64::from(frame.width.max(frame.height))).min(1.0);
    let w = ((f64::from(frame.width) * scale).round() as u32).max(1);
    let h = ((f64::from(frame.height) * scale).round() as u32).max(1);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for ty in 0..h {
        let y0 = ty * frame.height / h;
        let y1 = ((ty + 1) * frame.height / h).max(y0 + 1);
        for tx in 0..w {
            let x0 = tx * frame.width / w;
            let x1 = ((tx + 1) * frame.width / w).max(x0 + 1);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                let row = &frame.data[y as usize * frame.stride..];
                for x in x0..x1 {
                    let px = layout.read(&row[x as usize * layout.bytes..]);
                    // Weighted by alpha, so transparent pixels lend no colour.
                    let a = u32::from(px[3]);
                    let (r, g, b) = if layout.premultiplied {
                        (u32::from(px[0]) * 255, u32::from(px[1]) * 255, u32::from(px[2]) * 255)
                    } else {
                        (u32::from(px[0]) * a, u32::from(px[1]) * a, u32::from(px[2]) * a)
                    };
                    sum[0] += r;
                    sum[1] += g;
                    sum[2] += b;
                    sum[3] += a;
                }
            }
            let n = (x1 - x0) * (y1 - y0);
            match std::num::NonZeroU32::new(sum[3]) {
                None => out.extend_from_slice(&[0, 0, 0, 0]),
                Some(a) => {
                    out.push((sum[0] / a).min(255) as u8);
                    out.push((sum[1] / a).min(255) as u8);
                    out.push((sum[2] / a).min(255) as u8);
                    out.push((sum[3] / n) as u8);
                }
            }
        }
    }
    Some((w, h, out))
}

/// Where each channel sits in a pixel, for the integer formats.
struct Layout {
    bytes: usize,
    /// Byte offset of red, green, blue and alpha (`None`: opaque); for the
    /// 16-bit formats, of each channel's high byte.
    channels: [usize; 3],
    alpha: Option<usize>,
    grey: bool,
    premultiplied: bool,
}

impl Layout {
    fn of(format: gdk::MemoryFormat) -> Option<Self> {
        use gdk::MemoryFormat as F;
        // 16-bit channels are native-endian: the high byte is the second.
        let hi = usize::from(cfg!(target_endian = "little"));
        let rgb = |bytes, r, g, b, alpha, premultiplied| Layout {
            bytes,
            channels: [r, g, b],
            alpha,
            grey: false,
            premultiplied,
        };
        let grey = |bytes, alpha, premultiplied| Layout {
            bytes,
            channels: [0, 0, 0],
            alpha,
            grey: true,
            premultiplied,
        };
        Some(match format {
            F::B8g8r8a8Premultiplied => rgb(4, 2, 1, 0, Some(3), true),
            F::A8r8g8b8Premultiplied => rgb(4, 1, 2, 3, Some(0), true),
            F::R8g8b8a8Premultiplied => rgb(4, 0, 1, 2, Some(3), true),
            F::B8g8r8a8 => rgb(4, 2, 1, 0, Some(3), false),
            F::A8r8g8b8 => rgb(4, 1, 2, 3, Some(0), false),
            F::R8g8b8a8 => rgb(4, 0, 1, 2, Some(3), false),
            F::A8b8g8r8 => rgb(4, 3, 2, 1, Some(0), false),
            F::R8g8b8 => rgb(3, 0, 1, 2, None, false),
            F::B8g8r8 => rgb(3, 2, 1, 0, None, false),
            F::R16g16b16 => rgb(6, hi, 2 + hi, 4 + hi, None, false),
            F::R16g16b16a16Premultiplied => rgb(8, hi, 2 + hi, 4 + hi, Some(6 + hi), true),
            F::R16g16b16a16 => rgb(8, hi, 2 + hi, 4 + hi, Some(6 + hi), false),
            F::G8a8Premultiplied => grey(2, Some(1), true),
            F::G8a8 => grey(2, Some(1), false),
            F::G8 => grey(1, None, false),
            F::G16a16Premultiplied => Layout {
                channels: [hi; 3],
                ..grey(4, Some(2 + hi), true)
            },
            F::G16a16 => Layout {
                channels: [hi; 3],
                ..grey(4, Some(2 + hi), false)
            },
            F::G16 => Layout {
                channels: [hi; 3],
                ..grey(2, None, false)
            },
            _ => return None,
        })
    }

    /// One pixel as `[r, g, b, a]`.
    fn read(&self, px: &[u8]) -> [u8; 4] {
        let a = self.alpha.map_or(255, |i| px[i]);
        if self.grey {
            let v = px[self.channels[0]];
            return [v, v, v, a];
        }
        [
            px[self.channels[0]],
            px[self.channels[1]],
            px[self.channels[2]],
            a,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(format: gdk::MemoryFormat, w: u32, h: u32, data: Vec<u8>) -> RawFrame {
        let stride = data.len() / h as usize;
        RawFrame {
            width: w,
            height: h,
            stride,
            format,
            data,
            delay: None,
        }
    }

    #[test]
    fn averages_boxes_into_rgba() {
        // 2 × 1 RGB: red and blue, down to one pixel.
        let f = frame(gdk::MemoryFormat::R8g8b8, 2, 1, vec![255, 0, 0, 0, 0, 255]);
        let (w, h, px) = thumbnail(&f, 1).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![127, 0, 127, 255]);
    }

    #[test]
    fn never_enlarges_and_reorders_bgra() {
        let f = frame(gdk::MemoryFormat::B8g8r8a8, 1, 1, vec![10, 20, 30, 255]);
        let (w, h, px) = thumbnail(&f, 128).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![30, 20, 10, 255]);
    }

    #[test]
    fn transparent_pixels_lend_no_colour() {
        // Opaque white next to fully transparent black.
        let f = frame(
            gdk::MemoryFormat::R8g8b8a8,
            2,
            1,
            vec![255, 255, 255, 255, 0, 0, 0, 0],
        );
        let (_, _, px) = thumbnail(&f, 1).unwrap();
        assert_eq!(px, vec![255, 255, 255, 127]);
    }

    #[test]
    fn keeps_the_aspect_ratio() {
        let f = frame(gdk::MemoryFormat::G8, 400, 100, vec![128; 400 * 100]);
        let (w, h, px) = thumbnail(&f, 128).unwrap();
        assert_eq!((w, h), (128, 32));
        assert_eq!(&px[..4], &[128, 128, 128, 255]);
    }
}
