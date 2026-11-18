# Security policy

## Reporting a vulnerability

Report security problems privately, never in a public issue or pull request:

- on GitHub, through private vulnerability reporting:
  <https://github.com/NecturaLabs/nectura-ui/security/advisories/new>, or
- by e-mail to support@necturalabs.com, with "Security" and "nectura-ui" in the subject.

Say what is affected, how to reproduce it and what an attacker gains.

## What happens next

1. We acknowledge the report within 7 days.
2. We confirm or rule out the vulnerability and tell you which, within 14 days of the report where
   we can.
3. We fix it privately in a GitHub security advisory for this repository, asking GitHub for a CVE
   when the issue affects applications that use the library.
4. We publish the fix, update the applications that use it (Canopy), then publish the advisory,
   crediting you unless you ask us not to.

We follow coordinated disclosure: please keep the details private until the advisory is published,
or for 90 days from your report if that comes first. If a fix needs longer, we agree a date with
you.

## Supported versions

Only the latest commit on `main` receives fixes; applications pin a commit through their submodule
pointer and move it forward.

## Scope

The library draws egui widgets and makes no network or file-system access of its own. Of most
interest: the only `unsafe` code, the backdrop blur's OpenGL calls in `src/kit/glass/gl.rs`, and
anything that could make a widget mislead a user about what an action does.
