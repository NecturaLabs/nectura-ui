<!--
One focused change per pull request; a refactor goes in its own. The title takes commit-subject
form, `type(scope): summary` (CONTRIBUTING.md).
-->

## Summary

<!-- What was wrong or missing, what this does about it and why. Name any decision or shortcoming
     a reviewer should know about. -->

## Issue

<!-- Closes #N when this completes the issue's acceptance criteria, Refs #N when it does part of
     it, or None: <why>. -->

## Review focus

<!-- Where to look hardest, and what is safe to skim. -->

## Verification

<!-- The checks you ran and what you saw. "Tests pass" alone is not enough. -->

## Not verified

<!-- What you could not run or see, and why. None, if everything was checked. -->

## Screenshots

<!-- For a visible change: the component gallery in a dark and a light theme, dragged in here so
     GitHub hosts them. Otherwise one line: No visible change: <why> -->

## Checks

- [ ] `tools/check.sh` passes (format, clippy with `-D warnings`, tests).
- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) and are signed off (`git commit -s`).
- [ ] Colours, spacing and radii come only from `src/theme.rs`; no new `unsafe` outside `src/kit/glass/gl.rs`.
