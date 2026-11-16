# Yonix

Yonix manages NixOS configurations from a native GTK 4/libadwaita desktop app, the command line, or a browser panel. Each client uses the same Rust service.

**Experimental, version 0.1.0.** Yonix can activate NixOS configurations and remove old generations. Review the operation and retention settings before enabling system changes. Compatibility is not guaranteed, and no release support schedule is established.

- Native GTK 4/libadwaita desktop, CLI and browser panel with 13 interface languages.
- Flake and legacy configurations; check, build, compare, activate and roll back.
- Explicit system maintenance, multi-host management and signed build caches.
- Reproducible Nix packages for `x86_64-linux` and `aarch64-linux`; CI targets `x86_64-linux`.

## Contents

- [Install on NixOS](#install-on-nixos)
  - [Legacy configurations](#legacy-non-flake-configurations)
- [Desktop and terminal](#desktop-and-terminal)
- [Browser and remote access](#browser-and-remote-access)
- [Multiple hosts and shared cache](#multiple-hosts-and-shared-cache)
- [Development and verification](#development-and-verification)
- [Project layout](#layout)
- [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Changes](CHANGELOG.md) · [License and support](#license-and-project-support)

Yonix application code is licensed under **AGPL-3.0-or-later**. The browser app includes the official Alpha UI under separate AHCL-1.0 terms, which are not an open-source license. The matching official source archive is included; review the [third-party notices](THIRD_PARTY_NOTICES.md) before redistributing builds.

## Install on NixOS

Add Yonix to the `inputs` in your NixOS flake:

```nix
inputs.yonix.url = "github:REALalphas/yonix-up";
inputs.yonix.inputs.nixpkgs.follows = "nixpkgs";
```

Include its module in your `nixosSystem` modules and configure it:

```nix
{ inputs, ... }:
{
  imports = [ inputs.yonix.nixosModules.default ];

  programs.yonix = {
    enable = true;
    user = "operator"; # replace with your existing local user
    configPath = "/home/operator/nixos";
    # host defaults to networking.hostName
    # sourceMode = "path"; # include ignored/untracked files; see below
    allowSystemChanges = false; # enable only after reviewing the trust model below
    keepGenerations = "14d";
  };
}
```

Replace `operator` and `configPath` with your existing user and configuration checkout. Commit `flake.lock` to pin the selected revision.

`inputs` must be passed through `specialArgs`, or put the module directly in the flake's `modules` list. Apply this configuration once with your existing rebuild workflow. Yonix then appears in the application menu and as `yonix` in the terminal. Its service starts at boot, including on machines without a graphical login.

`allowSystemChanges` defaults to `false`. Enabling it authorizes the configured user and API token holder to administer the system through three fixed helper commands: switch, rollback and clean. A configuration owner who can activate NixOS code effectively has root privileges. The helper clears its environment and does not accept arbitrary commands, paths or flags from API clients.

The configured user must be able to read the checkout, and write it to update `flake.lock`. Private Git inputs need credentials accessible to the relevant process. Activation runs the fixed rebuild helper as root; root may need its own access to private inputs.

`sourceMode = "git"` is the default: Nix sees files tracked by Git. Use `"path"` when the configuration depends on ignored or untracked files. Path mode can put local source files into the world-readable Nix store, so keep secrets outside the checkout. It does not stage files or change Git ignore rules. `sourceMode` applies to flake mode only and is ignored for legacy configurations.

## Legacy (non-flake) configurations

Yonix detects the environment type from `configPath` at runtime: `flake.nix` present means **flake** (it wins when both files exist), otherwise `configuration.nix` means **legacy**, otherwise **unknown**. No configuration change or restart flag is needed; the same package serves both modes. The detected type is shown as a badge in the desktop window and the browser panel (`yonix status` reports it as `config_kind`).

| Action | Flake | Legacy |
|---|---|---|
| Check | `nix eval …drvPath` | `nixos-rebuild dry-build --no-flake -I nixos-config=…/configuration.nix` |
| Build | `nix build … --out-link <state>/result` | `nixos-rebuild build …` (the `result` symlink lands in the state directory, same GC root) |
| Diff | build + `nix store diff-closures` | same shape, over the legacy build |
| Update inputs | `nix flake update` | refused: legacy configurations have no `flake.lock`; manage channels outside Yonix |
| Switch / Rollback / Clean | fixed sudo helper | same helper; switch uses `--no-flake -I nixos-config=…`, rollback and clean are config-independent |

`unknown` environments (neither file present) allow only rollback and clean, like a missing `flake.nix` before.

On a server without GTK:

```nix
programs.yonix.package = inputs.yonix.packages.${pkgs.stdenv.hostPlatform.system}.yonix-headless;
```

The module installs the package, `/etc/yonix/config.json`, the systemd service and optional sudo rules. It opens a firewall port only with `openFirewall = true`. It does not copy your checkout into `/etc/nixos`, change hostname, pull Git changes or schedule cleanup.

## Desktop and terminal

```console
yonix                    # native libadwaita window
yonix status
yonix check              # evaluate the system derivation; no build or activation
yonix build              # build only, retained by a GC root in the state directory
yonix diff               # build and run nix store diff-closures
yonix update-inputs      # update flake.lock only, with confirmation
yonix switch             # build and activate, with confirmation
yonix rollback           # activate the previous system generation
yonix clean              # system checks, expired files/journals, Nix GC, deduplication, Btrfs and TRIM
yonix history
yonix logs JOB_ID
yonix web                # open the browser panel
yonix token              # print the private token for browser sign-in
```

`yonix update` is an alias for `switch`. Other old shortcuts and destructive flags are deliberately not carried over. `check` evaluates the selected system (flake derivation path, or legacy dry-build); it is not a replacement for all checks in `nix flake check`. `diff` compares package closures, not service configuration.

Every `switch`/`update` runs Btrfs maintenance before rebuilding. Only explicit `clean` runs `nix-collect-garbage --delete-older-than`, using `programs.yonix.keepGenerations` (default `14d`), as part of the system cleanup below. Neither path traverses home directories, manually removes GC roots, changes data/metadata profiles, or runs an unfiltered full balance. Used metadata is never relocated; metadata DUP is preserved. Empty data and metadata chunks are reclaimed first. Writable mounted filesystems are deduplicated by UUID; read-only mounts and other filesystem types are skipped.

Explicit `clean` runs one logged sequence:

- Reports local disk and inode usage and the read-only Btrfs allocation baseline.
- Runs `systemd-tmpfiles --clean` restricted to `/tmp`, `/var/tmp`, `/var/cache` and `/var/lib/systemd/coredump`. Only entries eligible under the machine's existing tmpfiles age/exclusion/locking rules are cleaned; paths without a cleanup rule are left alone. No `--remove`, forced recursive deletion, or user tmpfiles pass is used. Administrators remain responsible for custom rules within these prefixes; old temporary files may belong to users.
- Rotates journals, then vacuums archives with `programs.yonix.cleanup.journalMaxAge` (default `14d`) and `journalMaxSizeMiB` (default `512`). This removes diagnostic history, including archived user journals; active files are outside the vacuum limits. Rotation failure skips vacuum. Journal disk usage is reported afterward.
- Removes old Nix generations and unreachable store paths, retaining existing GC roots, including Yonix's build result.
- Verifies hashes of all remaining Nix store paths with `nix store verify --all --no-trust`, without repair, downloads or signature checks. `cleanup.verifyStore` defaults to `true`; a large store can take substantial time to read. Corruption or read errors fail the check and skip optimisation.
- Deduplicates identical Nix store files with `nix-store --optimise` (`cleanup.optimiseStore`, default `true`), then runs the existing bounded Btrfs maintenance.
- Runs `fstrim --fstab --verbose --quiet-unsupported` (`cleanup.trim`, default `true`). Only mounted filesystems configured in `/etc/fstab` (plus the root filesystem) are considered; unsupported filesystems are skipped and `X-fstrim.notrim` exclusions are honored. Reported discard bytes are not newly reclaimed file space.
- Reports failed system services and final local disk/inode usage. Failed services make the check fail; they are never reset, restarted or disabled automatically.

Each stage prints its command and result. Independent stages continue after a failure; the final completed/failed/skipped summary and nonzero exit status expose partial cleanup. No browser data, trash, home-directory caches, containers, images, volumes, backups, application databases, arbitrary logs or manual GC roots are pruned. Cache and crash-dump cleanup follows installed systemd rules rather than assuming that every file under a cache directory is disposable. The cleanup settings above are NixOS module options under `programs.yonix.cleanup`; no flags or filesystem paths are accepted from API clients. No cleanup timer or daemon-startup cleanup is added.

Maintenance reads actual block groups through the read-only Btrfs tree-search ioctl, supporting both the legacy extent tree and dedicated block-group tree. It queues non-empty data-only groups below 90% usage, sparsest first. Each relocation targets exactly one logical range with `vrange`, `usage=90`, and `limit=1`. `clean` attempts at most 24 data groups per filesystem per invocation, **continuing beyond the allocation reserve**. A healthy pre-switch pass attempts at most one; a low-reserve pre-switch pass can attempt up to 24 and stops once the reserve is restored. These are chunk limits, not wall-clock deadlines: a large chunk can still take time.

A root-owned queue in `/var/lib/yonix-btrfs-maintenance/<UUID>.json` survives subsequent invocations and package updates. It is separate from the daemon's user-owned state. Subsequent passes continue the queue instead of starting again at empty usage filters. GC or ordinary writes between invocations do not reset an unfinished queue and starve its tail; they require a fresh sweep after the current queue finishes. After a sweep with measured allocation gains, a new sweep can consolidate the changed layout. After a whole sweep without gains, the resulting layout is remembered and further data relocations are skipped until data group allocation/usage or available device workspace changes (for example after GC, file deletion or external recovery). A relocation that consumes allocation workspace stops the invocation immediately, preserving unvisited groups for the next run. A completed sweep repeats only if its net allocation delta was positive. State is only bookkeeping; removing it forces a rescan and does not delete filesystem content. Malformed/unreadable state fails visibly. A separate nonblocking lock serializes direct maintenance invocations too.

Clean prints a read-only allocation baseline before GC, then the native Nix GC result (paths/bytes freed). The Btrfs summary starts after file cleanup and store optimisation so its delta is not attributed to those steps. Each filesystem reports unallocated bytes before/after, their signed delta, attempted data groups and whether work remains, no safe candidates remain, an unchanged layout was already exhausted, or workspace decreased. Exit zero and the number of relocated chunks are **not** evidence of reclaimed space. Replacing a source chunk with an equally large destination yields no allocation progress. Concurrent writes can affect the measured delta. Completion means the bounded safe policy has no further verified work for the observed layout; it does not promise an empty disk or an optimal packing. Groups at or above 90%, mixed data/metadata groups and used metadata are outside that policy. Some free space inside chunks must remain; Btrfs can refuse or fail to compact a group, and a clean filesystem may yield zero new free bytes on subsequent runs.

`programs.yonix.btrfs.reserveGiB` defaults to 8 (range 1–128). The checked target is capped at 25% of device size and doubled under metadata pressure. Pressure includes metadata `bytes_reserved`, `bytes_pinned`, `bytes_may_use` (including global reserve) and `bytes_readonly`, rather than only metadata used. This is a target, not a hard reservation; file-level free space does not provide allocation workspace. A low reserve after the bounded pass, failed balance, or unreadable allocation/counter/ioctl information causes failure, even when some work succeeded. Other filesystems are still attempted. Pre-switch failure prevents rebuilding; clean marks the Btrfs stage failed and continues independent checks before returning failure. On relocation `ENOSPC`, inspect `btrfs filesystem usage --raw <mount>` and `journalctl -k -b`; restore allocation workspace before retrying. Raising the balance limit or deleting a small file may not help. No rescue device, timer, installer or background cleanup is added.

The last `<state_dir>/result` symlink remains a GC root for the most recent build. Generation age is a time-based rollback policy, not a guarantee of a minimum number of generations. “Clean” preserves active data, reachable store paths and the configured rollback policy; it does not remove every generation.

The desktop and browser interfaces support English, Russian, German, French, Spanish, Portuguese (Brazil), Italian, Ukrainian, Polish, Turkish, Simplified Chinese, Japanese and Korean. The desktop follows the system's preferred languages, defaulting to English if none are supported. The browser follows its preferred languages by default and offers a selector in settings; its choice is saved in local storage for that origin. CLI output, service errors and command logs retain their original language.

Operations are serialized by the server. A second operation receives an error while one is active. Exit failures are recorded as failures, with stdout and stderr in a persistent log. Closing a client does not cancel a job. There is no cancellation UI yet. When a rebuild updates Yonix itself, its daemon keeps running until an explicit service restart or reboot, so activation is not killed midway. Jobs marked interrupted after a server restart require inspection before retrying; Yonix does not assume activation completed successfully.

## Browser and remote access

Yonix does not provide built-in TLS or per-user accounts. Use the SSH tunnel below for connections over untrusted networks.

The service defaults to **`127.0.0.1:9847`**. Locally, open `http://127.0.0.1:9847` and sign in with `yonix token`. The token file and state directory are accessible only to the configured user and root. The browser saves a successfully verified gateway token in local storage for this origin and reconnects after reload. Disconnect in Maintenance removes the saved token; a rejected token is also cleared. Peer tokens remain in private server files and are never returned to the browser. Scripts, fonts and other assets are served locally; the panel does not load third-party network resources.

For remote access, on your client computer:

```console
ssh -N -L 9847:127.0.0.1:9847 operator@server
```

Open `http://127.0.0.1:9847` on that computer. Get the token from a separate SSH terminal (`ssh operator@server yonix token`) and paste it into the sign-in form. Use another local port if 9847 is occupied. SSH provides encryption and host authentication.

For direct access on a trusted local network, explicitly select the host's private IPv4 address:

```nix
programs.yonix.listenAddress = "10.0.0.10";
programs.yonix.openFirewall = true;
```

Open `http://10.0.0.10:9847`. Token authentication and same-origin/Host checks remain enabled. This mode uses unencrypted HTTP, so use SSH for untrusted networks. For a standalone JSON configuration, set `listen_address` to the same IPv4 address.

To listen on all IPv4 interfaces, set `programs.yonix.listenAddress = "0.0.0.0"` (JSON: `"listen_address": "0.0.0.0"`). A concrete public IPv4 address is also accepted. No additional permission option is required; remove the old `allowPublicBind` / `allow_public_bind` setting from existing configurations. Open the panel using the server's numeric interface address. With a wildcard bind, the CLI, desktop and `yonix web` use `127.0.0.1` locally. Host checks accept numeric IPv4 addresses and `localhost`; token authentication and same-origin checks still apply.

A native client can use the same tunnel:

```console
yonix --config remote.json \
  --url http://127.0.0.1:9847 \
  --token-file /path/to/private/remote.token gui
```

Generate a local configuration example with `yonix init > remote.json`. Protect a copied token with mode `0600`. Never add it to a Nix expression or the Git repository. To revoke a token, stop the server, remove its token file and restart it; existing clients must sign in with the new token.

## Multiple hosts and shared cache

Use the managed-host selector to operate another Yonix host through the current panel. Add hosts under **Connections**, or declare peers with private runtime token files, and enable signed Nix caches for automatic reuse of matching builds. See [setup and trust model](docs/peering.md) for nas/web examples. Configuration synchronization and activation remain manual.

## Development and verification

GitHub Actions runs `.github/workflows/ci.yml` on pushes and pull requests, and
can also be started manually with `workflow_dispatch`. It checks frontend
type safety, production builds and browser tests; Rust formatting, Clippy and
tests with and without GTK; and the display-dependent GTK test under Xvfb.
Separate Nix jobs build both packages, validate the NixOS module and run the
cleanup tests. The `vm`, `btrfs` and `peers` checks run in disposable NixOS VMs
on KVM-enabled Ubuntu runners. Actions have read-only repository permissions;
no deployment or repository secrets are required. A repository hygiene job validates workflows and scans the full Git history with Gitleaks (redacted output). CI uses the pinned
`flake.lock`, `Cargo.lock` and `package-lock.json` dependencies.

```console
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
npm ci && npm run build && npm test # frontend and 13-language checks
nix build                         # desktop package, including Rust tests
nix build .#yonix-headless
nix build .#checks.x86_64-linux.module
nix build .#checks.x86_64-linux.clean # policy/ioctl fixtures, cleanup isolation/failure tests, wrappers and shellcheck; no host maintenance
nix build .#checks.x86_64-linux.btrfs # disposable Btrfs VM: both block-group trees, convergence, data checksums
nix build .#checks.x86_64-linux.vm # isolated NixOS service/build/activation/rollback/permissions test
nix build .#checks.x86_64-linux.peers # isolated two-host peering and signed-cache test
```

Core and API tests can also run with `cargo test --no-default-features`. End-to-end tests run the real daemon and CLI with fake Nix executables to exercise success, exit failures, concurrent requests, auth, origin/Host validation and restart persistence without modifying the host system. A display-dependent GTK test is ignored by the default suite; run it in an isolated session:

```console
nix shell nixpkgs#xvfb-run nixpkgs#dbus --command \
  xvfb-run -a dbus-run-session --config-file=tests/dbus-session.conf -- \
  env GDK_BACKEND=x11 GTK_A11Y=none \
  cargo test --lib native_window_renders_and_connects -- --ignored --test-threads=1
```

For a manual uninstalled run, create a JSON configuration with `yonix init`, edit its absolute checkout/state paths and host, and run `yonix --config config.json serve`. In another terminal run `yonix --config config.json gui`. System-changing actions are unavailable without the installed NixOS helper.

Use `yonix-headless` to avoid GTK/libadwaita on servers. Release builds use Cargo's default parallel code generation without LTO to reduce build time. The Nix frontend build includes only the Alpha UI archives referenced by the npm lock file and prepares its offline cache in one npm invocation. Generated `web/` files and frontend-only tests are excluded from the Rust source derivation; production assets are still rebuilt in Nix and embedded into the immutable package.

## Layout

- `src/engine.rs`: command plans (flake and legacy), serialized jobs, logs and persistence.
- `src/server.rs`: local/LAN API, authentication and embedded web assets.
- `src/client.rs`: shared CLI/desktop client.
- `src/gui.rs`: native libadwaita interface; networking stays off the GTK thread.
- `frontend/`: React client using the official Alpha UI 0.4.11 release archives bundled in `vendor/alpha-ui/`.
- `web/`: generated assets embedded into the Rust binary; rebuild with `npm run build`.
- `vendor/alpha-ui/`: checksummed core/React release archives for reproducible Nix builds.
- `locales/`: shared JSON translations for the desktop and browser.
- `nix/`: package, NixOS module and isolated VM test.

To add a language, create a JSON file in `locales/` with the same keys as `en.json`, register it in `src/i18n.rs`, and add its display name to `frontend/i18n.ts`. The Rust and browser tests check the registered files, keys and selector options.

## License and project support

Copyright (C) 2026 Yonix contributors. Yonix application code is available under the [GNU Affero General Public License, version 3 or later](LICENSE), without warranty. Third-party packages and assets retain their own licenses; the AGPL grant does not relicense Alpha UI. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Report ordinary bugs and feature requests through [GitHub Issues](https://github.com/REALalphas/yonix-up/issues). Read [SECURITY.md](SECURITY.md) before reporting a possible vulnerability, and [CONTRIBUTING.md](CONTRIBUTING.md) before submitting changes. Support is provided on a best-effort basis.
