//! Virtual monitors: outputs with no screen behind them, made on request
//! (`output NAME create 1920x1080@60`) for orbit-remote-server, whose
//! client then sees a monitor of its own size instead of a mirror of the
//! local ones (§9, phase 2). They take a place, an area and workspaces as
//! any monitor does; nothing draws them but the screen copies.
//!
//! A timer at the mode's rate stands in for the vblank: each tick tells
//! the clients on it they may draw (frame callbacks) and lets the screen
//! copies waiting for a change look again.

use std::time::Duration;

use smithay::{
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::backend::GlobalId,
    },
    utils::Transform,
};

use crate::state::Orbit;

/// The most a virtual monitor may be, each way, and its rate.
const LARGEST: u16 = 7680;
const FASTEST: f64 = 240.0;

pub struct Virtual {
    output: Output,
    global: GlobalId,
    ticks: RegistrationToken,
}

impl Orbit {
    /// The virtual monitors' names.
    pub fn virtual_names(&self) -> Vec<String> {
        self.virtuals.iter().map(|v| v.output.name()).collect()
    }

    /// Makes virtual monitor `name`, `width`×`height` at `rate` Hz.
    pub fn create_virtual(&mut self, name: &str, (width, height): (u16, u16), rate: f64) -> Result<(), String> {
        if self.output_named(name).is_some() {
            return Err(format!("There is already an output called {name}"));
        }
        if !(1..=LARGEST).contains(&width) || !(1..=LARGEST).contains(&height) || !(1.0..=FASTEST).contains(&rate) {
            return Err(format!("output {name}: no virtual monitor of {width}x{height}@{rate}"));
        }
        let output = Output::new(
            name.to_owned(),
            PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "Orbit".into(), model: "Virtual".into(), serial_number: String::new() },
        );
        let global = output.create_global::<Orbit>(&self.display_handle);
        let mode = Mode { size: (i32::from(width), i32::from(height)).into(), refresh: (rate * 1000.0).round() as i32 };
        output.set_preferred(mode);
        output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
        let period = Duration::from_secs_f64(1.0 / rate);
        let ticking = output.clone();
        let ticks = self
            .loop_handle
            .insert_source(Timer::from_duration(period), move |_, _, state| {
                state.screencopy_after_frame(&ticking);
                crate::udev::send_frames(state, &ticking);
                TimeoutAction::ToDuration(period)
            })
            .map_err(|err| format!("output {name}: {err}"))?;
        tracing::info!("{name}: a virtual monitor, {width}x{height}@{rate}");
        self.virtuals.push(Virtual { output: output.clone(), global, ticks });
        self.add_output(&output);
        self.request_render();
        Ok(())
    }

    /// Takes virtual monitor `name` away; its windows go to the others.
    pub fn remove_virtual(&mut self, name: &str) -> Result<(), String> {
        let Some(i) = self.virtuals.iter().position(|v| v.output.name() == name) else {
            return Err(format!("There is no virtual output called {name}"));
        };
        let gone = self.virtuals.remove(i);
        self.loop_handle.remove(gone.ticks);
        self.display_handle.disable_global::<Orbit>(gone.global.clone());
        self.remove_output(&gone.output);
        // Removed for good once clients had time to see it disabled.
        let (handle, global) = (self.display_handle.clone(), gone.global);
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(5)), move |_, _, _| {
            handle.remove_global::<Orbit>(global.clone());
            TimeoutAction::Drop
        });
        tracing::info!("{name}: the virtual monitor is gone");
        self.request_render();
        Ok(())
    }
}
