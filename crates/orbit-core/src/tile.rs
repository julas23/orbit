//! Where the windows go, for now: side by side in columns over the
//! area the panel and dock leave free, with gaps and room for the borders.
//! Pure arithmetic, tested without a screen; the i3 tree (split, tabbed,
//! stacked) replaces it in the `orbit-layout` crate (PLANO-CORE §6).

/// A rectangle: x, y, width, height.
pub type Rect = (i32, i32, i32, i32);

/// The rectangles of `count` windows in `area` — their content, inside the
/// border. Gaps go between windows and around them.
pub fn columns(count: usize, area: Rect, gaps: i32, border: i32) -> Vec<Rect> {
    weighted(&vec![1.0; count], area, gaps, border)
}

/// The rectangles of columns as wide as their `weights` say, each its
/// share of the sum (Mod+right-drag moves the line between two).
pub fn weighted(weights: &[f64], area: Rect, gaps: i32, border: i32) -> Vec<Rect> {
    if weights.is_empty() {
        return Vec::new();
    }
    let (x, y, w, h) = area;
    let n = weights.len() as i32;
    let inner_w = (w - gaps * (n + 1)).max(n);
    let height = (h - 2 * gaps - 2 * border).max(1);
    let total: f64 = weights.iter().map(|k| k.max(0.0)).sum::<f64>().max(f64::EPSILON);
    // Each column ends where the sum of the weights up to it falls,
    // rounded, so the widths always add up to the whole.
    let mut ends = Vec::with_capacity(weights.len() + 1);
    let mut sum = 0.0;
    ends.push(0);
    for k in weights {
        sum += k.max(0.0);
        ends.push((inner_w as f64 * sum / total).round() as i32);
    }
    (0..n)
        .map(|i| {
            let (start, end) = (ends[i as usize], ends[i as usize + 1]);
            let left = x + gaps + start + i * gaps;
            (left + border, y + gaps + border, (end - start - 2 * border).max(1), height)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_window_fills_the_area() {
        assert_eq!(columns(1, (0, 0, 1000, 800), 6, 2), vec![(8, 8, 984, 784)]);
    }

    #[test]
    fn columns_share_the_width() {
        let rects = columns(3, (100, 0, 1000, 500), 10, 0);
        assert_eq!(rects.len(), 3);
        // 1000 - 4 gaps = 960, 320 each.
        assert_eq!(rects[0], (110, 10, 320, 480));
        assert_eq!(rects[1], (440, 10, 320, 480));
        assert_eq!(rects[2], (770, 10, 320, 480));
        // The last ends one gap before the edge.
        assert_eq!(rects[2].0 + rects[2].2, 100 + 1000 - 10);
    }

    #[test]
    fn remainders_and_tiny_areas() {
        let rects = columns(3, (0, 0, 101, 50), 0, 0);
        assert_eq!(rects.iter().map(|r| r.2).sum::<i32>(), 101);
        assert!(columns(5, (0, 0, 10, 10), 6, 2).iter().all(|r| r.2 >= 1 && r.3 >= 1));
        assert!(columns(0, (0, 0, 10, 10), 0, 0).is_empty());
    }

    #[test]
    fn weights_share_the_width() {
        // 1000 - 3 gaps = 970: a third and two thirds.
        let rects = weighted(&[1.0, 2.0], (0, 0, 1000, 500), 10, 0);
        assert_eq!(rects[0], (10, 10, 323, 480));
        assert_eq!(rects[1], (343, 10, 647, 480));
        assert_eq!(rects[1].0 + rects[1].2, 1000 - 10);
    }
}
