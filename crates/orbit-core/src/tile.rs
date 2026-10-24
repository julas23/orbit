//! Where the windows go, over the area the panel and dock leave free, with
//! gaps and room for the borders: each workspace has a layout (`Layout`,
//! the `layout` command): side by side in columns, master and stack (dwm),
//! one at a time (monocle), or each new window splitting the last one's
//! place (bspwm's spiral). Pure arithmetic, tested without a screen; the i3
//! tree (split, tabbed, stacked) comes with the `orbit-layout` crate
//! (PLANO-CORE §6).

/// A rectangle: x, y, width, height.
pub type Rect = (i32, i32, i32, i32);

/// How a workspace's tiled windows share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Side by side, as wide as their weights.
    #[default]
    Columns,
    /// The first window on the left (`MASTER_RATIO` of the width), the
    /// others stacked on the right.
    Master,
    /// Each one over the whole area, the focused one on top.
    Monocle,
    /// Each window splits the place of the one before along its longer
    /// side.
    Bsp,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Columns, Layout::Master, Layout::Monocle, Layout::Bsp];

    pub fn name(self) -> &'static str {
        match self {
            Layout::Columns => "columns",
            Layout::Master => "master",
            Layout::Monocle => "monocle",
            Layout::Bsp => "bsp",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }

    /// The one after this, round the list (`layout next`).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The rectangles of tiled windows with these `weights` (columns only
    /// use them) in `area`.
    pub fn rects(self, weights: &[f64], area: Rect, gaps: i32, border: i32) -> Vec<Rect> {
        match self {
            Layout::Columns => weighted(weights, area, gaps, border),
            Layout::Master => master(weights.len(), area, gaps, border),
            Layout::Monocle => vec![inset(area, gaps, border); weights.len()],
            Layout::Bsp => bsp(weights.len(), area, gaps, border),
        }
    }
}

/// The master's share of the width, in `Layout::Master`.
const MASTER_RATIO: f64 = 0.55;

/// `area` less the gaps around it and the border: one window's content.
fn inset((x, y, w, h): Rect, gaps: i32, border: i32) -> Rect {
    (x + gaps + border, y + gaps + border, (w - 2 * gaps - 2 * border).max(1), (h - 2 * gaps - 2 * border).max(1))
}

/// `count` rectangles one under the other in `area` (gaps between them
/// and around), content inside the border.
fn rows(count: usize, (x, y, w, h): Rect, gaps: i32, border: i32) -> Vec<Rect> {
    // Columns of the area turned on its side.
    columns(count, (y, x, h, w), gaps, border).into_iter().map(|(ry, rx, rh, rw)| (rx, ry, rw, rh)).collect()
}

/// The first window on the left, the others stacked on the right.
pub fn master(count: usize, area: Rect, gaps: i32, border: i32) -> Vec<Rect> {
    if count <= 1 {
        return columns(count, area, gaps, border);
    }
    let (x, y, w, h) = area;
    // The line between the two parts, half a gap each side of it.
    let split = (f64::from(w) * MASTER_RATIO).round() as i32;
    let left = (x, y, split + gaps / 2, h);
    let right = (x + split - gaps / 2, y, w - split + gaps / 2, h);
    let mut rects = columns(1, left, gaps, border);
    rects.extend(rows(count - 1, right, gaps, border));
    rects
}

/// Each window splits the place of the one before along its longer side,
/// the newer half going right or down: bspwm's spiral.
pub fn bsp(count: usize, area: Rect, gaps: i32, border: i32) -> Vec<Rect> {
    let mut places = Vec::with_capacity(count);
    let mut rest = area;
    for i in 0..count {
        if i + 1 == count {
            places.push(rest);
            break;
        }
        let (x, y, w, h) = rest;
        if w >= h {
            let half = w / 2;
            places.push((x, y, half + gaps / 2, h));
            rest = (x + half - gaps / 2, y, w - half + gaps / 2, h);
        } else {
            let half = h / 2;
            places.push((x, y, w, half + gaps / 2));
            rest = (x, y + half - gaps / 2, w, h - half + gaps / 2);
        }
    }
    places.into_iter().map(|p| inset(p, gaps, border)).collect()
}

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

    /// No two rectangles overlap, and all are inside `area`.
    fn apart(rects: &[Rect], area: Rect) -> bool {
        let inside = |&(x, y, w, h): &Rect| x >= area.0 && y >= area.1 && x + w <= area.0 + area.2 && y + h <= area.1 + area.3;
        let overlap = |a: &Rect, b: &Rect| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
        rects.iter().all(inside) && rects.iter().enumerate().all(|(i, a)| rects[i + 1..].iter().all(|b| !overlap(a, b)))
    }

    #[test]
    fn master_and_stack() {
        let area = (0, 0, 1000, 600);
        let rects = master(3, area, 10, 0);
        // 55% for the master; one gap between it and the stack.
        assert_eq!(rects[0], (10, 10, 535, 580));
        assert_eq!(rects[1].0, 555);
        assert_eq!(rects[1].0 + rects[1].2, 990);
        assert_eq!(rects[1].2, rects[2].2);
        assert!(rects[1].1 < rects[2].1);
        assert!(apart(&rects, area));
        assert_eq!(master(1, area, 10, 0), columns(1, area, 10, 0));
    }

    #[test]
    fn bsp_spirals_in() {
        let area = (0, 0, 1600, 900);
        let rects = bsp(4, area, 10, 2);
        assert_eq!(rects.len(), 4);
        assert!(apart(&rects, area));
        // The first takes the left half; the second the top of the right.
        assert!(rects[0].2 < 800 && rects[0].3 > 800);
        assert!(rects[1].0 > 800 && rects[1].3 < 450);
        assert_eq!(bsp(1, area, 10, 2), columns(1, area, 10, 2));
    }

    #[test]
    fn layouts_by_name_and_in_turn() {
        assert_eq!(Layout::parse("monocle"), Some(Layout::Monocle));
        assert_eq!(Layout::parse("tabbed"), None);
        assert_eq!(Layout::Bsp.next(), Layout::Columns);
        let monocle = Layout::Monocle.rects(&[1.0, 1.0], (0, 0, 100, 100), 5, 1);
        assert_eq!(monocle, vec![(6, 6, 88, 88); 2]);
    }

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
