//! The vertical panel's grid: 4 columns of 30 px cells, 10 px apart, as
//! many rows as the screen's height holds — 27 on a 1080 px screen
//! (5 + 27 × 30 + 26 × 10 + 5). Columns are named a–d and rows numbered
//! from 1, so the top-left cell is a1 (maintenance mode labels them).
//!
//! Every module takes a block of whole cells: as many columns as its size
//! says, and the rows its size says or, when its content is taller, the rows
//! that content needs. Modules flow left to right, top to bottom, each in the
//! first free block after the one before; the start part from the top row
//! down, the end part so that its last row is the bottom one, the centre part
//! in the middle.

use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*, subclass::prelude::*};

/// Cell side.
pub const CELL: i32 = 30;
/// Between two cells, across and down.
pub const GAP: i32 = 10;
/// From one cell to the next.
pub const PITCH: i32 = CELL + GAP;
/// Above the first row and below the last.
pub const PAD: i32 = 5;
pub const COLUMNS: i32 = 4;

/// How many whole rows fit in `height`.
pub fn rows_in(height: i32) -> i32 {
    ((height - 2 * PAD + GAP) / PITCH).max(0)
}

/// The size of a block of `cells` cells: `n × 30 + (n − 1) × 10`.
pub fn span(cells: i32) -> i32 {
    (cells * PITCH - GAP).max(0)
}

/// A cell's name: column letter and row number, `a1` … `d27`.
pub fn cell_name(col: i32, row: i32) -> String {
    format!("{}{}", char::from(b'a' + col as u8), row + 1)
}

/// The shape a module is built in: one of its three sizes on the vertical
/// panel's grid, or the single line of a horizontal panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Line,
    Mini,
    Normal,
    Full,
}

impl Form {
    pub fn vertical(self) -> bool {
        self != Form::Line
    }

    /// The CSS class of the module's root: `mini`, `normal`, `full`.
    pub fn class(self) -> &'static str {
        match self {
            Form::Line => "line",
            Form::Mini => "mini",
            Form::Normal => "normal",
            Form::Full => "full",
        }
    }
}

impl From<orbit_common::config::Size> for Form {
    fn from(size: orbit_common::config::Size) -> Self {
        use orbit_common::config::Size;
        match size {
            Size::Mini => Form::Mini,
            Size::Normal => Form::Normal,
            Size::Full => Form::Full,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Start,
    Center,
    End,
}

struct Item {
    widget: gtk::Widget,
    name: String,
    part: Part,
    cols: i32,
    rows: i32,
    /// The rows the content asked for last, to log a module outgrowing its
    /// size only when that changes.
    measured: Cell<i32>,
    /// The cells it was given last (`a19:d20`), logged when they change.
    cells: RefCell<String>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Board {
        pub(super) items: RefCell<Vec<Item>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Board {
        const NAME: &'static str = "OrbitBoard";
        type Type = super::Board;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Board {
        fn dispose(&self) {
            for item in self.items.take() {
                item.widget.unparent();
            }
        }
    }

    impl WidgetImpl for Board {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let width = span(COLUMNS);
            if orientation == gtk::Orientation::Horizontal {
                return (width, width, -1, -1);
            }
            let items = self.items.borrow();
            let rows: i32 = [Part::Start, Part::Center, Part::End]
                .into_iter()
                .map(|part| place(&items, part).1)
                .sum();
            let height = 2 * PAD + span(rows);
            (height, height, -1, -1)
        }

        fn size_allocate(&self, _width: i32, height: i32, _baseline: i32) {
            let items = self.items.borrow();
            let total = rows_in(height);
            for part in [Part::Start, Part::Center, Part::End] {
                let (blocks, used) = place(&items, part);
                let first = match part {
                    Part::Start => 0,
                    Part::Center => (total - used) / 2,
                    Part::End => total - used,
                }
                .max(0);
                for (i, (col, row, rows)) in blocks {
                    let item = &items[i];
                    let rect = gtk::Allocation::new(
                        col * PITCH,
                        PAD + (first + row) * PITCH,
                        span(item.cols),
                        span(rows),
                    );
                    item.widget.size_allocate(&rect, -1);
                    let row = first + row;
                    let cells = format!(
                        "{}:{}",
                        cell_name(col, row),
                        cell_name(col + item.cols - 1, row + rows - 1)
                    );
                    if *item.cells.borrow() != cells {
                        log::info!("panel: {} at {cells}", item.name);
                        item.cells.replace(cells);
                    }
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct Board(ObjectSubclass<imp::Board>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Board {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Board {
    /// Adds module `name` to `part`, after the ones already there, taking
    /// `cols` columns and at least `rows` rows.
    pub fn add(&self, widget: &gtk::Widget, name: &str, part: Part, cols: i32, rows: i32) {
        widget.set_parent(self);
        // A module never spills over the block it was given.
        widget.set_overflow(gtk::Overflow::Hidden);
        self.imp().items.borrow_mut().push(Item {
            widget: widget.clone(),
            name: name.into(),
            part,
            cols: cols.clamp(1, COLUMNS),
            rows: rows.max(1),
            measured: Cell::new(0),
            cells: RefCell::default(),
        });
    }
}

/// A module's place: its index in the items, and (column, row, rows taken).
type Block = (usize, (i32, i32, i32));

/// Where `part`'s modules go, relative to the part's first row, and the rows
/// the part takes.
fn place(items: &[Item], part: Part) -> (Vec<Block>, i32) {
    // Row by row, which of the four cells are taken.
    let mut taken: Vec<[bool; COLUMNS as usize]> = Vec::new();
    let mut blocks = Vec::new();
    let (mut row, mut col) = (0, 0);
    for (i, item) in items.iter().enumerate() {
        if item.part != part || !item.widget.should_layout() {
            continue;
        }
        let rows = item.rows.max(content_rows(item));
        let free = |taken: &Vec<[bool; 4]>, r: i32, c: i32| {
            (r..r + rows).all(|r| {
                (c..c + item.cols).all(|c| taken.get(r as usize).is_none_or(|line| !line[c as usize]))
            })
        };
        // The first free block from where the last module ended.
        loop {
            if col + item.cols <= COLUMNS && free(&taken, row, col) {
                break;
            }
            col += 1;
            if col + item.cols > COLUMNS {
                col = 0;
                row += 1;
            }
        }
        for r in row..row + rows {
            while taken.len() <= r as usize {
                taken.push([false; 4]);
            }
            for c in col..col + item.cols {
                taken[r as usize][c as usize] = true;
            }
        }
        blocks.push((i, (col, row, rows)));
        col += item.cols;
    }
    let used = taken.len() as i32;
    (blocks, used)
}

/// The rows `item`'s content needs at its width.
fn content_rows(item: &Item) -> i32 {
    let (min, natural, _, _) = item.widget.measure(gtk::Orientation::Vertical, span(item.cols));
    let rows = (natural.max(min) + GAP + PITCH - 1) / PITCH;
    if rows > item.rows && item.measured.replace(rows) != rows {
        log::warn!(
            "panel: {} is {} px tall, more than its {} row(s) of {} px: it takes {rows}",
            item.name,
            natural.max(min),
            item.rows,
            span(item.rows),
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_spans() {
        assert_eq!(rows_in(1080), 27);
        assert_eq!(2 * PAD + span(27), 1080);
        assert_eq!(span(COLUMNS), 150);
        assert_eq!(span(1), 30);
        assert_eq!(span(2), 70);
        assert_eq!(span(4), 150);
        assert_eq!(cell_name(0, 0), "a1");
        assert_eq!(cell_name(3, 26), "d27");
    }
}
