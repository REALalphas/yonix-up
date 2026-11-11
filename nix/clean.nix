{
  lib,
  writeShellScript,
  nix,
  btrfsMaintenance,
  keepGenerations,
}:
writeShellScript "yonix-clean" ''
  set -euo pipefail
  printf '$ nix-collect-garbage --delete-older-than %s\n' ${lib.escapeShellArg keepGenerations}
  ${nix}/bin/nix-collect-garbage --delete-older-than ${lib.escapeShellArg keepGenerations}

  if ! ${btrfsMaintenance} clean; then
    echo "Cleanup incomplete: Nix garbage collection succeeded, but Btrfs maintenance failed." >&2
    exit 1
  fi
''
