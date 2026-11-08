{
  description = "Yonix — NixOS companion for desktop, terminal and browser";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        rec {
          yonix = pkgs.callPackage ./nix/package.nix { };
          yonix-headless = yonix.override { withGui = false; };
          default = yonix;
        }
      );
      nixosModules.default = import ./nix/module.nix { inherit self; };
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              pkg-config
              gtk4
              libadwaita
              xvfb-run
              dbus
            ];
            inputsFrom = [ self.packages.${system}.yonix ];
          };
        }
      );
      checks = forAllSystems (system: {
        package = self.packages.${system}.yonix;
        vm = import ./nix/vm-test.nix {
          inherit self system;
          pkgs = import nixpkgs { inherit system; };
        };
        peers = import ./nix/peers-vm-test.nix {
          inherit self system;
          pkgs = import nixpkgs { inherit system; };
        };
        module =
          let
            evaluated = nixpkgs.lib.nixosSystem {
              inherit system;
              modules = [
                self.nixosModules.default
                ({ ... }: {
                  system.stateVersion = "26.05";
                  boot.isContainer = true;
                  networking.hostName = "test";
                  users.users.operator = {
                    isNormalUser = true;
                  };
                  programs.yonix = {
                    enable = true;
                    user = "operator";
                    configPath = "/home/operator/nixos";
                    allowSystemChanges = true;
                    listenAddress = "0.0.0.0";
                    peers.web = {
                      url = "http://10.0.0.2:9847";
                      tokenFile = "/run/secrets/web-token";
                    };
                    cache.enable = true;
                    caches.web = {
                      url = "http://10.0.0.2:9848";
                      publicKey = "web:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
                    };
                  };
                })
              ];
            };
            pkgs = import nixpkgs { inherit system; };
            config = evaluated.config;
          in
          assert config.services.nix-serve.enable;
          assert config.services.nix-serve.secretKeyFile == "/var/lib/yonix-cache/secret-key";
          assert builtins.elem "http://10.0.0.2:9848" config.nix.settings.extra-substituters;
          assert config.systemd.services.yonix.serviceConfig.User == "operator";
          assert config.systemd.services.yonix.serviceConfig.NoNewPrivileges == false;
          assert config.programs.yonix.cache.listenAddress == "0.0.0.0";
          pkgs.writeText "yonix-module-check" (
            builtins.toJSON {
              configFile = config.environment.etc."yonix/config.json".source;
              service = config.systemd.services.yonix.serviceConfig;
              sudo = config.security.sudo.extraRules;
            }
          );
      });
    };
}
