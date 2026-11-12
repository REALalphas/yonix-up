{ pkgs }:
let
  fakeNix = pkgs.writeShellScriptBin "nix-collect-garbage" ''
    printf 'gc %s\n' "$*" >> "$TEST_CALLS"
    exit "''${TEST_GC_STATUS:-0}"
  '';
  fakeMaintenance = pkgs.writeShellScript "fake-maintenance" ''
    printf 'maintenance %s\n' "$*" >> "$TEST_CALLS"
    if [ "$1" = report ]; then exit "''${TEST_REPORT_STATUS:-0}"; fi
    exit "''${TEST_MAINTENANCE_STATUS:-0}"
  '';
  fakeRebuild = pkgs.writeShellScriptBin "nixos-rebuild" ''
    printf 'rebuild %s\n' "$*" >> "$TEST_CALLS"
    exit "''${TEST_REBUILD_STATUS:-0}"
  '';
  btrfsMaintenance = pkgs.callPackage ./btrfs-maintenance.nix { };
  clean = pkgs.callPackage ./clean.nix {
    btrfsMaintenance = fakeMaintenance;
    nix = fakeNix;
    keepGenerations = "14d";
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
    shellcheck ${btrfsMaintenance} ${clean} ${switch}
    python3 ${../tests/btrfs-maintenance-test.py} ${./btrfs-maintenance.py}
    export TEST_CALLS="$PWD/calls"
    expect_failure() {
      if "$@" > log 2>&1; then
        echo "Expected failure: $*" >&2
        exit 1
      fi
    }
    ${clean} > log 2>&1
    printf 'maintenance report\ngc --delete-older-than 14d\nmaintenance clean\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_GC_STATUS=7 ${clean}
    printf 'maintenance report\ngc --delete-older-than 14d\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_MAINTENANCE_STATUS=1 ${clean}
    grep -F 'Nix garbage collection succeeded, but Btrfs maintenance failed.' log
    printf 'maintenance report\ngc --delete-older-than 14d\nmaintenance clean\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_REPORT_STATUS=1 ${clean}
    printf 'maintenance report\n' > expected
    diff -u expected "$TEST_CALLS"

    : > "$TEST_CALLS"
    ${switch} switch --flake 'path:/repo with spaces#nas' --no-write-lock-file > log 2>&1
    printf 'maintenance switch\nrebuild switch --flake path:/repo with spaces#nas --no-write-lock-file\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    ${switch} switch --no-flake -I nixos-config=/repo/configuration.nix > log 2>&1
    printf 'maintenance switch\nrebuild switch --no-flake -I nixos-config=/repo/configuration.nix\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_MAINTENANCE_STATUS=1 ${switch} switch
    printf 'maintenance switch\n' > expected
    diff -u expected "$TEST_CALLS"
    : > "$TEST_CALLS"
    expect_failure env TEST_REBUILD_STATUS=7 ${switch} switch
    grep -Fx 'rebuild switch' "$TEST_CALLS"
    touch "$out"
  ''
