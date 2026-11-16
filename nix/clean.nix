{
  lib,
  writeShellScript,
  nix,
  systemd,
  coreutils,
  util-linux,
  btrfsMaintenance,
  keepGenerations,
  journalMaxAge ? "14d",
  journalMaxSizeMiB ? 512,
  verifyStore ? true,
  optimiseStore ? true,
  trim ? true,
}:
writeShellScript "yonix-clean" ''
  set -euo pipefail
  failed=0
  completed=0
  skipped=0
  run_step() {
    local label="$1" status
    shift
    printf '\n=== %s ===\n' "$label"
    printf '$'; printf ' %q' "$@"; printf '\n'
    if "$@"; then
      completed=$((completed + 1))
      printf 'OK: %s\n' "$label"
      return 0
    else
      status=$?
      failed=$((failed + 1))
      printf 'FAILED: %s (exit %s); continuing independent steps.\n' "$label" "$status" >&2
      return 1
    fi
  }
  skip_step() {
    skipped=$((skipped + 1))
    printf '\nSKIPPED: %s\n' "$1"
  }
  disk_report() {
    ${coreutils}/bin/df -h -l || return "$?"
    ${coreutils}/bin/df -i -l
  }
  failed_units() {
    local units
    units=$(${systemd}/bin/systemctl --failed --no-legend --plain --no-pager) || return "$?"
    if [ -n "$units" ]; then
      printf '%s\n' "$units"
      echo 'Failed system services require attention; no services were reset or restarted.' >&2
      return 1
    fi
    echo 'No failed system services.'
  }

  echo 'Yonix clean: system maintenance; personal files and application data are outside the cleanup policy.'
  run_step 'Local disk space and inode usage before cleanup' disk_report || :
  run_step 'Allocation before Nix GC (read-only inspection)' ${btrfsMaintenance} report || :

  # Honor administrator/vendor age, lock, ownership and exclusion rules. Never
  # use --remove, --boot or an application-side recursive deletion fallback.
  run_step 'Expired system temporary files, caches and crash dumps (systemd policy)' \
    ${systemd}/bin/systemd-tmpfiles --clean \
    --prefix=/tmp --prefix=/var/tmp --prefix=/var/cache --prefix=/var/lib/systemd/coredump || :

  # Rotation makes old active journal files eligible for bounded vacuuming.
  if run_step 'Rotate system journal' ${systemd}/bin/journalctl --rotate; then
    run_step 'Vacuum archived system journals' ${systemd}/bin/journalctl \
      --vacuum-time=${lib.escapeShellArg journalMaxAge} --vacuum-size=${toString journalMaxSizeMiB}M || :
  else
    skip_step 'Journal vacuum: rotation failed'
  fi
  run_step 'Journal disk usage' ${systemd}/bin/journalctl --disk-usage || :
  run_step 'Old Nix generations and unreachable store paths' \
    ${nix}/bin/nix-collect-garbage --delete-older-than ${lib.escapeShellArg keepGenerations} || :

  store_ok=1
  ${
    if verifyStore then
      ''
        if ! run_step 'Nix store content integrity (no repair or signature check)' \
          ${nix}/bin/nix --extra-experimental-features nix-command store verify --all --no-trust; then
          store_ok=0
        fi
      ''
    else
      ''
        skip_step 'Nix store integrity check: disabled in configuration'
      ''
  }
  if [ "$store_ok" -ne 1 ]; then
    echo 'Store integrity check failed; no automatic repair or download was attempted.' >&2
  fi
  ${
    if optimiseStore then
      ''
        if [ "$store_ok" -eq 1 ]; then
          run_step 'Deduplicate identical Nix store files' ${nix}/bin/nix-store --optimise || :
        else
          skip_step 'Nix store optimisation: integrity check failed'
        fi
      ''
    else
      ''
        skip_step 'Nix store optimisation: disabled in configuration'
      ''
  }

  echo 'Btrfs maintenance after system cleanup (summary baseline excludes file cleanup):'
  run_step 'Bounded Btrfs maintenance' ${btrfsMaintenance} clean || :
  ${
    if trim then
      ''
        run_step 'TRIM supported mounted fstab filesystems (honor X-fstrim.notrim)' \
          ${util-linux}/bin/fstrim --fstab --verbose --quiet-unsupported || :
      ''
    else
      ''
        skip_step 'Filesystem TRIM: disabled in configuration'
      ''
  }
  run_step 'Failed system services' failed_units || :
  run_step 'Local disk space and inode usage after cleanup' disk_report || :
  printf '\nYonix clean summary: %s completed, %s failed, %s skipped.\n' "$completed" "$failed" "$skipped"
  echo 'Disk usage may change concurrently; journal size limits apply to archived files, and TRIM bytes are not newly freed file space.'
  if [ "$failed" -ne 0 ]; then
    echo 'Cleanup/checks incomplete: inspect FAILED entries above.' >&2
    exit 1
  fi
''
