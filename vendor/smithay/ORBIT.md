# Smithay, vendored

The `smithay` crate from https://github.com/Smithay/smithay at commit
`24647fc8d00131eb7f388efed26e35ea316b0018` (MIT, see `LICENSE.txt`), used
through `[patch]` in the workspace `Cargo.toml`. Only the crate itself is
kept; its examples, benches, dev-dependencies, workspace and profile
sections are dropped from `Cargo.toml`.

One change, in `src/wayland/dmabuf/mod.rs`: a linux-dmabuf global with
default feedback is created at version 5 instead of 6.

- Mesa 26 (as bundled in the Ghostty AppImage) no longer reads wl_drm, so
  without feedback its GL clients fall back to software (Ghostty refuses
  to start: it needs OpenGL 4.3).
- Version 6 no longer sends `main_device`, and GTK 4.18 (Debian 13) crashes
  on it while opening the display. Version 5 sends it.

Drop this copy when Smithay lets the compositor cap the version, or when
the GTK in use takes version 6.
