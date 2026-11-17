# nectura-ui

NecturaLabs' egui component library: design tokens and themes, fonts, motion, in-code illustrations
and the component kit (buttons, fields, menus, overlays, lists, tabs, toasts, glass surfaces).
Extracted from [Canopy](https://github.com/NecturaLabs/Canopy). egui 0.36, Rust 2024, MSRV 1.91.

## Use it

Add it to your repository as a submodule and make it a path dependency:

```sh
git submodule add https://github.com/NecturaLabs/nectura-ui.git crates/nectura-ui
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

## Licence

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option. Copyright (c) 2026 NecturaLabs. The bundled fonts keep
their own licences (listed in the fonts folder's README).

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
nectura-ui, as defined in the Apache-2.0 licence, is dual licensed as above, without any additional
terms or conditions. Report security problems privately through the repository's Security tab
(Report a vulnerability) or to support@necturalabs.com, never in a public issue.
