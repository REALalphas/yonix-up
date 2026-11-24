{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.yonix;
  officialCacheUrl = "https://yonix.cachix.org";
  officialCachePublicKey = "yonix.cachix.org-1:tuJfS1bMNuVctofrHgbmUcfYcqLfjieOmOI42ccjaqg=";
  inherit (lib)
    mkOption
    mkEnableOption
    types
    mkIf
    ;
  flakeRef = "${lib.optionalString (cfg.sourceMode == "path") "path:"}${cfg.configPath}#${cfg.host}";
  # ACL rules interpret specifiers and glob characters even in quoted paths.
  configAclPath = builtins.toJSON (
    lib.escape [ "*" "?" "[" "]" ] (lib.replaceStrings [ "%" ] [ "%%" ] cfg.configPath)
  );
  btrfsMaintenance = pkgs.callPackage ./btrfs-maintenance.nix {
    reserveBytes = cfg.btrfs.reserveGiB * 1024 * 1024 * 1024;
  };
  clean = pkgs.callPackage ./clean.nix {
    inherit btrfsMaintenance;
    keepGenerations = cfg.keepGenerations;
    inherit (cfg.cleanup)
      journalMaxAge
      journalMaxSizeMiB
      verifyStore
      optimiseStore
      trim
      ;
  };
  switch = pkgs.callPackage ./switch.nix { inherit btrfsMaintenance; };
  helper = pkgs.writeShellScript "yonix-system" ''
    set -eu
    if [ "$#" -ne 1 ]; then exit 64; fi
    case "$1" in
      switch)
        if [ -f ${lib.escapeShellArg cfg.configPath}/flake.nix ]; then
          set -- ${switch} switch --flake ${lib.escapeShellArg flakeRef} --no-write-lock-file
        elif [ -f ${lib.escapeShellArg cfg.configPath}/configuration.nix ]; then
          set -- ${switch} switch --no-flake -I nixos-config=${lib.escapeShellArg cfg.configPath}/configuration.nix
        else
          echo "No flake.nix or configuration.nix in ${lib.escapeShellArg cfg.configPath}" >&2
          exit 64
        fi
        ;;
      rollback) set -- ${pkgs.nixos-rebuild}/bin/nixos-rebuild switch --rollback --no-reexec ;;
      clean) set -- ${clean} ;;
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
    shared_token = true;
    peers = lib.mapAttrsToList (id: peer: {
      inherit id;
      inherit (peer) label url;
      token_file = peer.tokenFile;
    }) cfg.peers;
    cache =
      if cfg.cache.enable then
        {
          url = "http://${
            if cfg.cache.listenAddress == "0.0.0.0" then "127.0.0.1" else cfg.cache.listenAddress
          }:${toString cfg.cache.port}";
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
    user = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Deprecated. Adds a legacy operator to the yonix group. Use users.users.<name>.extraGroups for group membership.";
    };
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "Yonix package; use yonix-headless on servers without GTK.";
    };
    configPath = mkOption {
      type = types.strMatching "/.*";
      default = "/etc/nixos";
      description = "Absolute path to a writable NixOS configuration: a flake checkout (flake.nix) or a legacy directory (configuration.nix). Flake mode takes precedence when both files are present. Keep it on a local filesystem.";
    };
    sourceMode = mkOption {
      type = types.enum [
        "git"
        "path"
      ];
      default = "git";
      description = "Flake mode only; ignored for legacy configuration.nix setups. Git mode uses the Git-visible flake. Path mode includes local untracked and ignored files, matching checkouts with an ignored config.nix. Local source files can enter the world-readable Nix store; do not keep secrets in that checkout.";
    };
    host = mkOption {
      type = types.strMatching "[a-zA-Z0-9_-]+";
      default = config.networking.hostName;
      description = "nixosConfigurations attribute to manage.";
    };
    openFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Open the configured Yonix TCP port and LAN discovery on UDP 9847.";
    };
    listenAddress = mkOption {
      type = types.str;
      default = "0.0.0.0";
      description = "IPv4 bind address; defaults to all interfaces so LAN pairing can discover this host. Connections use HTTP; use SSH or a VPN on untrusted networks.";
    };
    port = mkOption {
      type = types.port;
      default = 9847;
      description = "Web/API port on listenAddress.";
    };
    allowSystemChanges = mkOption {
      type = types.bool;
      default = false;
      description = "Allow yonix group members and API token holders to request switch, rollback and cleanup through the daemon. Treat them as system administrators: the configured flake can execute code as root during activation.";
    };
    useOfficialCache = mkOption {
      type = types.bool;
      default = true;
      description = "Use the official signed Yonix binary cache at https://yonix.cachix.org.";
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
                description = "Runtime file containing the remote API token, mode 0600 or 0400, readable by the yonix service user. Never use a Nix path literal or put a token in the store.";
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
    cleanup = {
      journalMaxAge = mkOption {
        type = types.strMatching "[1-9][0-9]*d";
        default = "14d";
        description = "Maximum age of archived system journals during explicit clean. Active journals are rotated first.";
      };
      journalMaxSizeMiB = mkOption {
        type = types.ints.between 1 1048576;
        default = 512;
        description = "Archive size limit in MiB passed to journalctl during explicit clean. Active files are outside this limit.";
      };
      verifyStore = mkOption {
        type = types.bool;
        default = true;
        description = "Verify the hashes of all remaining Nix store contents during explicit clean, without signature checks, repair or downloads. May be slow on large stores.";
      };
      optimiseStore = mkOption {
        type = types.bool;
        default = true;
        description = "Deduplicate identical Nix store files during explicit clean. Skipped if the enabled integrity check fails.";
      };
      trim = mkOption {
        type = types.bool;
        default = true;
        description = "Run fstrim on supported mounted fstab filesystems (plus root) after explicit clean, honoring X-fstrim.notrim. Discard bytes are not newly reclaimed file space.";
      };
    };
    btrfs.reserveGiB = mkOption {
      type = types.ints.between 1 128;
      default = 8;
      description = "Unallocated Btrfs device space to recover before switch/update and after clean. Doubled under metadata pressure and capped at 25% of filesystem device size. Clean continues bounded compaction beyond this target and resumes its queue on subsequent calls. This is a checked target, not a hard reservation; no background cleanup is scheduled.";
    };
  };
  config = mkIf cfg.enable {
    warnings = lib.optional (cfg.user != null) ''
      programs.yonix.user is deprecated and will be removed in a future release.
      Yonix now runs as the shared yonix system user. Add ${cfg.user} to the yonix
      group with users.users.${cfg.user}.extraGroups = [ "yonix" ], then remove
      programs.yonix.user. When upgrading from a release that ran as ${cfg.user},
      restart yonix.service after activation to apply the new service credentials.
    '';
    assertions = [
      {
        assertion = !cfg.cache.enable || cfg.cache.port != cfg.port;
        message = "Yonix API and cache need different ports.";
      }
      {
        assertion = !(cfg.peers ? local);
        message = "The Yonix peer id local is reserved.";
      }
    ];
    users.groups.yonix.members = lib.optional (cfg.user != null) cfg.user;
    users.users.yonix = {
      isSystemUser = true;
      group = "yonix";
      home = "/var/lib/yonix";
    };
    systemd.tmpfiles.rules = [
      "A+ ${configAclPath} - - - - g:yonix:rwX,d:g:yonix:rwx"
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
    nix.settings.extra-substituters = lib.unique (
      lib.optional cfg.useOfficialCache officialCacheUrl
      ++ map (cache: cache.url) (builtins.attrValues cfg.caches)
    );
    nix.settings.extra-trusted-public-keys = lib.unique (
      lib.optional cfg.useOfficialCache officialCachePublicKey
      ++ map (cache: cache.publicKey) (builtins.attrValues cfg.caches)
    );
    # Let remote builders fetch their own dependencies from their substituters.
    # This avoids sending large dependency closures from a slow client before
    # the remote build can start.
    nix.settings.builders-use-substitutes = lib.mkDefault true;
    environment.systemPackages = [ cfg.package ];
    networking.firewall.allowedUDPPorts = lib.optionals cfg.openFirewall [ 9847 ];
    networking.firewall.allowedTCPPorts = lib.optionals cfg.openFirewall [ cfg.port ];
    environment.etc."yonix/config.json".source = json.generate "yonix-config.json" settings;
    nix.settings.experimental-features = [
      "nix-command"
      "flakes"
    ];
    systemd.services.yonix = {
      description = "Yonix NixOS companion";
      wantedBy = [ "multi-user.target" ];
      after = [
        "nix-daemon.socket"
        "systemd-tmpfiles-setup.service"
      ];
      # Do not kill an activation job when the switch updates Yonix itself.
      # Yonix can re-exec after its switch job has recorded the result.
      restartIfChanged = false;
      stopIfChanged = false;
      path = [
        pkgs.nix
        pkgs.nixos-rebuild
        pkgs.git
        pkgs.coreutils
        "/run/wrappers"
      ];
      environment = {
        GIT_CONFIG_COUNT = "1";
        GIT_CONFIG_KEY_0 = "safe.directory";
        GIT_CONFIG_VALUE_0 = cfg.configPath;
      };
      serviceConfig = {
        User = "yonix";
        Group = "yonix";
        ExecStart = "${cfg.package}/bin/yonix serve";
        StateDirectory = "yonix";
        StateDirectoryMode = "0770";
        UMask = "0007";
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
        # Self-reexec preserves the old daemon's credentials. Keep the legacy
        # operator authorized until systemd starts it as yonix.
        users = lib.unique ([ "yonix" ] ++ lib.optional (cfg.user != null) cfg.user);
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
