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
      installerSystem = "x86_64-linux";
      yonixUrl = "github:REALalphas/yonix-up" + nixpkgs.lib.optionalString (self ? rev) "/${self.rev}";
      nixpkgsUrl = "github:NixOS/nixpkgs/${nixpkgs.rev}";
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
        // nixpkgs.lib.optionalAttrs (system == installerSystem) {
          installer = pkgs.callPackage ./nix/installer.nix { inherit yonixUrl nixpkgsUrl; };
          installer-iso = self.nixosConfigurations.yonix-live.config.system.build.isoImage;
        }
      );
      apps.${installerSystem}.install = {
        type = "app";
        program = "${self.packages.${installerSystem}.installer}/bin/yonix-install";
      };
      nixosConfigurations.yonix-live = nixpkgs.lib.nixosSystem {
        system = installerSystem;
        modules = [
          "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
          {
            isoImage.edition = "yonix";
            environment.systemPackages = [ self.packages.${installerSystem}.installer ];
          }
        ];
      };
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
        clean = import ./nix/clean-test.nix {
          pkgs = import nixpkgs { inherit system; };
        };
        btrfs = import ./nix/btrfs-vm-test.nix {
          pkgs = import nixpkgs { inherit system; };
        };
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
          assert builtins.elem "https://yonix.cachix.org" config.nix.settings.extra-substituters;
          assert builtins.elem "yonix.cachix.org-1:tuJfS1bMNuVctofrHgbmUcfYcqLfjieOmOI42ccjaqg=" config.nix.settings.extra-trusted-public-keys;
          assert config.nix.settings.builders-use-substitutes;
          assert config.services.nix-serve.secretKeyFile == "/var/lib/yonix-cache/secret-key";
          assert builtins.elem "http://10.0.0.2:9848" config.nix.settings.extra-substituters;
          assert config.systemd.services.yonix.serviceConfig.User == "yonix";
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
