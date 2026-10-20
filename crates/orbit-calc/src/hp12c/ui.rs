//! The HP-12C keypad and display.
//!
//! Laid out as the 12c: four rows of ten keys, ENTER two rows tall; each
//! key's gold (f) function above it, its blue (g) function on its lower
//! face. Functions of programming, which is not implemented, are dimmed.
//!
//! Keyboard: digits, `.` or `,`, Enter, `+ − × ÷`, Backspace (←), Escape
//! (CLx), `f` and `g`; Ctrl+C copies X, Ctrl+V keys in a number.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};

use orbit_app::i18n::{number, tr_args};

use super::{Hp12c, Pad};
use crate::format::{self, Separators};

/// (key, label, gold function, blue function); "" for none, a leading `~`
/// for one not implemented, a leading `<` for Pango markup.
type KeySpec = (Pad, &'static str, &'static str, &'static str);

const ROWS: [[KeySpec; 10]; 4] = [
    [
        (Pad::N, "n", "AMORT", "12×"),
        (Pad::I, "i", "INT", "12÷"),
        (Pad::Pv, "PV", "NPV", "CF₀"),
        (Pad::Pmt, "PMT", "RND", "CFⱼ"),
        (Pad::Fv, "FV", "IRR", "Nⱼ"),
        (Pad::Chs, "CHS", "", "DATE"),
        (Pad::Digit(7), "7", "", "BEG"),
        (Pad::Digit(8), "8", "", "END"),
        (Pad::Digit(9), "9", "", "~MEM"),
        (Pad::Div, "÷", "", ""),
    ],
    [
        (Pad::Yx, "yˣ", "PRICE", "√x"),
        (Pad::Recip, "1/x", "YTM", "eˣ"),
        (Pad::PctT, "%T", "SL", "LN"),
        (Pad::DeltaPct, "Δ%", "SOYD", "FRAC"),
        (Pad::Pct, "%", "DB", "INTG"),
        (Pad::Eex, "EEX", "", "ΔDYS"),
        (Pad::Digit(4), "4", "", "D.MY"),
        (Pad::Digit(5), "5", "", "M.DY"),
        (
            Pad::Digit(6),
            "6",
            "",
            "<span overline=\"single\">x</span>w",
        ),
        (Pad::Mul, "×", "", "x²"),
    ],
    [
        (Pad::Rs, "~R/S", "~P/R", "~PSE"),
        (Pad::Sst, "~SST", "Σ", "~BST"),
        (Pad::RollDown, "R↓", "~PRGM", "~GTO"),
        (Pad::Swap, "x≷y", "FIN", "~x≤y"),
        (Pad::Clx, "CLx", "REG", "~x=0"),
        (Pad::Enter, "ENTER", "PREFIX", ""),
        (Pad::Digit(1), "1", "", "x̂,r"),
        (Pad::Digit(2), "2", "", "ŷ,r"),
        (Pad::Digit(3), "3", "", "n!"),
        (Pad::Sub, "−", "", "←"),
    ],
    [
        (Pad::On, "~ON", "", ""),
        (Pad::F, "f", "", ""),
        (Pad::G, "g", "", ""),
        (Pad::Sto, "STO", "", ""),
        (Pad::Rcl, "RCL", "", ""),
        // Under ENTER, which spans two rows.
        (Pad::Enter, "", "", ""),
        (Pad::Digit(0), "0", "", "<span overline=\"single\">x</span>"),
        // The label is the language's decimal separator.
        (Pad::Point, "", "", "s"),
        (Pad::SigmaPlus, "Σ+", "", "Σ−"),
        (Pad::Add, "+", "", "LSTx"),
    ],
];

struct View {
    calc: RefCell<Hp12c>,
    sep: Separators,
    number: gtk::Label,
    indicators: gtk::Label,
}

pub fn new(window: &gtk::ApplicationWindow) -> gtk::Widget {
    let view = Rc::new(View {
        calc: RefCell::new(Hp12c::default()),
        sep: Separators::from_catalog(),
        number: gtk::Label::builder()
            .xalign(1.0)
            .css_classes(["number"])
            .build(),
        indicators: gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["indicators"])
            .build(),
    });

    let lcd = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["lcd"])
        .build();
    lcd.append(&view.number);
    lcd.append(&view.indicators);

    let keypad = gtk::Grid::builder()
        .row_spacing(5)
        .column_spacing(10)
        .column_homogeneous(true)
        .css_classes(["keypad"])
        .build();
    for (row, keys) in ROWS.iter().enumerate() {
        for (col, &(pad, label, gold, blue)) in keys.iter().enumerate() {
            if label.is_empty() && pad != Pad::Point {
                continue; // the lower half of ENTER
            }
            let decimal = view.sep.decimal.to_string();
            let label = if pad == Pad::Point { &decimal } else { label };
            let tall = pad == Pad::Enter;
            let cell = key(&view, pad, label, gold, blue, tall);
            keypad.attach(&cell, col as i32, row as i32, 1, if tall { 2 } else { 1 });
        }
    }

    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(15)
        .css_classes(["hp12c"])
        .build();
    page.append(&lcd);
    page.append(&keypad);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let view = view.clone();
        let page = page.downgrade();
        let window = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            if !page.upgrade().is_some_and(|p| p.is_mapped()) {
                return glib::Propagation::Proceed;
            }
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                let Some(window) = window.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                match key.to_lower() {
                    gdk::Key::c => view.copy(&window),
                    gdk::Key::v => view.paste(&window),
                    _ => return glib::Propagation::Proceed,
                }
                return glib::Propagation::Stop;
            }
            let pads: &[Pad] = match key {
                gdk::Key::Return | gdk::Key::KP_Enter => &[Pad::Enter],
                gdk::Key::BackSpace => &[Pad::G, Pad::Sub],
                gdk::Key::Escape | gdk::Key::Delete => &[Pad::Clx],
                _ => match key.to_unicode() {
                    Some(c @ '0'..='9') => &[DIGITS[usize::from(c as u8 - b'0')]],
                    Some('.' | ',') => &[Pad::Point],
                    Some('+') => &[Pad::Add],
                    Some('-') => &[Pad::Sub],
                    Some('*' | '×') => &[Pad::Mul],
                    Some('/' | '÷') => &[Pad::Div],
                    Some('f' | 'F') => &[Pad::F],
                    Some('g' | 'G') => &[Pad::G],
                    _ => return glib::Propagation::Proceed,
                },
            };
            for &pad in pads {
                view.calc.borrow_mut().press(pad);
            }
            view.refresh();
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    view.refresh();
    page.upcast()
}

const DIGITS: [Pad; 10] = [
    Pad::Digit(0),
    Pad::Digit(1),
    Pad::Digit(2),
    Pad::Digit(3),
    Pad::Digit(4),
    Pad::Digit(5),
    Pad::Digit(6),
    Pad::Digit(7),
    Pad::Digit(8),
    Pad::Digit(9),
];

/// A label, dimmed when its text starts with `~` (not implemented). Text
/// starting with `<` is Pango markup: x̄ is drawn with an overline, since
/// the combining macron vanishes at small sizes.
fn function_label(text: &str, class: &str) -> gtk::Label {
    let (text, unused) = match text.strip_prefix('~') {
        Some(rest) => (rest, true),
        None => (text, false),
    };
    let label = gtk::Label::builder()
        .label(text)
        .use_markup(text.starts_with('<'))
        .css_classes([class])
        .build();
    if unused {
        label.add_css_class("unused");
    }
    label
}

/// One key: its gold function above, the key with its blue function below.
fn key(view: &Rc<View>, pad: Pad, label: &str, gold: &str, blue: &str, tall: bool) -> gtk::Box {
    let face = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build();
    face.append(&function_label(label, "main"));
    if !blue.is_empty() {
        face.append(&function_label(blue, "blue"));
    }
    let button = gtk::Button::builder()
        .child(&face)
        .css_classes(["key"])
        .focusable(false)
        .focus_on_click(false)
        .vexpand(tall)
        .build();
    match pad {
        Pad::F => button.add_css_class("f"),
        Pad::G => button.add_css_class("g"),
        Pad::Enter => button.add_css_class("enter"),
        _ => {}
    }
    {
        let view = view.clone();
        button.connect_clicked(move |_| {
            view.calc.borrow_mut().press(pad);
            view.refresh();
        });
    }
    let cell = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    // Always there, even empty, so all keys line up.
    cell.append(&function_label(gold, "gold"));
    cell.append(&button);
    cell
}

impl View {
    fn refresh(&self) {
        let calc = self.calc.borrow();
        let text = match calc.error() {
            Some(code) => tr_args("calc-hp-error", &[("code", number(code))]),
            None => calc.display(self.sep.decimal, self.sep.group),
        };
        self.number.set_label(&text);
        let a = calc.annunciators();
        let mut parts: Vec<String> = Vec::new();
        if a.f {
            parts.push("f".into());
        }
        if a.g {
            parts.push("g".into());
        }
        if a.begin {
            parts.push("BEGIN".into());
        }
        if a.dmy {
            parts.push("D.MY".into());
        }
        if let Some(day) = a.weekday {
            parts.push(format!("        {day}"));
        }
        // A space keeps the line's height when it is empty.
        let text = parts.join("   ");
        self.indicators
            .set_label(if text.is_empty() { " " } else { &text });
    }

    fn copy(&self, window: &gtk::ApplicationWindow) {
        let plain = Separators {
            group: None,
            ..self.sep
        };
        let x = self.calc.borrow().x();
        window
            .clipboard()
            .set_text(&format::number(x, plain).replace('−', "-"));
    }

    fn paste(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let view = self.clone();
        window
            .clipboard()
            .read_text_async(None::<&gtk::gio::Cancellable>, move |text| {
                let Ok(Some(text)) = text else { return };
                let text = format::internal(text.trim(), view.sep).replace('−', "-");
                if let Ok(x) = text.parse::<f64>() {
                    view.calc.borrow_mut().paste(x);
                    view.refresh();
                }
            });
    }
}
