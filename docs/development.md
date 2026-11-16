# Development and Verification

This document details local development workflows, code standards, automated test suites, and packaging details for Yonix.

## Development Environment

Enter the Nix development shell to access the Rust toolchain, Node.js, GTK 4, libadwaita, and build tools:

```console
nix develop
```

---

## Code Quality and Verification Commands

### Code Formatting and Linting

```console
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --no-default-features --all-targets -- -D warnings
```

### Unit and Integration Tests

Run the Rust test suite (desktop and headless):

```console
# Full suite with GTK features
cargo test --locked

# Headless / server-only crate test
cargo test --locked --no-default-features
```

End-to-end tests exercise the real daemon, CLI, job plans, concurrency serialization, origin/Host validation, and state persistence with mocked Nix binaries, ensuring no accidental modifications to the host system.

### Frontend Typechecking and Tests

The web panel and translations are verified using Node.js:

```console
npm ci --ignore-scripts --no-audit --no-fund
npm run build   # Typechecks TypeScript and compiles production assets
npm test        # Validates translations, models, and UI logic
```

### Display-Dependent GTK Test

A native GTK 4 window integration test is included under an `--ignored` flag to avoid requiring an active display server during normal test runs. To run it in an isolated headless X11 session:

```console
nix shell nixpkgs#xvfb-run nixpkgs#dbus --command \
  xvfb-run -a dbus-run-session --config-file=tests/dbus-session.conf -- \
  env GDK_BACKEND=x11 GTK_A11Y=none \
  cargo test --lib native_window_renders_and_connects -- --ignored --test-threads=1
```

---

## Nix Builds and Checks

The project includes reproducible Nix derivations and isolated VM test environments:

```console
# Build desktop package (GTK 4 / libadwaita)
nix build

# Build headless server package (no GTK dependencies)
nix build .#yonix-headless

# NixOS module option validation
nix build .#checks.x86_64-linux.module

# Policy, ioctl fixtures, and cleanup isolation/failure tests
nix build .#checks.x86_64-linux.clean

# Disposable Btrfs VM test (block-group trees, convergence, data checksums)
nix build .#checks.x86_64-linux.btrfs

# Disposable NixOS VM test (service, build, activation, rollback, permissions)
nix build .#checks.x86_64-linux.vm

# Disposable peering VM test (two-host management, signed cache substitution)
nix build .#checks.x86_64-linux.peers
```

> [!NOTE]
> Tests involving system activation or filesystem maintenance run exclusively inside disposable NixOS virtual machines on KVM-enabled runners. Never run raw maintenance tests directly against production workstations.

---

## Continuous Integration (CI)

GitHub Actions runs `.github/workflows/ci.yml` on pushes, pull requests, and manual triggers:
- **Repository Hygiene**: Runs `actionlint` to validate GitHub workflows and `gitleaks` to scan Git history for accidental secrets.
- **Frontend**: Audits TypeScript types, bundles production assets, and verifies all 13 localization catalogs.
- **Rust & GTK**: Runs `cargo fmt`, `clippy`, unit tests, and the headless Xvfb GTK test.
- **Nix matrix**: Builds both packages, validates module definitions, and executes all VM-based test suites.

CI requires no repository secrets and executes using pinned dependencies from `flake.lock`, `Cargo.lock`, and `package-lock.json`.

---

## Manual Uninstalled Execution

To run and debug Yonix without installing it system-wide:

1. Generate a sample configuration:
   ```console
   yonix init > config.json
   ```
2. Edit `config.json` to specify absolute paths for your checkout and state directories, and configure your hostname.
3. Start the daemon in one terminal:
   ```console
   cargo run -- --config config.json serve
   ```
4. In another terminal, connect with the desktop client:
   ```console
   cargo run -- --config config.json gui
   ```

*(Privileged system actions such as `switch` or `clean` require the installed NixOS helper and are unavailable during uninstalled user runs.)*

---

## Packaging and Build Details

- **Headless package**: `yonix-headless` excludes GTK 4, libadwaita, and graphic libraries, making it ideal for servers and minimal installations.
- **Embedded Web Assets**: The React frontend is compiled and committed to `web/`. When `build.rs` compiles the daemon, assets in `web/` are embedded directly into the binary.
- **Vendor Alpha UI**: The browser interface uses official Alpha UI release archives stored in `vendor/alpha-ui/`. The Nix frontend build references these offline archives without requiring Internet access during the build.
- **Optimization**: Release builds use Cargo's parallel code generation without LTO to balance runtime performance with reasonable compilation times.

---

## Adding a New Interface Language

Yonix supports 13 languages across both the desktop client and web panel. To contribute a new language:

1. Create a translation file: `locales/<lang-code>.json` using the same keys as `locales/en.json`.
2. Register the language in the Rust backend: `src/i18n.rs`.
3. Register the language and display name in the frontend: `frontend/i18n.ts`.
4. Run `npm test` and `cargo test` to ensure all keys match and pass validation.
