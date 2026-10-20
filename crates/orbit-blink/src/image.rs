//! A captured image, kept as the compositor gives it: 4 bytes a pixel in
//! B, G, R, X order (little-endian XRGB8888) — also what cairo draws from,
//! so the frozen screen is shown without converting it.

use anyhow::Context;
use gtk::{cairo, gdk_pixbuf, glib};

#[derive(Clone)]
pub struct Image {
    pub width: i32,
    pub height: i32,
    /// `width * 4` bytes a row, top row first.
    pub bgrx: Vec<u8>,
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    /// The part of `self` inside `other`, if any.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        (right > x && bottom > y).then_some(Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && y >= f64::from(self.y)
            && x < f64::from(self.x + self.width)
            && y < f64::from(self.y + self.height)
    }

    /// From two corners, in any order.
    pub fn from_corners(a: (f64, f64), b: (f64, f64)) -> Rect {
        let x = a.0.min(b.0).floor() as i32;
        let y = a.1.min(b.1).floor() as i32;
        Rect {
            x,
            y,
            width: (a.0.max(b.0).ceil() as i32 - x).max(1),
            height: (a.1.max(b.1).ceil() as i32 - y).max(1),
        }
    }
}

impl Image {
    /// The pixels inside `rect`, clamped to the image.
    pub fn crop(&self, rect: Rect) -> Option<Image> {
        let r = rect.intersect(Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        })?;
        let mut bgrx = Vec::with_capacity((r.width * r.height * 4) as usize);
        for row in r.y..r.y + r.height {
            let start = ((row * self.width + r.x) * 4) as usize;
            bgrx.extend_from_slice(&self.bgrx[start..start + (r.width * 4) as usize]);
        }
        Some(Image {
            width: r.width,
            height: r.height,
            bgrx,
        })
    }

    /// As a cairo surface, for drawing.
    pub fn surface(&self) -> anyhow::Result<cairo::ImageSurface> {
        cairo::ImageSurface::create_for_data(
            self.bgrx.clone(),
            cairo::Format::Rgb24,
            self.width,
            self.height,
            self.width * 4,
        )
        .context("cairo surface")
    }

    /// As a pixbuf (RGB), for thumbnails and PNG.
    pub fn pixbuf(&self) -> gdk_pixbuf::Pixbuf {
        let mut rgb = Vec::with_capacity((self.width * self.height * 3) as usize);
        for px in self.bgrx.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from_owned(rgb),
            gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            self.width,
            self.height,
            self.width * 3,
        )
    }

    /// PNG-encoded.
    pub fn png(&self) -> anyhow::Result<Vec<u8>> {
        self.pixbuf()
            .save_to_bufferv("png", &[])
            .context("encoding the PNG")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles() {
        let a = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let b = Rect {
            x: 5,
            y: 8,
            width: 10,
            height: 10,
        };
        assert_eq!(
            a.intersect(b),
            Some(Rect {
                x: 5,
                y: 8,
                width: 5,
                height: 2
            })
        );
        assert_eq!(
            a.intersect(Rect {
                x: 10,
                y: 0,
                width: 3,
                height: 3
            }),
            None
        );
        assert_eq!(
            Rect::from_corners((7.5, 2.0), (1.2, 6.0)),
            Rect {
                x: 1,
                y: 2,
                width: 7,
                height: 4
            }
        );
        assert!(a.contains(9.9, 0.0) && !a.contains(10.0, 0.0));
    }

    #[test]
    fn crop_and_png() {
        // 3×2: pixel value = its index.
        let bgrx: Vec<u8> = (0..6u8).flat_map(|i| [i, i, i, 255]).collect();
        let image = Image {
            width: 3,
            height: 2,
            bgrx,
        };
        let part = image
            .crop(Rect {
                x: 1,
                y: 0,
                width: 5,
                height: 5,
            })
            .unwrap();
        assert_eq!((part.width, part.height), (2, 2));
        let firsts: Vec<u8> = part.bgrx.chunks(4).map(|p| p[0]).collect();
        assert_eq!(firsts, [1, 2, 4, 5]);
        let png = part.png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
