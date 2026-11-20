{
  writeShellApplication,
  coreutils,
  util-linux,
  systemd,
  jq,
  dosfstools,
  e2fsprogs,
  btrfs-progs,
  openssl,
  tzdata,
  nix,
  nixos-install,
  nixos-enter,
  yonixUrl,
  nixpkgsUrl,
}:
writeShellApplication {
  name = "yonix-install";
  runtimeInputs = [
    coreutils
    util-linux
    systemd
    jq
    dosfstools
    e2fsprogs
    btrfs-progs
    openssl
    nix
    nixos-install
    nixos-enter
  ];
  runtimeEnv = {
    YONIX_FLAKE_URL = yonixUrl;
    NIXPKGS_FLAKE_URL = nixpkgsUrl;
    ZONEINFO = "${tzdata}/share/zoneinfo";
  };
  text = builtins.readFile ./installer.sh;
}
