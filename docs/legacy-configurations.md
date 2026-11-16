# Legacy (Non-Flake) Configurations

Yonix supports both modern Nix flakes and traditional channel-based (`configuration.nix`) NixOS configurations using the same unified Rust daemon, CLI, and interfaces.

## Environment Detection

Yonix automatically detects the environment type from `configPath` at startup and runtime:

- If `flake.nix` is present in `configPath`, the environment is **`flake`** (even if `configuration.nix` also exists).
- If `configuration.nix` is present without `flake.nix`, the environment is **`legacy`**.
- If neither file exists, the environment is **`unknown`**.

The detected type is displayed as a badge in the GTK window and browser header, and reported as `config_kind` by `yonix status`. No manual configuration flags or service restarts are required.

---

## Action Matrix

| Operation | Flake Mode | Legacy Mode |
|---|---|---|
| **Check** | `nix eval …drvPath` | `nixos-rebuild dry-build --no-flake -I nixos-config=…/configuration.nix` |
| **Build** | `nix build … --out-link <state>/result` | `nixos-rebuild build …` (stores `result` symlink in `<state>/result` as a GC root) |
| **Diff** | Build + `nix store diff-closures` | Build + `nix store diff-closures` |
| **Update inputs** | `nix flake update` | **Refused**: Legacy configurations do not use `flake.lock`. Update channels externally with `nix-channel --update`. |
| **Switch** | Fixed privileged helper (`nixos-rebuild switch --flake …`) | Fixed privileged helper (`nixos-rebuild switch --no-flake -I nixos-config=…`) |
| **Rollback** | Fixed privileged helper (`nixos-rebuild switch --rollback`) | Identical: Rollback operates on NixOS generations independent of configuration source. |
| **Clean** | Full maintenance and Nix GC | Identical: System cleanup and garbage collection operate on the Nix store independent of configuration source. |

---

## Handling of `unknown` Environments

When neither `flake.nix` nor `configuration.nix` is found at `configPath`:

- Build, check, diff, update-inputs, and switch operations are refused.
- **Rollback** and **clean** operations remain permitted so an administrator can recover the system or perform maintenance even if the configuration source repository is unavailable or misplaced.

---

## Configuration Nuances

- **`sourceMode`**: The option `programs.yonix.sourceMode` (`"git"` or `"path"`) applies solely to flake evaluation and is ignored for legacy configurations.
- **Channels**: Ensure that NIX_PATH or required channel definitions are configured for root and the executing user if legacy builds depend on system channels.
