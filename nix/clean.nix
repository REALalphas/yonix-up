{
  lib,
  writeShellScript,
  nix,
  btrfsMaintenance,
  keepGenerations,
}:
writeShellScript "yonix-clean" ''
  set -euo pipefail
  echo "Allocation before Nix GC (read-only inspection):"
  ${btrfsMaintenance} report
  printf '$ nix-collect-garbage --delete-older-than %s\n' ${lib.escapeShellArg keepGenerations}
  ${nix}/bin/nix-collect-garbage --delete-older-than ${lib.escapeShellArg keepGenerations}

  echo "Btrfs maintenance after Nix GC (summary baseline excludes GC):"
  if ! ${btrfsMaintenance} clean; then
    echo "Cleanup incomplete: Nix garbage collection succeeded, but Btrfs maintenance failed." >&2
    exit 1
  fi
''
