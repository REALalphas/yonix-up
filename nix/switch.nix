{
  writeShellScript,
  nixos-rebuild,
  btrfsMaintenance,
}:
writeShellScript "yonix-switch" ''
  set -eu
  ${btrfsMaintenance} switch
  exec ${nixos-rebuild}/bin/nixos-rebuild "$@"
''
