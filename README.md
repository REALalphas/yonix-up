# Yonix

Yonix is a NixOS administration companion with one daemon shared by the CLI,
GTK desktop app, and web panel. It checks, builds, compares, activates, rolls
back, and cleans NixOS systems. It also manages several hosts and signed binary
caches.

Yonix is experimental software. Review the trust model before enabling actions
that change the system.

## Install on NixOS

> The [live ISO installer](docs/live-installer.md) covers fresh installations.

Add the flake input and module:

```nix
inputs.yonix.url = "github:REALalphas/yonix-up";
inputs.yonix.inputs.nixpkgs.follows = "nixpkgs";

{ inputs, ... }:
{
  imports = [ inputs.yonix.nixosModules.default ];

  programs.yonix = {
    enable = true;
    # The module adds https://yonix.cachix.org automatically.
    configPath = "/etc/nixos";
    allowSystemChanges = false;
  };

  # Add each administrator who should use Yonix to the shared group.
  users.users.alice = {
    isNormalUser = true;
    extraGroups = [ "wheel" "yonix" ];
  };
  users.users.bob.extraGroups = [ "yonix" ];
}
```

Members of the `yonix` group can use the daemon and edit the configured
checkout. Use `allowSystemChanges = true` only for administrators. A changed
NixOS configuration can execute code as root during activation.

The module grants access within `configPath`. When it sits under a private
directory, give the `yonix` service user search permission on each private
parent so it can reach the checkout. An activation script ordered after
`users` reapplies the ACL when NixOS resets the home directory mode. The
example covers a checkout at `/home/alice/nixos`.

```nix
system.activationScripts.yonixConfigAccess = {
  deps = [ "users" ];
  text = ''
    ${pkgs.acl}/bin/setfacl -m u:yonix:--x /home/alice
  '';
};
```

If an upgrade leaves the daemon running as your former operator account, run
`sudo systemctl restart yonix.service` once the active operation has finished.

Use `yonix-headless` on servers without GTK:

```nix
programs.yonix.package = inputs.yonix.packages.${pkgs.stdenv.hostPlatform.system}.yonix-headless;
```

## Daily use

```console
yonix status       # system and configuration status
yonix check        # evaluate without building or activating
yonix build        # build and retain the result
yonix diff         # compare the result with the running system
yonix switch       # build and activate
yonix rollback     # select the previous generation
yonix clean        # run explicit maintenance
yonix history      # list jobs
yonix logs JOB_ID  # inspect a job
yonix web          # open the browser panel
yonix token        # print the local API token
```

The standalone daemon listens on `http://127.0.0.1:9847` by default. The NixOS
module enables LAN access on TCP/UDP 9847 by default so pairing discovery works
between hosts. Open the local URL and sign in with `yonix token`. For remote
access to a standalone daemon, use an SSH tunnel:

```console
ssh -N -L 9847:127.0.0.1:9847 user@server
```

Read [Web Interface and Remote Access](docs/remote-access.md) before exposing a
LAN or wildcard address.

## Documentation

- [Maintenance and Btrfs optimization](docs/maintenance.md)
- [Web interface and remote access](docs/remote-access.md)
- [Legacy `configuration.nix` support](docs/legacy-configurations.md)
- [Multiple hosts and signed caches](docs/peering.md)
- [Development and verification](docs/development.md)

## Development

```console
nix develop
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm ci && npm run build && npm test
nix build .#default --no-link
```

The full VM and packaging checks are listed in the [development guide](docs/development.md).

## License

Yonix application code is licensed under the [GNU Affero General Public License v3 or later](LICENSE).
The bundled Alpha UI has separate terms. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
