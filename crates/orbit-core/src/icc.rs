//! A monitor's colour profile (core.yaml `outputs: NAME: { profile: FILE }`):
//! the calibration curves of an ICC profile (its `vcgt` tag, what a
//! calibration with DisplayCAL or colord leaves there) loaded into the
//! monitor's gamma ramp, as GNOME and KDE load them. The rest of the
//! profile (how programs' colours map to the panel's) is the programs'
//! own business: the core transforms nothing it draws.
//!
//! A night light (wlr-gamma-control, `gamma`) takes the ramp while it
//! runs; when it goes, the profile's comes back.

use std::path::Path;

/// The calibration curves, red, green and blue, each from 0 to 1 at even
/// steps of the input.
#[derive(Debug, Clone, PartialEq)]
pub struct Curves(pub [Vec<f64>; 3]);

impl Curves {
    /// The curves at `size` entries each, as the gamma ramp takes them.
    pub fn ramp(&self, size: usize) -> [Vec<u16>; 3] {
        self.0.clone().map(|c| resample(&c, size))
    }
}

/// `curve` read at `size` even steps, between its points linearly.
fn resample(curve: &[f64], size: usize) -> Vec<u16> {
    let last = curve.len().saturating_sub(1);
    (0..size)
        .map(|i| {
            let x = if size > 1 { i as f64 / (size - 1) as f64 } else { 0.0 };
            let v = if last == 0 {
                curve.first().copied().unwrap_or(x)
            } else {
                let at = x * last as f64;
                let lo = at.floor() as usize;
                let hi = (lo + 1).min(last);
                curve[lo] + (curve[hi] - curve[lo]) * (at - lo as f64)
            };
            (v.clamp(0.0, 1.0) * 65535.0).round() as u16
        })
        .collect()
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// ICC's s15Fixed16Number.
fn fixed_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from(u32_at(b, at)? as i32) / 65536.0)
}

/// The profile's calibration curves; none when it has no `vcgt` tag (or
/// is no ICC profile).
pub fn curves(bytes: &[u8]) -> Option<Curves> {
    if bytes.get(36..40)? != b"acsp" {
        return None;
    }
    let count = u32_at(bytes, 128)? as usize;
    let tag = (0..count.min(1000)).find_map(|i| {
        let at = 132 + i * 12;
        (bytes.get(at..at + 4)? == b"vcgt").then(|| Some((u32_at(bytes, at + 4)? as usize, u32_at(bytes, at + 8)? as usize)))?
    })?;
    let (offset, size) = tag;
    let t = bytes.get(offset..offset.checked_add(size)?)?;
    if t.get(0..4)? != b"vcgt" {
        return None;
    }
    match u32_at(t, 8)? {
        // A table: channels, entries, bytes an entry, then the values.
        0 => {
            let (channels, entries, width) = (u16_at(t, 12)? as usize, u16_at(t, 14)? as usize, u16_at(t, 16)? as usize);
            if entries < 2 || !matches!(width, 1 | 2) || !matches!(channels, 1 | 3) {
                return None;
            }
            let read = |c: usize| -> Option<Vec<f64>> {
                (0..entries)
                    .map(|i| {
                        let at = 18 + (c * entries + i) * width;
                        Some(if width == 1 { f64::from(*t.get(at)?) / 255.0 } else { f64::from(u16_at(t, at)?) / 65535.0 })
                    })
                    .collect()
            };
            let red = read(0)?;
            let (green, blue) = if channels == 3 { (read(1)?, read(2)?) } else { (red.clone(), red.clone()) };
            Some(Curves([red, green, blue]))
        }
        // A formula a channel: gamma, minimum, maximum.
        1 => {
            let channel = |c: usize| -> Option<Vec<f64>> {
                let at = 12 + c * 12;
                let (gamma, min, max) = (fixed_at(t, at)?, fixed_at(t, at + 4)?, fixed_at(t, at + 8)?);
                if gamma.is_nan() || gamma <= 0.0 {
                    return None;
                }
                Some((0..256).map(|i| min + (max - min) * (i as f64 / 255.0).powf(gamma)).collect())
            };
            Some(Curves([channel(0)?, channel(1)?, channel(2)?]))
        }
        _ => None,
    }
}

/// The curves of the profile in `path`, said when it cannot be read or
/// has none.
pub fn load(path: &Path) -> Option<Curves> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let found = curves(&bytes);
            if found.is_none() {
                tracing::info!("{}: no calibration curves (vcgt) to load", path.display());
            }
            found
        }
        Err(err) => {
            tracing::warn!("{}: {err}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ICC profile holding only `vcgt`.
    fn profile(vcgt: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[36..40].copy_from_slice(b"acsp");
        b.extend(1u32.to_be_bytes());
        b.extend(b"vcgt");
        b.extend(144u32.to_be_bytes());
        b.extend((vcgt.len() as u32).to_be_bytes());
        b.extend(vcgt);
        b
    }

    #[test]
    fn table_of_three_channels() {
        let mut t = b"vcgt".to_vec();
        t.extend([0; 4]);
        t.extend(0u32.to_be_bytes());
        t.extend(3u16.to_be_bytes());
        t.extend(2u16.to_be_bytes());
        t.extend(2u16.to_be_bytes());
        for v in [0u16, 65535, 0, 32768, 1000, 60000] {
            t.extend(v.to_be_bytes());
        }
        let c = curves(&profile(&t)).unwrap();
        let [r, g, b] = c.ramp(3);
        assert_eq!(r, [0, 32768, 65535]);
        assert_eq!(g, [0, 16384, 32768]);
        assert_eq!((b[0], b[2]), (1000, 60000));
    }

    #[test]
    fn formula() {
        let mut t = b"vcgt".to_vec();
        t.extend([0; 4]);
        t.extend(1u32.to_be_bytes());
        for _ in 0..3 {
            t.extend(65536u32.to_be_bytes()); // gamma 1
            t.extend(0u32.to_be_bytes()); // from 0
            t.extend(32768u32.to_be_bytes()); // to 0.5
        }
        let [r, ..] = curves(&profile(&t)).unwrap().ramp(2);
        assert_eq!(r, [0, 32768]);
    }

    #[test]
    fn none_without_vcgt() {
        assert_eq!(curves(b"not a profile"), None);
        let mut b = profile(b"vcgt");
        b[132..136].copy_from_slice(b"desc");
        assert_eq!(curves(&b), None);
    }
}
