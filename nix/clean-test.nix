{ pkgs }:
let
  # Only these fake executables are used by the clean integration tests: never
  # invoke host GC, tmpfiles, journal vacuum, optimise, balance or discard.
  fakeTools = pkgs.runCommand "yonix-clean-fake-tools" { } ''
    mkdir -p "$out/bin"
    for name in nix-collect-garbage nix-store nix systemd-tmpfiles journalctl systemctl df fstrim; do
      ln -s ${pkgs.writeShellScript "yonix-clean-fake-tool" ''
        name="''${0##*/}"
        printf '%s' "$name" >> "$TEST_CALLS"
        printf ' <%s>' "$@" >> "$TEST_CALLS"
        printf '\n' >> "$TEST_CALLS"
        if [ "$name" = systemctl ]; then printf '%s' "''${TEST_FAILED_UNITS:-}"; fi
        if [ "$name $*" = "''${TEST_FAIL_COMMAND:-}" ]; then exit 7; fi
      ''} "$out/bin/$name"
    done
  '';
  fakeMaintenance = pkgs.writeShellScript "fake-maintenance" ''
    printf 'maintenance <%s>\n' "$*" >> "$TEST_CALLS"
    if [ "$1" = report ]; then exit "''${TEST_REPORT_STATUS:-0}"; fi
    exit "''${TEST_MAINTENANCE_STATUS:-0}"
  '';
  fakeRebuild = pkgs.writeShellScriptBin "nixos-rebuild" ''
    printf 'rebuild %s\n' "$*" >> "$TEST_CALLS"
    exit "''${TEST_REBUILD_STATUS:-0}"
  '';
  btrfsMaintenance = pkgs.callPackage ./btrfs-maintenance.nix { };
  makeClean =
    extra:
    pkgs.callPackage ./clean.nix (
      {
        btrfsMaintenance = fakeMaintenance;
        nix = fakeTools;
        systemd = fakeTools;
        coreutils = fakeTools;
        util-linux = fakeTools;
        keepGenerations = "14d";
      }
      // extra
    );
  clean = makeClean { };
  minimalClean = makeClean {
    verifyStore = false;
    optimiseStore = false;
    trim = false;
  };
  customClean = makeClean {
    journalMaxAge = "30d";
    journalMaxSizeMiB = 128;
    keepGenerations = "7d";
  };
  switch = pkgs.callPackage ./switch.nix {
    btrfsMaintenance = fakeMaintenance;
    nixos-rebuild = fakeRebuild;
  };
in
pkgs.runCommand "yonix-clean-check"
  {
    nativeBuildInputs = [
      pkgs.shellcheck
      pkgs.python3
    ];
  }
  ''
    shellcheck ${btrfsMaintenance} ${clean} ${minimalClean} ${customClean} ${switch}
    python3 ${../tests/btrfs-maintenance-test.py} ${./btrfs-maintenance.py}
    python3 ${../tests/clean-test.py} ${clean} ${minimalClean} ${customClean} ${pkgs.systemd}/bin/systemd-tmpfiles
    export TEST_CALLS="$PWD/calls"
    expect_failure() {
      if "$@" > log 2>&1; then
        echo "Expected failure: $*" >&2
        exit 1
      fi
    }
    : > "$TEST_CALLS"
    ${switch} switch --flake 'path:/repo with spaces#nas' --no-write-lock-file > log 2>&1
    printf 'maintenance <switch>\nrebuild switch --flake path:/repo with spaces#nas --no-write-lock-file\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    ${switch} switch --no-flake -I nixos-config=/repo/configuration.nix > log 2>&1
    printf 'maintenance <switch>\nrebuild switch --no-flake -I nixos-config=/repo/configuration.nix\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_MAINTENANCE_STATUS=1 ${switch} switch
    printf 'maintenance <switch>\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_REBUILD_STATUS=7 ${switch} switch
    grep -Fx 'rebuild switch' "$TEST_CALLS"
    touch "$out"
  ''
