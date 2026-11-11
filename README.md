# Yonix

A NixOS companion written in Rust: native GTK 4/libadwaita application, CLI and a browser panel backed by the same operation service.

This is the first working version, not a release with an established compatibility guarantee. The original Bash implementation is preserved in `old/` and is not installed by the new package.

## Install on NixOS

Add the flake as an input. While developing locally:

```nix
inputs.yonix.url = "git+file:///home/alphas/Documents/Projects/_sysadmin/nix/yonix-up";
inputs.yonix.inputs.nixpkgs.follows = "nixpkgs";
```

Include its module in your `nixosSystem` modules and configure it:

```nix
{ inputs, ... }:
{
  imports = [ inputs.yonix.nixosModules.default ];

  programs.yonix = {
    enable = true;
    user = "alphas";
    configPath = "/home/alphas/Documents/Projects/_sysadmin/nix/nix-new";
    host = "nas"; # defaults to networking.hostName
    sourceMode = "path"; # include local config.nix even when Git ignores it
    allowSystemChanges = true;
    keepGenerations = "14d";
  };
}
```

`inputs` must be passed through `specialArgs`, or put the module directly in the flake's `modules` list. Apply this configuration once with your existing rebuild workflow. Yonix then appears in the application menu and as `yonix` in the terminal. Its service starts at boot, including on machines without a graphical login.

`allowSystemChanges` defaults to `false`. Enabling it authorizes the configured user and API token holder to administer the system through three fixed helper commands: switch, rollback and clean. A configuration owner who can activate NixOS code effectively has root privileges. The helper clears its environment and does not accept arbitrary commands, paths or flags from API clients.

The configured user must be able to read the checkout, and write it to update `flake.lock`. Private Git inputs need credentials accessible to the relevant process. Activation runs the fixed rebuild helper as root; root may need its own access to private inputs.

`sourceMode = "git"` is the default: Nix sees files tracked by Git. Use `"path"` for a local checkout with ignored or untracked configuration files, as in the original yonix workflow. Path mode can put local source files into the world-readable Nix store, so keep secrets outside the checkout. It does not stage files or change Git ignore rules. `sourceMode` applies to flake mode only and is ignored for legacy configurations.

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
yonix clean              # delete old generations, then garbage collect
yonix history
yonix logs JOB_ID
yonix web                # open the browser panel
yonix token              # print the private token for browser sign-in
```

`yonix update` is an alias for `switch`. Other old shortcuts and destructive flags are deliberately not carried over. `check` evaluates the selected system (flake derivation path, or legacy dry-build); it is not a replacement for all checks in `nix flake check`. `diff` compares package closures, not service configuration. Every `yonix switch` (including its `update` alias) runs light Btrfs maintenance before building: reclaim empty data/metadata chunks, then balance at most one data chunk below 5% usage per writable mounted filesystem, deduped by UUID. `yonix clean` runs `nix-collect-garbage --delete-older-than` first, then the same maintenance with up to two data and two metadata chunks below 5% usage. Neither path runs an unfiltered full balance.

Both paths check actual unallocated device space, targeting the smaller of 2 GiB or 25% of device size. If reclaiming empty chunks is insufficient, they automatically compact up to two data chunks per pass with usage thresholds of 5%, 10%, 25%, 50%, 75% and 90%, checking the reserve after every pass. Productive thresholds repeat until the target is reached, with at most twelve recovery passes (24 data chunks) per filesystem. Data relocation precedes used-metadata relocation to make allocation space available. File-level free space alone does not provide this reserve. Btrfs can still refuse relocation with `ENOSPC`; failed commands, unreadable allocation information or a reserve that remains below target cause a clear failure. Other filesystems are still attempted. A failed pre-switch maintenance prevents the rebuild; a failed clean reports partial cleanup, keeping any space already freed by GC. Read-only Btrfs mounts and other filesystem types are skipped. The switch path does not delete generations or run GC. Cleanup does not delete `/tmp`, journal files or GC roots manually. The last `<state_dir>/result` symlink is intentionally kept as a GC root for the most recent build. Generation age is a time-based policy, not a guarantee of a minimum number of rollback generations.

The desktop and browser interfaces support English, Russian, German, French, Spanish, Portuguese (Brazil), Italian, Ukrainian, Polish, Turkish, Simplified Chinese, Japanese and Korean. The desktop follows the system's preferred languages, defaulting to English if none are supported. The browser follows its preferred languages by default and offers a selector in settings; its choice is saved in local storage for that origin. CLI output, service errors and command logs retain their original language.

Operations are serialized by the server. A second operation receives an error while one is active. Exit failures are recorded as failures, with stdout and stderr in a persistent log. Closing a client does not cancel a job. There is no cancellation UI yet. When a rebuild updates Yonix itself, its daemon keeps running until an explicit service restart or reboot, so activation is not killed midway. Jobs marked interrupted after a server restart require inspection before retrying; Yonix does not assume activation completed successfully.

## Browser and remote access

The service defaults to **`127.0.0.1:9847`**. Locally, open `http://127.0.0.1:9847` and sign in with `yonix token`. The token file and state directory are accessible only to the configured user and root. The browser saves a successfully verified gateway token in local storage for this origin and reconnects after reload. Disconnect in Maintenance removes the saved token; a rejected token is also cleared. Peer tokens remain in private server files and are never returned to the browser. No third-party scripts, fonts or network services are used.

For remote access, on your client computer:

```console
ssh -N -L 9847:127.0.0.1:9847 alphas@nas
```

Open `http://127.0.0.1:9847` on that computer. Get the token from a separate SSH terminal (`ssh alphas@nas yonix token`) and paste it into the sign-in form. Use another local port if 9847 is occupied. SSH provides encryption and host authentication.

For direct access on a trusted local network, explicitly select the host's private IPv4 address:

```nix
programs.yonix.listenAddress = "10.0.2.142";
programs.yonix.openFirewall = true;
```

Open `http://10.0.2.142:9847`. Token authentication and same-origin/Host checks remain enabled. This mode uses unencrypted HTTP, so use SSH for untrusted networks. For a standalone JSON configuration, set `listen_address` to the same IPv4 address.

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

```console
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
npm ci && npm run build && npm test # frontend and 13-language checks
nix build                         # desktop package, including Rust tests
nix build .#yonix-headless
nix build .#checks.x86_64-linux.module
nix build .#checks.x86_64-linux.vm # isolated NixOS service/build/activation/rollback/permissions test
```

Core and API tests can also run with `cargo test --no-default-features`. End-to-end tests run the real daemon and CLI with fake Nix executables to exercise success, exit failures, concurrent requests, auth, origin/Host validation and restart persistence without modifying the host system. A display-dependent GTK test is ignored by the default suite; run it in an isolated session:

```console
nix shell nixpkgs#xvfb-run nixpkgs#dbus --command \
  xvfb-run -a dbus-run-session -- env GDK_BACKEND=x11 GTK_A11Y=none \
  cargo test --lib native_window_renders_and_connects -- --ignored --test-threads=1
```

For a manual uninstalled run, create a JSON configuration with `yonix init`, edit its absolute checkout/state paths and host, and run `yonix --config config.json serve`. In another terminal run `yonix --config config.json gui`. System-changing actions are unavailable without the installed NixOS helper.

Use `yonix-headless` to avoid GTK/libadwaita on servers. Release builds use Cargo's default parallel code generation without LTO to reduce build time. The Nix frontend build includes only the Alpha UI archives referenced by the npm lock file and prepares its offline cache in one npm invocation. Generated `web/` files and frontend-only tests are excluded from the Rust source derivation; production assets are still rebuilt in Nix and embedded into the immutable package.

## Layout

- `src/engine.rs`: command plans (flake and legacy), serialized jobs, logs and persistence.
- `src/server.rs`: local/LAN API, authentication and embedded web assets.
- `src/client.rs`: shared CLI/desktop client.
- `src/gui.rs`: native libadwaita interface; networking stays off the GTK thread.
- `frontend/`: React client using the official Alpha UI 0.4.11 release from alphas-base.
- `web/`: generated assets embedded into the Rust binary; rebuild with `npm run build`.
- `vendor/alpha-ui/`: checksummed core/React release archives for reproducible Nix builds.
- `locales/`: shared JSON translations for the desktop and browser.
- `nix/`: package, NixOS module and isolated VM test.

To add a language, create a JSON file in `locales/` with the same keys as `en.json`, register it in `src/i18n.rs`, and add its display name to `frontend/i18n.ts`. The Rust and browser tests check the registered files, keys and selector options.

Next work: generation browsing, service-level diff, cancellation semantics, richer progress reporting and accessibility review. Built-in TLS and multi-user permissions are outside this first version.
