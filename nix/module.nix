{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.yonix;
  inherit (lib)
    mkOption
    mkEnableOption
    types
    mkIf
    ;
  flakeRef = "${lib.optionalString (cfg.sourceMode == "path") "path:"}${cfg.configPath}#${cfg.host}";
  helper = pkgs.writeShellScript "yonix-system" ''
    set -eu
    if [ "$#" -ne 1 ]; then exit 64; fi
    case "$1" in
      switch) set -- ${pkgs.nixos-rebuild}/bin/nixos-rebuild switch --flake ${lib.escapeShellArg flakeRef} --no-write-lock-file ;;
      rollback) set -- ${pkgs.nixos-rebuild}/bin/nixos-rebuild switch --rollback --no-reexec ;;
      clean)
        # GC first, then a soft btrfs rebalance (usage=5) on every mounted btrfs.
        # Balance failures must not fail cleanup: balance is best-effort only.
        ${pkgs.nix}/bin/nix-collect-garbage --delete-older-than ${lib.escapeShellArg cfg.keepGenerations}
        # One balance pass per filesystem (dedupe by UUID: subvolumes share one FS).
        ${pkgs.util-linux}/bin/findmnt -rn -t btrfs -o TARGET,UUID | ${pkgs.gawk}/bin/awk '!seen[$2]++ {print $1}' | while IFS= read -r mount; do
          [ -n "$mount" ] || continue
          ${pkgs.btrfs-progs}/bin/btrfs balance start -musage=5 "$mount" || true
          ${pkgs.btrfs-progs}/bin/btrfs balance start -dusage=5 "$mount" || true
        done
        exit 0
        ;;
      *) exit 64 ;;
    esac
    exec ${pkgs.util-linux}/bin/flock --nonblock /run/lock/yonix-system.lock \
      ${pkgs.coreutils}/bin/env -i HOME=/root \
      PATH=${
        lib.makeBinPath [
          pkgs.nix
          pkgs.git
          pkgs.coreutils
          pkgs.systemd
          pkgs.util-linux
        ]
      }:/run/current-system/sw/bin \
      NIX_CONFIG='experimental-features = nix-command flakes' \
      GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0=${lib.escapeShellArg cfg.configPath} \
      "$@"
  '';
  json = pkgs.formats.json { };
  settings = {
    config_path = cfg.configPath;
    source_mode = cfg.sourceMode;
    host = cfg.host;
    port = cfg.port;
    listen_address = cfg.listenAddress;
    state_dir = "/var/lib/yonix";
    peers = lib.mapAttrsToList (id: peer: {
      inherit id;
      inherit (peer) label url;
      token_file = peer.tokenFile;
    }) cfg.peers;
    cache =
      if cfg.cache.enable then
        {
          url = "http://${cfg.cache.listenAddress}:${toString cfg.cache.port}";
          public_key_file = "/var/lib/yonix-cache/public-key";
        }
      else
        null;
    privileged_helper = if cfg.allowSystemChanges then toString helper else null;
  };
in
{
  options.programs.yonix = {
    enable = mkEnableOption "Yonix desktop, CLI and local web service";
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "Yonix package; use yonix-headless on servers without GTK.";
    };
    user = mkOption {
      type = types.str;
      description = "Existing local user who owns the configuration and manages Yonix. This user can read the API token.";
    };
    configPath = mkOption {
      type = types.strMatching "/.*";
      default = "/etc/nixos";
      description = "Absolute path to a writable NixOS flake checkout. Keep it on a local filesystem.";
    };
    sourceMode = mkOption {
      type = types.enum [
        "git"
        "path"
      ];
      default = "git";
      description = "Git mode uses the Git-visible flake. Path mode includes local untracked and ignored files, matching checkouts with an ignored config.nix. Local source files can enter the world-readable Nix store; do not keep secrets in that checkout.";
    };
    host = mkOption {
      type = types.strMatching "[a-zA-Z0-9_-]+";
      default = config.networking.hostName;
      description = "nixosConfigurations attribute to manage.";
    };
    openFirewall = mkEnableOption "opening the configured Yonix TCP port";
    listenAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = "Concrete loopback or private LAN IPv4 address. LAN mode uses HTTP; use only on a trusted local network, or keep loopback and use SSH. Wildcard and public addresses are rejected.";
    };
    port = mkOption {
      type = types.port;
      default = 9847;
      description = "Web/API port on listenAddress.";
    };
    allowSystemChanges = mkOption {
      type = types.bool;
      default = false;
      description = "Allow the configured user and API token holder to switch, roll back and clean via a fixed sudo helper. Treat them as system administrators: the configured flake can execute code as root during activation.";
    };
    peers = mkOption {
      default = { };
      description = "Explicit Yonix peers accessible from this instance. Remote credentials stay on the gateway.";
      type = types.attrsOf (
        types.submodule (
          { name, ... }: {
            options = {
              label = mkOption {
                type = types.str;
                default = name;
              };
              url = mkOption {
                type = types.str;
                description = "Numeric private/loopback HTTP origin over a trusted LAN/VPN or local SSH tunnel.";
              };
              tokenFile = mkOption {
                type = types.strMatching "/.*";
                description = "Runtime file containing the remote API token, mode 0600 or 0400, readable by programs.yonix.user. Never use a Nix path literal or put a token in the store.";
              };
            };
          }
        )
      );
    };
    cache = {
      enable = mkEnableOption "serving the local Nix store as a signed binary cache";
      listenAddress = mkOption {
        type = types.str;
        default = cfg.listenAddress;
        description = "Cache bind address on the trusted LAN/VPN, or loopback.";
      };
      port = mkOption {
        type = types.port;
        default = 9848;
      };
      openFirewall = mkEnableOption "opening the binary cache TCP port";
      keyName = mkOption {
        type = types.strMatching "[a-zA-Z0-9_.-]+";
        default = "${cfg.host}-yonix";
        description = "Stable signing identity. Keys persist across rebuilds under /var/lib/yonix-cache.";
      };
    };
    caches = mkOption {
      default = { };
      description = "Trusted peer caches. Nix automatically fetches matching builds on demand; signatures remain required.";
      type = types.attrsOf (
        types.submodule {
          options = {
            url = mkOption {
              type = types.str;
              description = "HTTP(S) binary cache URL reachable by the Nix daemon.";
            };
            publicKey = mkOption {
              type = types.strMatching "[a-zA-Z0-9_.-]+:[A-Za-z0-9+/=]+";
              description = "Pinned signing public key obtained from the cache host.";
            };
          };
        }
      );
    };
    keepGenerations = mkOption {
      type = types.strMatching "[1-9][0-9]*d";
      default = "14d";
      description = "Age of generations to retain during explicit cleanup. No automatic cleanup is scheduled.";
    };
  };
  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.cache.enable || cfg.cache.port != cfg.port;
        message = "Yonix API and cache need different ports.";
      }
      {
        assertion = !(cfg.peers ? local);
        message = "The Yonix peer id local is reserved.";
      }
      {
        assertion =
          builtins.hasAttr cfg.user config.users.users && config.users.users.${cfg.user}.isNormalUser;
        message = "programs.yonix.user must name an existing normal user.";
      }
    ];
    services.nix-serve = mkIf cfg.cache.enable {
      enable = true;
      bindAddress = cfg.cache.listenAddress;
      port = cfg.cache.port;
      openFirewall = cfg.cache.openFirewall;
      secretKeyFile = "/var/lib/yonix-cache/secret-key";
    };
    systemd.services.yonix-cache-key = mkIf cfg.cache.enable {
      description = "Create persistent Yonix cache signing keys";
      before = [ "nix-serve.service" ];
      requiredBy = [ "nix-serve.service" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StateDirectory = "yonix-cache";
        StateDirectoryMode = "0755";
        UMask = "0077";
      };
      script = ''
        set -eu
        cd /var/lib/yonix-cache
        if [ ! -e secret-key ]; then
          ${pkgs.nix}/bin/nix-store --generate-binary-cache-key ${lib.escapeShellArg cfg.cache.keyName} secret-key.next public-key.next
          chmod 600 secret-key.next
          chmod 644 public-key.next
          mv public-key.next public-key
          mv secret-key.next secret-key
        fi
        test -s secret-key
        test -s public-key
      '';
    };
    systemd.services.nix-serve = mkIf cfg.cache.enable {
      after = [ "yonix-cache-key.service" ];
      requires = [ "yonix-cache-key.service" ];
    };
    nix.settings.extra-substituters = map (cache: cache.url) (builtins.attrValues cfg.caches);
    nix.settings.extra-trusted-public-keys = map (cache: cache.publicKey) (
      builtins.attrValues cfg.caches
    );
    environment.systemPackages = [ cfg.package ];
    networking.firewall.allowedTCPPorts = lib.optionals cfg.openFirewall [ cfg.port ];
    environment.etc."yonix/config.json".source = json.generate "yonix-config.json" settings;
    nix.settings.experimental-features = [
      "nix-command"
      "flakes"
    ];
    systemd.services.yonix = {
      description = "Yonix NixOS companion";
      wantedBy = [ "multi-user.target" ];
      after = [ "nix-daemon.socket" ];
      # Do not kill an activation job when the switch updates Yonix itself.
      # The new daemon version starts on the next explicit restart or boot.
      restartIfChanged = false;
      stopIfChanged = false;
      path = [
        pkgs.nix
        pkgs.git
        pkgs.coreutils
        "/run/wrappers"
      ];
      environment = {
        HOME = config.users.users.${cfg.user}.home;
      };
      serviceConfig = {
        User = cfg.user;
        ExecStart = "${cfg.package}/bin/yonix serve";
        StateDirectory = "yonix";
        StateDirectoryMode = "0700";
        UMask = "0077";
        Restart = "on-failure";
        RestartSec = 3;
        KillMode = "control-group";
        # The fixed sudo helper needs setuid; all other operations stay unprivileged.
        NoNewPrivileges = !cfg.allowSystemChanges;
      };
    };
    security.sudo.enable = mkIf cfg.allowSystemChanges true;
    security.sudo.extraRules = lib.optionals cfg.allowSystemChanges [
      {
        users = [ cfg.user ];
        commands =
          map
            (action: {
              command = "${helper} ${action}";
              options = [ "NOPASSWD" ];
            })
            [
              "switch"
              "rollback"
              "clean"
            ];
      }
    ];
  };
}
