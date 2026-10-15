# nectura-ui

NecturaLabs' egui component library: design tokens and themes, fonts, motion, in-code illustrations
and the component kit (buttons, fields, menus, overlays, lists, tabs, toasts, glass surfaces).
Extracted from Canopy; private. egui 0.36, Rust 2024, MSRV 1.91.

## Use it

Add it to your repository as a submodule and make it a path dependency:

```sh
git submodule add git@github.com:NecturaLabs/nectura-ui.git crates/nectura-ui
```

```toml
# Cargo.toml of the application
[dependencies]
nectura-ui = { path = "crates/nectura-ui" }
eframe = { version = "0.36", default-features = false, features = ["glow", "wayland", "x11"] }

[dev-dependencies]
nectura-ui = { path = "crates/nectura-ui", features = ["testkit"] }
```

At start-up install the host and the fonts, then apply a theme:

```rust
struct Host;
impl nectura_ui::host::Host for Host {
    // optional: the system's reduce-motion and reduce-transparency answers, the context-menu key
}
nectura_ui::host::install(&Host);
nectura_ui::fonts::install(&ctx);
nectura_ui::theme::apply(&ctx, nectura_ui::theme::ThemeId::Midnight, appearance);
```

`kit::Shortcut` carries the key hint a component shows (`.shortcut("Ctrl+K")`); convert your own
command type with `impl From<YourCommand> for Shortcut`.

The `testkit` feature exposes `kit::testkit` and keeps theme, motion and tooltip state per thread,
so an application's parallel tests do not leak into each other. Do not enable it in a release build.

## Develop

`tools/check.sh` runs format, clippy and the tests. The component gallery
(`kit::gallery::show`) draws every component in every state; the application passes its own
sections. Colours, spacing and radii come only from `src/theme.rs`. The only `unsafe` code is the
backdrop blur's OpenGL calls in `src/kit/glass/gl.rs`.

The bundled fonts need a licence row in the fonts folder's README and the licence text beside them.
