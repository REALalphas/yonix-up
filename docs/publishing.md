# Publication and release checklist

This checklist separates a public source repository from a supported binary release. No release, audit certification or compatibility promise is implied by adding these files.

## Source repository

- Review the full history reachable from branches and tags for credentials and private material. Removing a value in the latest commit does not remove it from history. Revoke any exposed credentials before publication.
- Git author identities and historical local paths remain visible in history. Review them before changing repository visibility; do not rewrite shared history silently.
- Confirm redistribution rights for every bundled archive. The current tree includes only the Alpha UI core and React 0.4.11 packages and their matching official source archive. Older archives remain in Git history; the library owner has confirmed ownership, while third-party material retains its upstream terms.
- Application metadata uses AGPL-3.0-or-later. Alpha UI retains AHCL-1.0. Its owner has confirmed ownership; the AHCL licensor exception does not extend to third-party or uncontrolled contributions.
- Review GitHub settings: description, topics, Issues, private vulnerability reporting, secret scanning where available, and branch protection or rulesets. Enable required checks only after the first CI run establishes their names and results.
- Commit the intended files, push, and verify CI on the pushed revision before changing visibility. Visibility is still a separate repository setting.

## Before distributing builds

- Provide complete corresponding source for the exact Yonix revision, lock files and build scripts, and a clear source offer to remote users where AGPL requires it. A link to a moving branch alone does not identify the source of an older binary.
- Provide the complete corresponding Alpha UI source required by its distribution terms. `vendor/alpha-ui/manifest.json` identifies `alpha-ui-source-0.4.11.tar.gz`, size 354448, with its SHA-512 integrity value. The matching official source archive is included unchanged in `vendor/alpha-ui/` and has been verified against the manifest. Keep it accessible with distributions of this version; package tarballs alone are not a substitute for the preferred source and build scripts.
- Clarify the combined distribution terms for recipients: Yonix is AGPL-3.0-or-later; Alpha UI is AHCL-1.0 with additional use conditions. Do not describe the complete bundled application as having exclusively open-source dependencies. Library ownership does not by itself resolve recipients' license obligations.
- Keep all third-party license texts with redistributed assets and packages. Nix packages install the notices under `share/doc/yonix/`.
- Run the documented frontend, Rust, NixOS module, cleanup and disposable VM checks. CI currently covers `x86_64-linux`; verify `aarch64-linux` separately before claiming a tested ARM release.
- Choose a release tag and write release notes with supported environments, known limitations, retention behavior and upgrade instructions. Do not mark an untested development snapshot as stable.

The archived Bash implementation is outside current maintenance. Do not publish it as a current installation method.

## Local preparation checks — 2026-11-13

The publication working tree was checked locally before committing or pushing:

- Gitleaks 8.30.1 scanned all local Git refs (104 commits), including archive inspection to depth 2: no leaks reported. A separate scan of the working snapshot and expanded current Alpha UI archives also reported no leaks. This is automated pattern detection, not a guarantee that no private information exists. Historical author identities and example paths were not removed.
- All three retained Alpha UI archives match the sizes and SHA-512 values in the official manifest.
- `npm ci --ignore-scripts --no-audit --no-fund`, production build/typecheck and all 14 frontend tests passed. The generated `web/` bundle was refreshed.
- Rust formatting, headless Clippy with warnings denied and the full headless test suite passed. A scheduling race found in the workflow test fixture was replaced with an explicit build-release gate.
- Nix built desktop and headless packages with their default Rust tests and evaluated the NixOS module check on a snapshot of the final package sources. Both installed packages contain byte-identical copies of the application and third-party license texts.
- Actionlint 1.7.12, local documentation file links and `git diff --check` passed.

The display-dependent GTK test, cleanup and NixOS VM checks, and ARM builds were not rerun during this preparation. GitHub-hosted CI has not been run for these unpushed changes. Repository description and topics were set; visibility remains private. Private vulnerability reporting availability could not be confirmed through the GitHub API (HTTP 404), so SECURITY.md documents a fallback.

Independent review found and corrected two documentation-link errors: upstream notices now retain their original `licenses/` relative layout, and the publication checklist is installed beside the notices. The unchanged upstream notices and license texts remain byte-identical to the official package.
