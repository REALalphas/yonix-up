# Yonix — archived Bash implementation

Historical reference only; not maintained or installed by the current package. Use the [current installation instructions](../README.md#install-on-nixos). The commands below describe the original script.

Yonix is a wrapper for `nixos-rebuild` and `nix flake` that automates privilege elevation, directory tracking, and system maintenance.

## Installation

```bash
git clone https://github.com/REALalphas/yonix-up.git
./yonix-up/old/yonix
```

The script will self-install to `~/.local/bin/yonix` on first run.

## Usage

```bash
yonix <command> [options]
```

### Commands

| Command | Short | Description |
|---------|-------|-------------|
| `update` | `u` | Run `nixos-rebuild switch` |
| `upgrade` | `U` | Update flake inputs and rebuild |
| `clean` | `c` | Garbage collect, optimize store, vacuum logs |
| `trace` | `t` | Rebuild with `--show-trace` |
| `build` | `b` | Build new system (sets hostname) |
| `dir` | `d` | Show current config directory |
| `git-check` | `g` | Check/pull git repository updates |

### Configuration

Yonix tracks your NixOS configuration directory in `~/.config/yonix/nix-dir`.
Running any command within a directory containing `flake.nix` updates this pointer.
