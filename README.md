# Yonix

Yonix manages NixOS configurations from a native GTK 4/libadwaita desktop app, the command line, or a browser panel. Each client communicates with the same unified Rust daemon.

**Experimental, version 0.1.0.** Yonix can activate NixOS configurations and remove old generations. Review the operation and retention settings before enabling system changes. Compatibility is not guaranteed, and no release support schedule is established.

- **Native GTK 4, CLI, and Web**: Modern libadwaita desktop interface, full-featured CLI, and responsive browser panel.
- **13 Interface Languages**: English, Russian, German, French, Spanish, Portuguese (Brazil), Italian, Ukrainian, Polish, Turkish, Simplified Chinese, Japanese, and Korean.
- **Flake and Legacy Support**: Automatic runtime detection for modern flakes and traditional `configuration.nix` setups.
- **Safe System Maintenance**: Automated Btrfs chunk compaction, Nix store deduplication, integrity verification, and tmpfiles/journal cleanup.
- **Multi-Host Management**: Manage remote Yonix instances through one panel and share signed Nix binary caches.
- **Reproducible Nix Packages**: Native builds for `x86_64-linux` and `aarch64-linux` (including a headless server variant).

---

## Documentation

Comprehensive guides are available in the [`docs/`](docs/) directory:

- [**System Maintenance and Btrfs Optimization**](docs/maintenance.md) — Detailed cleanup stages, ioctl block-group compaction, allocation targets, and queue persistence.
- [**Web Interface and Remote Access**](docs/remote-access.md) — Browser panel usage, SSH tunneling, LAN/wildcard bindings, remote GTK client, and token security.
- [**Legacy (Non-Flake) Configurations**](docs/legacy-configurations.md) — Environment detection, legacy action matrix, and channel management.
- [**Multi-host Management and Shared Cache**](docs/peering.md) — Pairing hosts, peer credentials, and configuring signed binary caches.
- [**Development and Verification Guide**](docs/development.md) — Build workflows, automated test suites, disposable VM tests, and packaging notes.

---

## Install on NixOS

Add Yonix to your flake inputs:

```nix
inputs.yonix.url = "github:REALalphas/yonix-up";
inputs.yonix.inputs.nixpkgs.follows = "nixpkgs";
```

Import the NixOS module and configure your instance:

```nix
{ inputs, ... }:
{
  imports = [ inputs.yonix.nixosModules.default ];

  programs.yonix = {
    enable = true;
    user = "operator";             # replace with your local user
    configPath = "/home/operator/nixos";
    allowSystemChanges = false;    # enable only after reviewing the trust model below
    keepGenerations = "14d";
    # host defaults to networking.hostName
    # sourceMode = "git";          # or "path" to include untracked/ignored files
  };
}
```

For headless servers without GTK dependencies:

```nix
programs.yonix.package = inputs.yonix.packages.${pkgs.stdenv.hostPlatform.system}.yonix-headless;
```

Apply this configuration with your usual rebuild command. Yonix will appear in your desktop application menu, as the `yonix` CLI command, and as a systemd service running at boot.

> [!IMPORTANT]
> **Trust Model**: `allowSystemChanges` defaults to `false`. Enabling it authorizes the configured user and API token holder to execute three fixed, privileged helper commands: `switch`, `rollback`, and `clean`. A user who can activate arbitrary NixOS code effectively holds root privileges. The helper clears its environment and refuses arbitrary paths or commands.

For non-flake setups using `configuration.nix`, see [Legacy Configurations](docs/legacy-configurations.md).

---

## Desktop and Terminal

Run commands via the `yonix` CLI or launch the native desktop GUI:

```console
yonix                    # Launch native libadwaita desktop window
yonix status             # Check system status, generation, and config kind
yonix check              # Dry-run evaluation (no build or activation)
yonix build              # Build configuration into a state GC root
yonix diff               # Build and compare store closures with current system
yonix update-inputs      # Update flake.lock with interactive confirmation
yonix switch             # Build and activate configuration (alias: yonix update)
yonix rollback           # Revert to the previous NixOS generation
yonix clean              # Full system maintenance (Btrfs, Nix GC, verify, fstrim)
yonix history            # Display history of executed operations
yonix logs <JOB_ID>      # View execution log for a specific job
yonix web                # Open the browser panel in your default browser
yonix token              # Display the private gateway authentication token
```

### Operation Model

- **Concurrency serialization**: Operations are serialized by the server; conflicting concurrent actions are rejected.
- **Resilience**: Closing a client does not interrupt running tasks. When a rebuild updates the Yonix package itself, the daemon remains active until the job finishes.
- **Detailed logs**: Command output and stderr are recorded in persistent logs inspectable via `yonix logs` or the web panel.

---

## System Maintenance

Yonix provides automated maintenance without risky blind deletions:

- **Pre-switch maintenance**: Every `switch` checks Btrfs allocation reserves and compacts sparse data block groups when needed.
- **Explicit `yonix clean`**: Performs a logged sequence including tmpfiles cleanup (`/tmp`, `/var/tmp`, `/var/cache`, coredumps), journal vacuuming, Nix garbage collection, store hash verification, hard-link deduplication, Btrfs compaction, and filesystem TRIM.

Home directories, browser caches, containers, and active GC roots are never removed. For full details on the Btrfs ioctl allocator and cleanup policies, see [System Maintenance and Btrfs Optimization](docs/maintenance.md).

---

## Web Interface and Remote Access

The web daemon listens locally at **`http://127.0.0.1:9847`** by default.

1. Open `http://127.0.0.1:9847` in your browser.
2. Sign in with the bearer token from `yonix token`.

### Remote Access via SSH Tunnel

For remote machines, forward the port securely over SSH:

```console
ssh -N -L 9847:127.0.0.1:9847 operator@server
```

Then open `http://127.0.0.1:9847` locally and paste the token from `ssh operator@server yonix token`.

For direct private LAN access (`programs.yonix.listenAddress`), wildcard bindings (`0.0.0.0`), and remote GTK client setups, see [Web Interface and Remote Access](docs/remote-access.md).

---

## Multiple Hosts and Shared Cache

Yonix allows managing multiple hosts through a single web panel:

- Pair hosts dynamically via one-time pairing links or 6-digit codes under **Connections**.
- Declare persistent peers in Nix with private runtime token files.
- Enable signed Nix HTTP binary caches (`programs.yonix.cache`) to automatically share and reuse build artifacts across hosts.

For configuration examples and security recommendations, see [Multi-host Management and Shared Cache](docs/peering.md).

---

## Development and Verification

Enter the development shell and run standard quality checks:

```console
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
npm ci && npm run build && npm test
```

### Reproducible Nix Checks

```console
nix build                               # Native desktop package
nix build .#yonix-headless              # Headless server package
nix build .#checks.x86_64-linux.module  # NixOS module evaluation check
nix build .#checks.x86_64-linux.clean   # Cleanup & policy isolation tests
nix build .#checks.x86_64-linux.btrfs   # Disposable Btrfs VM test
nix build .#checks.x86_64-linux.vm      # Disposable NixOS service & activation VM
nix build .#checks.x86_64-linux.peers   # Disposable multi-host & signed cache VM
```

For headless GTK test instructions under Xvfb, manual uninstalled execution, and packaging architecture, see the [Development and Verification Guide](docs/development.md).

---

## Layout

- `src/engine.rs` — Command execution plans (flake and legacy), job serialization, logs, and state.
- `src/server.rs` — Local/LAN HTTP API, token authentication, and embedded web assets.
- `src/client.rs` — Shared API client for CLI and desktop clients.
- `src/gui.rs` — Native GTK 4/libadwaita interface.
- `frontend/` — React web client using bundled Alpha UI release archives.
- `web/` — Compiled production frontend assets embedded into the daemon.
- `locales/` — Translation files across 13 supported languages.
- `nix/` — Package derivations, NixOS modules, and disposable VM tests.
- `docs/` — Deep-dive documentation and architecture guides.

---

## License and Project Support

Yonix application code is licensed under the [GNU Affero General Public License, version 3 or later](LICENSE).

The web panel bundles the official Alpha UI release under separate AHCL-1.0 terms (not an open-source license). The corresponding source archive is included in `vendor/alpha-ui/`; review [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) before redistributing binaries.

- Report issues and feature requests at [GitHub Issues](https://github.com/REALalphas/yonix-up/issues).
- Before submitting patches or reporting vulnerabilities, please consult [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).
