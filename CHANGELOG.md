# Changelog

All notable changes to nectura-ui are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html); before 1.0 a minor version may break
the API. Applications pin a commit through their submodule pointer; each release is a tag and a
[GitHub release](https://github.com/NecturaLabs/nectura-ui/releases).

## [Unreleased]

### Changed

- `tree_indent` is 14 instead of 20, so nested tree rows sit closer to their parent.
- `TreeRow::icon_tone` colours a row's chevron when the row has children and no icon.

## [0.1.0] - 2026-11-18

### Added

- The first public release of the component library extracted from Canopy: design tokens and
  themes with high-contrast variants, bundled fonts, motion with reduced-motion support, in-code
  illustrations, and the component kit (buttons, fields, menus, overlays, lists, tabs, toasts,
  glass surfaces with the backdrop blur), with a component gallery and a `testkit` feature for
  applications' tests.
- Community files, issue forms, CI (format, clippy, tests, documentation, Rust 1.95, `cargo deny`)
  and Dependabot.

### Changed

- The minimum supported Rust version is 1.95, which egui 0.36.2 requires.

### Fixed

- Broken and private links in the API documentation.

[Unreleased]: https://github.com/NecturaLabs/nectura-ui/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/NecturaLabs/nectura-ui/releases/tag/v0.1.0
