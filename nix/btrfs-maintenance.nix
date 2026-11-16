{
  lib,
  writeShellScript,
  python3,
  util-linux,
  btrfs-progs,
  reserveBytes ? 8589934592,
  sysfsRoot ? "/sys/fs/btrfs",
  stateDir ? "/var/lib/yonix-btrfs-maintenance",
  cleanLimit ? 24,
}:
writeShellScript "yonix-btrfs-maintenance" ''
  exec ${python3}/bin/python3 ${./btrfs-maintenance.py} \
    --btrfs ${btrfs-progs}/bin/btrfs --findmnt ${util-linux}/bin/findmnt \
    --reserve ${toString reserveBytes} --sysfs ${lib.escapeShellArg sysfsRoot} \
    --state-dir ${lib.escapeShellArg stateDir} --clean-limit ${toString cleanLimit} "$@"
''
