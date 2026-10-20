//! The standard and scientific profiles: a display and a keypad, also
//! driven from the keyboard. Both type an expression and evaluate it with
//! the usual precedence (expr.rs); the scientific keypad adds functions,
//! powers, parentheses and the choice of degrees or radians.
//!
//! The display has three lines: the expression of the last result (after
//! `=`), what is being typed in large type, and below it the value it would
//! have — or why `=` gave none.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::tr;

use crate::{
    entry::{Entry, Key},
    expr::{Angle, Error},
    format::{self, Separators},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Standard,
    Scientific,
}

struct Calc {
    entry: RefCell<Entry>,
    sep: Separators,
    angle: Cell<Angle>,
    history: gtk::Label,
    main: gtk::Label,
    preview: gtk::Label,
}

/// A key of the keypad: (label, key, CSS class). The decimal point's label
/// is left empty: it is the language's separator. `ANGLE` is the degrees /
/// radians switch.
type Spec = (&'static str, Key, &'static str);

const ANGLE: Spec = ("", Key::Clear, "angle");

const STANDARD: &[&[Spec]] = &[
    &[
        ("C", Key::Clear, "clear"),
        ("⌫", Key::Back, "fn"),
        ("%", Key::Percent, "fn"),
        ("÷", Key::Op('/'), "op"),
    ],
    &[
        ("7", Key::Digit('7'), "digit"),
        ("8", Key::Digit('8'), "digit"),
        ("9", Key::Digit('9'), "digit"),
        ("×", Key::Op('*'), "op"),
    ],
    &[
        ("4", Key::Digit('4'), "digit"),
        ("5", Key::Digit('5'), "digit"),
        ("6", Key::Digit('6'), "digit"),
        ("−", Key::Op('-'), "op"),
    ],
    &[
        ("1", Key::Digit('1'), "digit"),
        ("2", Key::Digit('2'), "digit"),
        ("3", Key::Digit('3'), "digit"),
        ("+", Key::Op('+'), "op"),
    ],
    &[
        ("±", Key::Sign, "digit"),
        ("0", Key::Digit('0'), "digit"),
        ("", Key::Point, "digit"),
        ("=", Key::Equals, "equals"),
    ],
];

/// Four columns of functions to the left of the standard keys.
const SCIENTIFIC: &[&[Spec]] = &[
    &[
        ANGLE,
        ("(", Key::Open, "fn"),
        (")", Key::Close, "fn"),
        ("n!", Key::Factorial, "fn"),
    ],
    &[
        ("sin", Key::Func("sin"), "fn"),
        ("cos", Key::Func("cos"), "fn"),
        ("tan", Key::Func("tan"), "fn"),
        ("xʸ", Key::Op('^'), "fn"),
    ],
    &[
        ("sin⁻¹", Key::Func("asin"), "fn"),
        ("cos⁻¹", Key::Func("acos"), "fn"),
        ("tan⁻¹", Key::Func("atan"), "fn"),
        ("√x", Key::Func("√"), "fn"),
    ],
    &[
        ("ln", Key::Func("ln"), "fn"),
        ("log", Key::Func("log"), "fn"),
        ("eˣ", Key::Text("e^"), "fn"),
        ("x²", Key::Postfix("^2"), "fn"),
    ],
    &[
        ("π", Key::Text("π"), "fn"),
        ("e", Key::Text("e"), "fn"),
        ("1/x", Key::Postfix("^-1"), "fn"),
        ("|x|", Key::Func("abs"), "fn"),
    ],
];

pub fn new(window: &gtk::ApplicationWindow, layout: Layout) -> gtk::Widget {
    let label = |class: &str| {
        gtk::Label::builder()
            .xalign(1.0)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            // Long text is cut at the start, never widens the window: the
            // keypad sets the width.
            .max_width_chars(1)
            .css_classes([class])
            .build()
    };
    let calc = Rc::new(Calc {
        entry: RefCell::new(Entry::default()),
        sep: Separators::from_catalog(),
        angle: Cell::new(Angle::Degrees),
        history: label("history"),
        main: label("main"),
        preview: label("preview"),
    });

    let display = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["display"])
        .build();
    display.append(&calc.history);
    display.append(&calc.main);
    display.append(&calc.preview);

    let keypad = gtk::Grid::builder()
        .row_spacing(5)
        .column_spacing(5)
        .row_homogeneous(true)
        .column_homogeneous(true)
        .css_classes(["keypad"])
        .build();
    let columns: Vec<(i32, &[&[Spec]])> = match layout {
        Layout::Standard => vec![(0, STANDARD)],
        Layout::Scientific => vec![(0, SCIENTIFIC), (4, STANDARD)],
    };
    for (first, rows) in columns {
        for (row, keys) in rows.iter().enumerate() {
            for (col, &spec) in keys.iter().enumerate() {
                let button = if spec == ANGLE {
                    angle_button(&calc)
                } else {
                    key_button(&calc, spec)
                };
                keypad.attach(&button, first + col as i32, row as i32, 1, 1);
            }
        }
    }

    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes([match layout {
            Layout::Standard => "standard",
            Layout::Scientific => "scientific",
        }])
        .build();
    page.append(&display);
    page.append(&keypad);

    let keys = gtk::EventControllerKey::new();
    // Before any widget, so Enter and Escape always reach the calculator.
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let calc = calc.clone();
        let page = page.downgrade();
        let window = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            // Only while this profile is the one shown.
            if !page.upgrade().is_some_and(|p| p.is_mapped()) {
                return glib::Propagation::Proceed;
            }
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match (ctrl, key.to_lower()) {
                (true, gdk::Key::c) => {
                    calc.copy(&window.upgrade());
                    return glib::Propagation::Stop;
                }
                (true, gdk::Key::v) => {
                    calc.paste(&window.upgrade());
                    return glib::Propagation::Stop;
                }
                (true, _) => return glib::Propagation::Proceed,
                _ => {}
            }
            match key_for(key) {
                Some(k) => {
                    calc.press(k);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
    }
    window.add_controller(keys);

    calc.refresh();
    page.upcast()
}

/// The calculator key for a keyboard key. Both `.` and `,` are the decimal
/// point, whatever the language: keyboards differ in which one is at hand.
fn key_for(key: gdk::Key) -> Option<Key> {
    match key {
        gdk::Key::Return | gdk::Key::KP_Enter => return Some(Key::Equals),
        gdk::Key::BackSpace => return Some(Key::Back),
        gdk::Key::Escape | gdk::Key::Delete | gdk::Key::KP_Delete => return Some(Key::Clear),
        _ => {}
    }
    let c = key.to_unicode()?;
    Some(match c {
        '0'..='9' => Key::Digit(c),
        '.' | ',' => Key::Point,
        '+' | '-' | '*' | '/' | '^' => Key::Op(c),
        '×' => Key::Op('*'),
        '÷' => Key::Op('/'),
        '(' => Key::Open,
        ')' => Key::Close,
        '%' => Key::Percent,
        '!' => Key::Factorial,
        '=' => Key::Equals,
        _ => return None,
    })
}

fn error_text(err: Error) -> String {
    tr(match err {
        Error::Syntax => "calc-error-syntax",
        Error::DivisionByZero => "calc-error-division-by-zero",
        Error::Domain => "calc-error-domain",
        Error::Overflow => "calc-error-overflow",
    })
}

fn key_button(calc: &Rc<Calc>, (text, key, class): Spec) -> gtk::Button {
    let text = if key == Key::Point {
        calc.sep.decimal.to_string()
    } else {
        text.to_string()
    };
    let button = gtk::Button::builder()
        .label(text)
        .css_classes(["key", class])
        // Keys never take the focus: Enter must mean `=`, not "press the
        // last key clicked again".
        .focusable(false)
        .focus_on_click(false)
        .build();
    let calc = calc.clone();
    button.connect_clicked(move |_| calc.press(key));
    button
}

/// DEG / RAD: shows the current unit of angles; a click switches it.
fn angle_button(calc: &Rc<Calc>) -> gtk::Button {
    let button = gtk::Button::builder()
        .label("DEG")
        .css_classes(["key", "fn", "angle"])
        .tooltip_text(tr("calc-angle"))
        .focusable(false)
        .focus_on_click(false)
        .build();
    let calc = calc.clone();
    button.connect_clicked(move |button| {
        let angle = match calc.angle.get() {
            Angle::Degrees => Angle::Radians,
            Angle::Radians => Angle::Degrees,
        };
        calc.angle.set(angle);
        button.set_label(match angle {
            Angle::Degrees => "DEG",
            Angle::Radians => "RAD",
        });
        calc.refresh();
    });
    button
}

impl Calc {
    fn press(&self, key: Key) {
        self.entry.borrow_mut().press(key, self.angle.get());
        self.refresh();
    }

    fn refresh(&self) {
        let entry = self.entry.borrow();
        let set_preview = |text: &str, class: Option<&str>| {
            self.preview.set_label(text);
            self.preview.set_css_classes(&["preview"]);
            if let Some(class) = class {
                self.preview.add_css_class(class);
            }
        };
        if let Some(done) = entry.done() {
            self.history.set_label(&format!(
                "{} =",
                format::expression(&done.expression, self.sep)
            ));
            self.main.set_label(&format::number(done.value, self.sep));
            set_preview("", None);
            return;
        }
        self.history.set_label("");
        let text = entry.text();
        self.main.set_label(&if text.is_empty() {
            "0".to_string()
        } else {
            format::expression(text, self.sep)
        });
        if let Some(err) = entry.error() {
            set_preview(&error_text(err), Some("error"));
            return;
        }
        // The value, unless the text already is just that number.
        let value = entry
            .preview(self.angle.get())
            .map(|v| format::number(v, self.sep))
            .filter(|v| *v != self.main.label());
        set_preview(&value.map(|v| format!("= {v}")).unwrap_or_default(), None);
    }

    /// The result, or the value being previewed, in the user's notation
    /// without digit groups — the form spreadsheets and other programs read.
    fn value_text(&self) -> Option<String> {
        let entry = self.entry.borrow();
        let value = entry
            .done()
            .map(|d| d.value)
            .or_else(|| entry.preview(self.angle.get()))?;
        let plain = Separators {
            group: None,
            ..self.sep
        };
        Some(format::number(value, plain).replace('−', "-"))
    }

    fn copy(&self, window: &Option<gtk::ApplicationWindow>) {
        if let (Some(text), Some(window)) = (self.value_text(), window) {
            window.clipboard().set_text(&text);
        }
    }

    fn paste(self: &Rc<Self>, window: &Option<gtk::ApplicationWindow>) {
        let Some(window) = window else { return };
        let calc = self.clone();
        window
            .clipboard()
            .read_text_async(None::<&gtk::gio::Cancellable>, move |text| {
                if let Ok(Some(text)) = text {
                    calc.entry
                        .borrow_mut()
                        .set_text(&format::internal(&text, calc.sep));
                    calc.refresh();
                }
            });
    }
}
