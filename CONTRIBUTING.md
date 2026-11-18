# Contributing

nectura-ui is NecturaLabs' egui component library, used first by
[Canopy](https://github.com/NecturaLabs/Canopy). Contributions are welcome. [AGENTS.md](AGENTS.md)
holds the conventions in short; report security problems privately, as [SECURITY.md](SECURITY.md)
says, never in a public issue.

## Your first contribution

1. Find an issue: [`good first issue`](https://github.com/NecturaLabs/nectura-ui/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
   and [`help wanted`](https://github.com/NecturaLabs/nectura-ui/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
   mark ones that are ready. Say in the issue that you are working on it.
2. For a new component or a change to a token, open an issue first and agree the design.
3. Fork, make a branch, change the code with its tests, and run `tools/check.sh`.
4. Commit with [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
   (`feat(kit): ...`, `fix(theme): ...`) and a sign-off (`git commit -s`).
5. Open a pull request against `main`, fill in the template and link the issue.

## What fits

The library knows nothing about an application: no Git, no commands, no network or file access.
An application's side comes through `host::Host` and `kit::Shortcut`. Colours, spacing and radii
come only from `src/theme.rs`, icons from `egui-phosphor`, and illustrations are drawn in code.
`unsafe` code is accepted only in `src/kit/glass/gl.rs`, one operation per block with a
`// SAFETY:` comment. Bundled fonts and assets need an OFL, MIT, Apache-2.0, CC0 or similar
licence, with a row in the fonts folder's README.

## Checks

`tools/check.sh` runs `cargo fmt --check`, clippy with `-D warnings` and the tests; CI also builds
the documentation, checks the minimum Rust version (1.95) and runs `cargo deny check`. A visible
change needs screenshots of the component gallery (`kit::gallery::show`) in a dark and a light
theme. Silence a lint only with `#[expect(lint, reason = "...")]` at the narrowest scope.

A change reaches an application when the application bumps its submodule pointer; for Canopy,
its own `tools/check.sh` runs before that bump.

## Licence of contributions

nectura-ui is licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option. Unless you explicitly state otherwise, any contribution
you intentionally submit for inclusion, as defined in the Apache-2.0 licence, is dual licensed as
above, without any additional terms or conditions. Sign off each commit (`git commit -s`) to
certify the [Developer Certificate of Origin](https://developercertificate.org/). Commits made on
the GitHub website are signed off automatically.

## What to expect

The library has one maintainer, so reviews happen when there is time. A new issue or pull request
usually gets a first reply within a week. A pull request is merged once its checks pass and the
review's blocking points are resolved. How decisions are made: [GOVERNANCE.md](GOVERNANCE.md).
