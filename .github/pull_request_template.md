## What and why

<!-- What changes, and the issue it closes (Closes #N). -->

## Checks

- [ ] `tools/check.sh` passes (format, clippy with `-D warnings`, tests).
- [ ] A visible change: screenshots of the component gallery in a dark and a light theme.
- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) and are signed off (`git commit -s`).
- [ ] Colours, spacing and radii come only from `src/theme.rs`; no new `unsafe` outside `src/kit/glass/gl.rs`.
