//! The IPv4 subnet profile: an address and a prefix in, the network around
//! it out.
//!
//! The address field takes `10.0.0.199/24`, `10.0.0.199/255.255.255.0`, the
//! same with a space, or a bare address with the prefix from the spin button
//! beside it. The two stay in step: a prefix typed in the field moves the
//! spin button, and the spin button rewrites the field's prefix.

use std::{cell::Cell, net::Ipv4Addr, rc::Rc};

use gtk::prelude::*;
use orbit_app::i18n::tr;

use crate::{
    format::{self, Separators},
    ipv4::{self, Kind, ParseError, Subnet},
};

struct Page {
    address: gtk::Entry,
    prefix: gtk::SpinButton,
    message: gtk::Label,
    values: Vec<gtk::Label>,
    sep: Separators,
    /// Set while one field updates the other, so they don't loop.
    syncing: Cell<bool>,
}

/// The result rows, by message id of their label.
const ROWS: &[&str] = &[
    "calc-subnet-address",
    "calc-subnet-mask",
    "calc-subnet-wildcard",
    "calc-subnet-network",
    "calc-subnet-broadcast",
    "calc-subnet-first",
    "calc-subnet-last",
    "calc-subnet-hosts",
    "calc-subnet-size",
    "calc-subnet-class",
    "calc-subnet-kind",
    "calc-subnet-binary-address",
    "calc-subnet-binary-mask",
    "calc-subnet-hex-address",
    "calc-subnet-hex-mask",
];

pub fn new() -> gtk::Widget {
    let address = gtk::Entry::builder()
        .placeholder_text("10.0.0.199/24")
        .hexpand(true)
        .css_classes(["address"])
        .build();
    let prefix = gtk::SpinButton::with_range(0.0, 32.0, 1.0);
    prefix.set_value(24.0);
    prefix.set_tooltip_text(Some(&tr("calc-subnet-prefix")));

    let inputs = gtk::Box::builder().spacing(5).build();
    inputs.append(&address);
    inputs.append(&gtk::Label::new(Some("/")));
    inputs.append(&prefix);

    let message = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["message"])
        .build();

    let grid = gtk::Grid::builder()
        .row_spacing(5)
        .column_spacing(15)
        .css_classes(["results"])
        .build();
    let mut values = Vec::new();
    for (row, id) in ROWS.iter().enumerate() {
        let name = gtk::Label::builder()
            .label(tr(id))
            .xalign(1.0)
            .css_classes(["name"])
            .build();
        // As wide as the longest value, the binary form (36 characters, and
        // a little room), from the start: the window keeps its size as
        // results come and go.
        let value = gtk::Label::builder()
            .xalign(0.0)
            .width_chars(38)
            .max_width_chars(38)
            .selectable(true)
            .css_classes(["value"])
            .build();
        grid.attach(&name, 0, row as i32, 1, 1);
        grid.attach(&value, 1, row as i32, 1, 1);
        values.push(value);
    }

    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["subnet"])
        .build();
    page.append(&inputs);
    page.append(&message);
    page.append(&grid);

    let state = Rc::new(Page {
        address,
        prefix,
        message,
        values,
        sep: Separators::from_catalog(),
        syncing: Cell::new(false),
    });
    {
        let state = state.clone();
        state
            .address
            .clone()
            .connect_changed(move |_| state.address_changed());
    }
    {
        let state = state.clone();
        state
            .prefix
            .clone()
            .connect_value_changed(move |_| state.prefix_changed());
    }
    state.update();
    page.upcast()
}

impl Page {
    fn address_changed(&self) {
        if self.syncing.get() {
            return;
        }
        if let Ok((_, Some(prefix))) = ipv4::parse(&self.address.text()) {
            self.syncing.set(true);
            self.prefix.set_value(f64::from(prefix));
            self.syncing.set(false);
        }
        self.update();
    }

    fn prefix_changed(&self) {
        if self.syncing.get() {
            return;
        }
        // A prefix written in the field follows the spin button.
        let text = self.address.text();
        if let Some((addr, _)) = text.split_once(['/', ' ']) {
            self.syncing.set(true);
            let position = self.address.position();
            self.address
                .set_text(&format!("{}/{}", addr, self.prefix.value_as_int()));
            self.address.set_position(position);
            self.syncing.set(false);
        }
        self.update();
    }

    fn update(&self) {
        let text = self.address.text();
        self.address.remove_css_class("error");
        let parsed = ipv4::parse(&text);
        let subnet = match parsed {
            Ok((address, prefix)) => {
                let prefix = prefix.unwrap_or(self.prefix.value_as_int().clamp(0, 32) as u8);
                Some(Subnet::new(address, prefix))
            }
            Err(_) => None,
        };
        let message = match (parsed, text.trim().is_empty()) {
            (_, true) => tr("calc-subnet-hint"),
            (Err(ParseError::Address), _) => tr("calc-subnet-bad-address"),
            (Err(ParseError::Mask), _) => tr("calc-subnet-bad-mask"),
            (Ok(_), _) => String::new(),
        };
        if parsed.is_err() && !text.trim().is_empty() {
            self.address.add_css_class("error");
        }
        // Always shown, even empty, so the page keeps its height.
        self.message.set_label(&message);

        let rows = subnet.map(|s| self.rows(&s));
        for (i, label) in self.values.iter().enumerate() {
            let value = rows
                .as_ref()
                .and_then(|r| r.get(i).cloned())
                .unwrap_or_else(|| "—".into());
            label.set_label(&value);
        }
    }

    /// The value of each row of ROWS, in order.
    fn rows(&self, s: &Subnet) -> Vec<String> {
        let count = |n: u64| format::number(n as f64, self.sep);
        let none = || tr("calc-subnet-none");
        vec![
            s.address.to_string(),
            format!("{}  (/{})", s.mask, s.prefix),
            s.wildcard.to_string(),
            format!("{}/{}", s.network, s.prefix),
            s.broadcast.map_or_else(none, |b| b.to_string()),
            s.first.to_string(),
            s.last.to_string(),
            count(s.hosts),
            count(s.size),
            s.class.to_string(),
            tr(kind_id(s.kind)),
            ipv4::binary(s.address, s.prefix),
            ipv4::binary(Ipv4Addr::from_bits(ipv4::mask_of(s.prefix)), s.prefix),
            ipv4::hex(s.address),
            ipv4::hex(s.mask),
        ]
    }
}

fn kind_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Private => "calc-kind-private",
        Kind::Shared => "calc-kind-shared",
        Kind::Loopback => "calc-kind-loopback",
        Kind::LinkLocal => "calc-kind-link-local",
        Kind::Multicast => "calc-kind-multicast",
        Kind::Reserved => "calc-kind-reserved",
        Kind::Broadcast => "calc-kind-broadcast",
        Kind::Public => "calc-kind-public",
    }
}
