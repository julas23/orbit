//! The touchpads and mice, set up as core.yaml says (`touchpad:`,
//! `mouse:`) when they are plugged in and at each `reload`: tap to click,
//! natural scrolling, speed and acceleration, disable while typing, how a
//! click and a scroll are made, left-handed buttons. libinput's own
//! defaults for what the file leaves out — but tapping and natural
//! scrolling on a touchpad, which most people expect and libinput leaves
//! off.

use serde::Deserialize;
use smithay::reexports::input::{AccelProfile, ClickMethod, Device, DeviceCapability, ScrollMethod};

use crate::state::Orbit;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Touchpad {
    pub tap: bool,
    pub natural_scroll: bool,
    /// From -1 (slowest) to 1 (fastest); 0 is libinput's middle.
    pub speed: f64,
    pub disable_while_typing: bool,
    /// Left and right together make a middle click.
    pub middle_emulation: bool,
    /// `clickfinger` (one, two, three fingers: left, right, middle) or
    /// `areas` (the bottom right corner is the right button).
    pub click: Option<String>,
    /// `two_finger`, `edge`, or `none`.
    pub scroll: Option<String>,
    pub left_handed: bool,
}

impl Default for Touchpad {
    fn default() -> Self {
        Self {
            tap: true,
            natural_scroll: true,
            speed: 0.0,
            disable_while_typing: true,
            middle_emulation: false,
            click: None,
            scroll: None,
            left_handed: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mouse {
    pub natural_scroll: bool,
    pub speed: f64,
    /// `adaptive` (libinput's) or `flat` (no acceleration, for games).
    pub profile: Option<String>,
    pub left_handed: bool,
}

/// Whether `device` is a touchpad: it can be tapped.
fn is_touchpad(device: &Device) -> bool {
    device.config_tap_finger_count() > 0
}

/// Sets `device` up as `touchpad` and `mouse` say.
pub fn configure(device: &mut Device, touchpad: &Touchpad, mouse: &Mouse) {
    if !device.has_capability(DeviceCapability::Pointer) {
        return;
    }
    let name = device.name().into_owned();
    let warn = |what: &str, result: Result<(), smithay::reexports::input::DeviceConfigError>| {
        if let Err(err) = result {
            tracing::debug!("{name}: {what}: {err:?}");
        }
    };
    if is_touchpad(device) {
        let t = touchpad;
        warn("tap", device.config_tap_set_enabled(t.tap));
        warn(
            "natural scroll",
            device.config_scroll_set_natural_scroll_enabled(t.natural_scroll),
        );
        warn("speed", device.config_accel_set_speed(t.speed.clamp(-1.0, 1.0)));
        warn(
            "disable while typing",
            device.config_dwt_set_enabled(t.disable_while_typing),
        );
        warn(
            "middle emulation",
            device.config_middle_emulation_set_enabled(t.middle_emulation),
        );
        warn("left handed", device.config_left_handed_set(t.left_handed));
        match t.click.as_deref() {
            Some("clickfinger") => warn("click", device.config_click_set_method(ClickMethod::Clickfinger)),
            Some("areas") => warn("click", device.config_click_set_method(ClickMethod::ButtonAreas)),
            Some(other) => tracing::warn!("touchpad click: {other}? clickfinger or areas"),
            None => {}
        }
        match t.scroll.as_deref() {
            Some("two_finger") => warn("scroll", device.config_scroll_set_method(ScrollMethod::TwoFinger)),
            Some("edge") => warn("scroll", device.config_scroll_set_method(ScrollMethod::Edge)),
            Some("none") => warn("scroll", device.config_scroll_set_method(ScrollMethod::NoScroll)),
            Some(other) => tracing::warn!("touchpad scroll: {other}? two_finger, edge or none"),
            None => {}
        }
        tracing::info!("{name}: a touchpad, set up");
    } else {
        let m = mouse;
        warn(
            "natural scroll",
            device.config_scroll_set_natural_scroll_enabled(m.natural_scroll),
        );
        warn("speed", device.config_accel_set_speed(m.speed.clamp(-1.0, 1.0)));
        warn("left handed", device.config_left_handed_set(m.left_handed));
        match m.profile.as_deref() {
            Some("flat") => warn("profile", device.config_accel_set_profile(AccelProfile::Flat)),
            Some("adaptive") => warn("profile", device.config_accel_set_profile(AccelProfile::Adaptive)),
            Some(other) => tracing::warn!("mouse profile: {other}? adaptive or flat"),
            None => {}
        }
    }
}

impl Orbit {
    /// A device plugged in (or found at the start).
    pub fn device_added(&mut self, mut device: Device) {
        configure(&mut device, &self.config.touchpad, &self.config.mouse);
        self.devices.push(device);
    }

    pub fn device_removed(&mut self, device: &Device) {
        self.devices.retain(|d| d != device);
    }

    /// The config was read again: every device set up anew.
    pub fn configure_devices(&mut self) {
        let (touchpad, mouse) = (self.config.touchpad.clone(), self.config.mouse.clone());
        for device in &mut self.devices {
            configure(device, &touchpad, &mouse);
        }
    }
}
