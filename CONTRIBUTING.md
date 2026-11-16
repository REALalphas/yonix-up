# Contributing

Yonix is experimental. Focused fixes, documentation improvements and reproducible bug reports are welcome. For larger changes, open an issue first to discuss the problem and intended behavior.

## Bug reports and feature requests

Use [GitHub Issues](https://github.com/REALalphas/yonix-up/issues). Bug reports are most useful with the Yonix revision, NixOS version, architecture, configuration type (flake or legacy), expected and actual behavior, and steps to reproduce. Remove API tokens, peer credentials, cache signing keys, private input URLs and personal paths from logs or configuration before sharing them.

For security vulnerabilities, follow [SECURITY.md](SECURITY.md) instead of posting details publicly. Be respectful and focus on the code and behavior; abusive or off-topic discussions may be closed.

## Development

Clone the repository and enter the pinned Rust and GTK development environment:

```sh
git clone https://github.com/REALalphas/yonix-up.git
cd yonix-up
nix develop
```

From the repository root, run the Rust checks used by CI:

```sh
nix develop --command cargo fmt --check
nix develop --command cargo clippy --locked --all-targets -- -D warnings
nix develop --command cargo clippy --locked --no-default-features --all-targets -- -D warnings
nix develop --command cargo test --locked
nix develop --command cargo test --locked --no-default-features
```

With Node.js 24 installed (the CI version), install the locked dependencies and run the frontend build and tests:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build
npm test
```

See [Development and verification](README.md#development-and-verification) for Nix package, NixOS module, GTK and disposable VM checks. Tests involving system activation or filesystem maintenance belong in disposable VMs; do not run maintenance commands against your workstation to verify a change. To scan the full Git history for accidentally committed credentials, use the CI command:

```sh
nix shell --inputs-from . nixpkgs#gitleaks --command gitleaks git --redact --max-archive-depth=2 --log-opts=--all .
```

## Pull requests

Keep changes focused. Explain the problem, resulting behavior and checks actually run; note unavailable checks. Add regression coverage when behavior changes, and update user-facing documentation and translations where needed. Frontend changes should rebuild the tracked `web/` assets with `npm run build`. Do not commit credentials, runtime state or unrelated generated files.

Use official Alpha UI releases and public APIs. Do not modify or patch the library, create a custom variant, or override component internals with application CSS. Preserve its separate license and notices.

## Licensing

Contribute only work you have the right to submit. Unless explicitly agreed otherwise, contributions to Yonix application code are submitted under AGPL-3.0-or-later, as described in [LICENSE](LICENSE). Separately licensed dependencies and assets retain their own terms. No contributor license agreement or copyright assignment is required.
