{
  lib,
  writeShellScript,
  nix,
  util-linux,
  gawk,
  btrfs-progs,
  keepGenerations,
}:
writeShellScript "yonix-clean" ''
  set -euo pipefail
  printf '$ nix-collect-garbage --delete-older-than %s\n' ${lib.escapeShellArg keepGenerations}
  ${nix}/bin/nix-collect-garbage --delete-older-than ${lib.escapeShellArg keepGenerations}

  # Enumerate all mounts: filtering in findmnt would also return failure when
  # there are no Btrfs filesystems, hiding actual enumeration errors.
  if ! mounts=$(${util-linux}/bin/findmnt -rn -o TARGET,FSTYPE,UUID); then
    echo "Cleanup incomplete: Nix garbage collection succeeded, but mounted filesystems could not be listed." >&2
    exit 1
  fi
  mounts=$(printf '%s\n' "$mounts" | ${gawk}/bin/awk '$2 == "btrfs" && !seen[$3]++ {print $1}')
  failed=0
  balance() {
    if ! ${btrfs-progs}/bin/btrfs balance start "$@" "$mount"; then
      echo "WARNING: Btrfs balance $* failed for $mount." >&2
      failed=1
    fi
  }
  # Subvolumes share a filesystem: run once per UUID and keep going after
  # failures so that one full filesystem does not prevent cleanup of others.
  while IFS= read -r mount; do
    [ -n "$mount" ] || continue
    # findmnt --raw hex-escapes unsafe characters, including spaces.
    mount=$(printf '%b' "$mount")
    # Empty chunks can be reclaimed without allocating relocation workspace.
    balance -dusage=0 -musage=0
    balance -musage=5
    balance -dusage=5
  done <<< "$mounts"
  if [ "$failed" -ne 0 ]; then
    echo "Cleanup incomplete: Nix garbage collection succeeded, but Btrfs balancing failed. Inspect btrfs filesystem usage and the kernel log for the affected filesystems." >&2
    exit 1
  fi
''
