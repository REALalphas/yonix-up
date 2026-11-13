# Contributing to Yonix

Yonix is experimental. Focused fixes, documentation improvements and reproducible bug reports are welcome. Larger changes should start with an issue explaining the problem and intended behavior.

## Reports

Use [GitHub Issues](https://github.com/REALalphas/yonix-up/issues) for bugs and feature requests. Include the Yonix revision, architecture, NixOS version, configuration type (flake or legacy), expected behavior and a minimal reproduction. Remove API tokens, peer credentials, cache signing keys, private input URLs and personal paths from logs. Report vulnerabilities using [SECURITY.md](SECURITY.md).

Be respectful, discuss the code and behavior, and avoid personal attacks or harassment. Maintainers may close abusive or off-topic discussions.

## Local development

Clone the repository and enter the pinned development environment:

```sh
git clone https://github.com/REALalphas/yonix-up.git
cd yonix-up
nix develop
npm ci --ignore-scripts --no-audit --no-fund
npm run build
npm test
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
```

Before submitting changes, scan for accidentally committed credentials:

```sh
nix shell --inputs-from . nixpkgs#gitleaks --command gitleaks git --redact --max-archive-depth=2 --log-opts=--all .
```

See the [README](README.md#development-and-verification) for GTK, NixOS module, cleanup and disposable VM checks. Tests that exercise system activation or filesystem maintenance belong in disposable VMs; do not run maintenance commands against your workstation to verify a patch.

## Pull requests

Keep changes focused. Explain the problem, resulting behavior and checks you actually ran. Add regression coverage when behavior changes; document unavailable checks. Update user-facing documentation and translations where needed. Do not commit credentials, runtime state or unrelated generated files. When frontend code changes, rebuild the tracked `web/` assets with `npm run build`; Nix rebuilds them independently at package-build time.

Use the official Alpha UI packages and public APIs. Read [ALPHA_UI_AGENT_RULES.md](ALPHA_UI_AGENT_RULES.md) before changing the browser client. Record missing library behavior in `ui-unspec/`; do not patch archives, installed packages or component internals with application CSS. Preserve the library's separate license and notices.

## Licensing

Contribute only work you have the right to submit. Unless explicitly agreed otherwise, contributions to Yonix application code are submitted under AGPL-3.0-or-later, as described in [LICENSE](LICENSE). Separately licensed dependencies and assets retain their own terms. No contributor license agreement or copyright assignment is required.
