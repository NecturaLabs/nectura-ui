# nectura-ui

egui component library shared by NecturaLabs applications (Canopy first). Rust 2024, MSRV 1.91.

- The library knows nothing about an application: no Git, no commands. The application's side
  comes through `host::Host` and `kit::Shortcut`.
- Colours, spacing and radii come only from `src/theme.rs`; icons from `egui-phosphor`.
- `unsafe` only in `src/kit/glass/gl.rs`, one operation per block with a `// SAFETY:` comment.
- Silence a lint only with `#[expect(lint, reason = "...")]` at the narrowest scope.
- State a test must not share between threads (look, motion, tooltip level, blur status) is
  thread-local under `cfg(any(test, feature = "testkit"))`; keep it that way.
- Check: `tools/check.sh`. Changes here reach an application by bumping its submodule pointer;
  run the application's checks (Canopy: `tools/check.sh`) before you do.
- Commits follow Conventional Commits. No credentials in the repository.
