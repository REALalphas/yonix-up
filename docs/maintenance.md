# System Maintenance and Btrfs Optimization

Yonix provides automated, bounded system maintenance through the `yonix clean` command and pre-switch maintenance routines. Maintenance preserves active data, reachable store paths, and the configured generation retention policy while reclaiming space safely.

## Operational Overview

- **Pre-switch maintenance**: Every `yonix switch` (or `yonix update`) performs a bounded Btrfs check and compaction before rebuilding and activating a new configuration.
- **Explicit cleanup (`yonix clean`)**: Runs a full, logged sequence of system checks, journal vacuuming, Nix garbage collection, store verification and deduplication, Btrfs compaction, and filesystem TRIM.

Neither path traverses user home directories, manually deletes user files, modifies Btrfs data/metadata RAID profiles, or runs an unfiltered full filesystem balance.

---

## The `yonix clean` Sequence

Explicit `clean` runs the following sequential stages, logging each command and result:

1. **Baseline usage**: Reports local disk and inode usage along with the read-only Btrfs allocation baseline.
2. **Temporary files (`systemd-tmpfiles`)**: Runs `systemd-tmpfiles --clean` restricted strictly to `/tmp`, `/var/tmp`, `/var/cache`, and `/var/lib/systemd/coredump`.
   - Only entries eligible under installed age, exclusion, and locking rules are cleaned.
   - No forced recursive deletion (`--remove`) or user-level tmpfiles passes are executed.
   - Paths without cleanup rules remain untouched.
3. **Journal cleanup**: Rotates systemd journals, then vacuums archives according to configured module limits:
   - `programs.yonix.cleanup.journalMaxAge` (default: `14d`).
   - `programs.yonix.cleanup.journalMaxSizeMiB` (default: `512`).
   - Active journals are outside vacuum limits. If rotation fails, vacuuming is skipped. Journal disk usage is reported afterward.
4. **Nix garbage collection**: Removes old NixOS generations and unreachable store paths:
   - Retains generations within `programs.yonix.keepGenerations` (default: `14d`).
   - Retains all existing GC roots, including the `<state_dir>/result` symlink from the latest Yonix build.
5. **Nix store verification**: Verifies cryptographic hashes of all remaining store paths using `nix store verify --all --no-trust`.
   - Controlled by `programs.yonix.cleanup.verifyStore` (default: `true`).
   - Does not perform network downloads or signature checks.
   - On a large store, verification takes substantial time. Any read error or corruption fails the check and aborts subsequent store optimization.
6. **Store deduplication**: Deduplicates identical store files via hardlinks using `nix-store --optimise` (`programs.yonix.cleanup.optimiseStore`, default: `true`).
7. **Btrfs maintenance**: Executes bounded block-group compaction (detailed below).
8. **Filesystem TRIM**: Invokes `fstrim --fstab --verbose --quiet-unsupported` (`programs.yonix.cleanup.trim`, default: `true`).
   - Only mounted filesystems listed in `/etc/fstab` and the root filesystem are trimmed.
   - Unsupported filesystems and mounts with `X-fstrim.notrim` are honored and skipped.
   - Reported discard bytes reflect controller discard requests, not newly freed filesystem bytes.
9. **Service health check**: Reports failed systemd services. If any failed units are detected, the cleanup run marks the check as failed. Units are never automatically restarted, reset, or disabled.

### Partial Failures and Isolation

Independent stages continue even if a preceding stage encounters an error. The final summary reports completed, failed, and skipped steps, exiting with a non-zero status if any step failed.

Cleanup settings are configured exclusively through NixOS module options under `programs.yonix.cleanup`; no arbitrary flags or filesystem paths are accepted from API clients or CLI arguments.

---

## Btrfs Maintenance Engine

Btrfs maintenance compacts non-empty, sparse data block groups to return unallocated space to the storage pool without risking filesystem exhaustion.

### Block Group Inspection and Selection

- Uses the read-only Btrfs tree-search ioctl, supporting both the legacy extent tree and the modern dedicated block-group tree.
- Identifies non-empty data-only block groups with usage below 90%, sorting them sparsest first.
- Mixed data/metadata groups and metadata groups are never relocated. Used metadata is never relocated; metadata DUP profiles are preserved.
- Empty data and metadata chunks are reclaimed first.
- Only writable mounted filesystems are processed (deduplicated by UUID); read-only mounts and non-Btrfs filesystems are skipped.

### Relocation and Limits

- Each relocation targets exactly one logical range using `vrange`, `usage=90`, and `limit=1`.
- **Pre-switch pass**: A healthy filesystem attempts at most 1 chunk relocation. If unallocated reserve is below the configured threshold, it attempts up to 24 chunks, stopping as soon as the target reserve is restored.
- **Clean pass**: Attempts at most 24 data groups per filesystem per invocation, continuing through candidates to maximize compaction.
- If a relocation consumes allocation workspace rather than freeing it, the process stops immediately to avoid starving the filesystem.

### Persistent Maintenance Queue

- State is tracked per filesystem in `/var/lib/yonix-btrfs-maintenance/<UUID>.json` (root-owned, mode `0600`).
- The queue survives daemon restarts, invocations, and package updates.
- Subsequent runs resume from the existing queue rather than restarting with empty usage filters.
- If a complete sweep finishes with net allocation gains, subsequent sweeps can consolidate further.
- If an entire sweep yields no allocation gains, the layout is remembered as exhausted. Further data relocations are skipped until external changes (GC, file deletion, or recovery) modify allocation or available workspace.
- A non-blocking lock serializes concurrent maintenance runs.

### Allocation Reserve and Target Calculation

The target reserve is configured via `programs.yonix.btrfs.reserveGiB` (default: `8`, range `1`–`128`).

- The target reserve is capped at 25% of the total device capacity.
- Under metadata pressure, the target reserve is **doubled**. Pressure includes:
  - `bytes_reserved`
  - `bytes_pinned`
  - `bytes_may_use` (including global reserve)
  - `bytes_readonly`
- If unallocated space remains below the target after maintenance, or if an ioctl error occurs, the stage reports failure. Pre-switch failure aborts the configuration rebuild.

### Troubleshooting `ENOSPC`

If chunk relocation fails with `ENOSPC` (No space left on device):
1. Check allocation details:
   ```console
   sudo btrfs filesystem usage --raw /
   sudo journalctl -k -b
   ```
2. Ensure unallocated raw disk space is available for Btrfs to create new block groups. Deleting small individual files may not immediately release raw chunks if they share extents.
3. If necessary, temporarily add a device, perform maintenance, and remove the temporary device.

---

## Retention and Rollback Policy

- The build result symlink `<state_dir>/result` is registered as a Nix garbage collector root for the most recent build.
- `programs.yonix.keepGenerations` defines an age-based cutoff (e.g. `14d`). Generations older than this duration without active GC roots are deleted during `yonix clean`.
- Rollback preserves previous system generations until explicitly pruned by `yonix clean`.
