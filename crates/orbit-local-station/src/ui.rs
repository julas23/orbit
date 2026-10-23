//! The station's own screens (the server's address, connecting, the
//! pairing code, what went wrong), in Slint. Slint draws them with its software renderer into a
//! buffer of the monitor's size, which the station puts on the screen
//! (`screen::draw`); the keys never go to Slint, the station types the code
//! into it (`Station::key`).

use std::{rc::Rc, time::Instant};

use slint::{
    ComponentHandle, PhysicalSize,
    platform::{
        Platform, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType},
    },
};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::{Rectangle, Transform},
};

slint::slint! {
    export component Face inherits Window {
        in property <string> server;
        in property <string> status;
        in property <bool> asking;
        in property <string> code;
        in property <string> problem;
        // The server's address being typed (F2, or no server yet).
        in property <bool> editing;
        in property <string> address;
        in property <string> keys;
        // Sizes follow the screen (a 54th of its height, set by `Ui::new`):
        // a 4K monitor gets the same look as a 1080p one.
        in property <length> unit: 20px;

        background: #0d1117;
        VerticalLayout {
            alignment: center;
            spacing: root.unit * 2;
            Text {
                text: "Orbit";
                font-size: root.unit * 5;
                font-weight: 300;
                color: #e6edf3;
                horizontal-alignment: center;
            }
            Text {
                text: root.server;
                font-size: root.unit * 1.2;
                color: #7d8590;
                horizontal-alignment: center;
            }
            Text {
                text: root.status;
                font-size: root.unit * 1.6;
                color: #c9d1d9;
                horizontal-alignment: center;
            }
            if root.editing: HorizontalLayout {
                alignment: center;
                Rectangle {
                    width: root.unit * 24;
                    height: root.unit * 3;
                    border-radius: root.unit * 0.5;
                    border-width: 2px;
                    border-color: #58a6ff;
                    background: #161b22;
                    Text {
                        text: root.address + "▏";
                        font-size: root.unit * 1.6;
                        color: #e6edf3;
                    }
                }
            }
            if root.asking: HorizontalLayout {
                alignment: center;
                spacing: root.unit;
                for i in 6: Rectangle {
                    width: root.unit * 3.2;
                    height: root.unit * 4;
                    border-radius: root.unit * 0.5;
                    border-width: 2px;
                    border-color: i == root.code.character-count ? #58a6ff : #30363d;
                    background: #161b22;
                    Text {
                        text: i < root.code.character-count ? "•" : "";
                        font-size: root.unit * 2.4;
                        color: #e6edf3;
                    }
                }
            }
            Text {
                text: root.problem;
                font-size: root.unit * 1.2;
                color: #f85149;
                horizontal-alignment: center;
            }
        }
        Text {
            x: 0;
            y: parent.height - self.height - root.unit * 2;
            width: parent.width;
            text: root.keys;
            font-size: root.unit;
            color: #7d8590;
            horizontal-alignment: center;
        }
    }
}

/// Slint's platform: one window, the screen.
struct Station {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for Station {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.start.elapsed()
    }
}

pub struct Ui {
    window: Rc<MinimalSoftwareWindow>,
    pub face: Face,
    pub buffer: MemoryRenderBuffer,
    width: usize,
}

impl Ui {
    /// The screens, at the monitor's `size`.
    pub fn new((width, height): (u32, u32)) -> anyhow::Result<Self> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(Station { window: window.clone(), start: Instant::now() }))
            .map_err(|e| anyhow::anyhow!("Slint: {e}"))?;
        let face = Face::new().map_err(|e| anyhow::anyhow!("Slint: {e}"))?;
        window.set_size(PhysicalSize::new(width, height));
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        face.set_unit(height as f32 / 54.0);
        face.show().map_err(|e| anyhow::anyhow!("Slint: {e}"))?;
        let buffer = MemoryRenderBuffer::new(Fourcc::Abgr8888, (width as i32, height as i32), 1, Transform::Normal, None);
        Ok(Self { window, face, buffer, width: width as usize })
    }

    /// Lets Slint's timers and animations run, and draws what changed into
    /// the buffer; true if something did.
    pub fn update(&mut self) -> bool {
        slint::platform::update_timers_and_animations();
        let width = self.width;
        let mut drawn = false;
        let mut context = self.buffer.render();
        self.window.draw_if_needed(|renderer| {
            drawn = true;
            let _ = context.draw(|bytes| {
                // SAFETY: the pixels are four bytes each with an alignment of
                // one (`repr(C)` of four `u8`s), and the buffer is a whole
                // number of them, in the same order as DRM's ABGR8888.
                let pixels = unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<PremultipliedRgbaColor>(), bytes.len() / 4) };
                let region = renderer.render(pixels, width);
                Ok::<_, ()>(
                    region
                        .iter()
                        .map(|(at, size)| Rectangle::new((at.x, at.y).into(), (size.width as i32, size.height as i32).into()))
                        .collect(),
                )
            });
        });
        drawn
    }

    /// How long until Slint has something to do (an animation running, a timer).
    pub fn next_wake(&self) -> Option<std::time::Duration> {
        if self.window.has_active_animations() {
            return Some(std::time::Duration::from_millis(16));
        }
        slint::platform::duration_until_next_timer_update()
    }
}
