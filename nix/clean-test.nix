{ pkgs }:
let
  fakeNix = pkgs.writeShellScriptBin "nix-collect-garbage" ''
    printf 'gc %s\n' "$*" >> "$TEST_CALLS"
    exit "''${TEST_GC_STATUS:-0}"
  '';
  fakeUtilLinux = pkgs.writeShellScriptBin "findmnt" ''
    echo findmnt >> "$TEST_CALLS"
    cat "$TEST_MOUNTS"
    exit "''${TEST_FINDMNT_STATUS:-0}"
  '';
  fakeBtrfs = pkgs.writeShellScriptBin "btrfs" ''
    set -eu
    printf 'btrfs %s\n' "$*" >> "$TEST_CALLS"
    mount="''${!#}"
    if [ "$1 $2" = 'filesystem usage' ]; then
      if [ "''${TEST_USAGE_FAILURE:-}" = "$mount" ]; then exit 1; fi
      if [ "''${TEST_USAGE_INVALID:-}" = yes ]; then
        echo 'Unrecognized output'
        exit 0
      fi
      size=107374182400
      free=4294967296
      if [ "''${TEST_SMALL_FS:-}" = yes ]; then
        size=1073741824
        free=268435456
      fi
      if [ "''${TEST_LOW_SPACE:-}" = "$mount" ] && [ ! -f "$TEST_STATE/recovered" ]; then
        free=1048576
        if [ -f "$TEST_STATE/reserve" ]; then free=$(cat "$TEST_STATE/reserve"); fi
      fi
      printf 'Overall:\n    Device size: %s\n    Device unallocated: %s\n' "$size" "$free"
      exit 0
    fi
    if [ "''${TEST_BALANCE_FAILURE:-}" = "$mount" ]; then
      echo 'ERROR: error during balancing: No space left on device' >&2
      exit 1
    fi
    if [ "''${TEST_FAIL_DATA5:-}" = "$mount" ] && [ "$3" = '-dusage=5,limit=2' ]; then
      echo 'ERROR: error during balancing: No space left on device' >&2
      exit 1
    fi
    case "$3" in
      -dusage=0)
        if [ "''${TEST_EMPTY_RECOVERS:-}" = "$mount" ]; then touch "$TEST_STATE/recovered"; fi
        ;;
      -dusage=*,limit=2)
        if [ "''${TEST_RECOVER_AT:-}" = "$3" ]; then
          if [ -n "''${TEST_RECOVER_STEP:-}" ]; then
            free=1048576
            if [ -f "$TEST_STATE/reserve" ]; then free=$(cat "$TEST_STATE/reserve"); fi
            echo "$((free + TEST_RECOVER_STEP))" > "$TEST_STATE/reserve"
          else
            touch "$TEST_STATE/recovered"
          fi
        fi
        ;;
    esac
  '';
  fakeRebuild = pkgs.writeShellScriptBin "nixos-rebuild" ''
    printf 'rebuild %s\n' "$*" >> "$TEST_CALLS"
    exit "''${TEST_REBUILD_STATUS:-0}"
  '';
  btrfsMaintenance = pkgs.callPackage ./btrfs-maintenance.nix {
    util-linux = fakeUtilLinux;
    btrfs-progs = fakeBtrfs;
  };
  clean = pkgs.callPackage ./clean.nix {
    inherit btrfsMaintenance;
    nix = fakeNix;
    keepGenerations = "14d";
  };
  switch = pkgs.callPackage ./switch.nix {
    inherit btrfsMaintenance;
    nixos-rebuild = fakeRebuild;
  };
in
pkgs.runCommand "yonix-clean-check" { nativeBuildInputs = [ pkgs.shellcheck ]; } ''
  shellcheck ${btrfsMaintenance} ${clean} ${switch}
  export TEST_CALLS="$PWD/calls" TEST_MOUNTS="$PWD/mounts" TEST_STATE="$PWD/state"
  mkdir "$TEST_STATE"
  reset_calls() {
    : > "$TEST_CALLS"
    rm -f "$TEST_STATE/recovered" "$TEST_STATE/reserve"
  }
  expect_failure() {
    if "$@" > log 2>&1; then
      echo "Expected failure: $*" >&2
      exit 1
    fi
  }
  balance_count() {
    test "$(grep -c '^btrfs balance' "$TEST_CALLS")" -eq "$1"
  }
  cat > "$TEST_MOUNTS" <<'EOF'
  / btrfs root-uuid rw,relatime
  /nix btrfs root-uuid rw,relatime
  /home btrfs home-uuid rw,relatime
  /boot vfat boot-uuid rw,relatime
  /space\x20mount btrfs space-uuid rw,relatime
  /snapshot btrfs snapshot-uuid ro,relatime
  EOF
  # Healthy clean: one pass per writable UUID, decoded paths, limited work.
  ${clean} > log 2>&1
  cat > expected <<'EOF'
  gc --delete-older-than 14d
  findmnt
  btrfs balance start -dusage=0 /
  btrfs balance start -musage=0 /
  btrfs filesystem usage --raw /
  btrfs balance start -dusage=5,limit=2 /
  btrfs balance start -musage=5,limit=2 /
  btrfs filesystem usage --raw /
  btrfs balance start -dusage=0 /home
  btrfs balance start -musage=0 /home
  btrfs filesystem usage --raw /home
  btrfs balance start -dusage=5,limit=2 /home
  btrfs balance start -musage=5,limit=2 /home
  btrfs filesystem usage --raw /home
  btrfs balance start -dusage=0 /space mount
  btrfs balance start -musage=0 /space mount
  btrfs filesystem usage --raw /space mount
  btrfs balance start -dusage=5,limit=2 /space mount
  btrfs balance start -musage=5,limit=2 /space mount
  btrfs filesystem usage --raw /space mount
  EOF
  diff -u expected "$TEST_CALLS"

  # An empty-chunk failure is reported, but other filesystems still run.
  reset_calls
  expect_failure env TEST_BALANCE_FAILURE=/ ${clean}
  grep -F 'WARNING: Btrfs maintenance failed for /.' log
  grep -F 'Nix garbage collection succeeded, but Btrfs maintenance failed.' log
  grep -Fx 'btrfs balance start -musage=5,limit=2 /home' "$TEST_CALLS"

  printf '/home btrfs home-uuid rw,relatime\n' > "$TEST_MOUNTS"
  # Allocation recovers only at 50%: data relocation precedes used metadata.
  reset_calls
  TEST_LOW_SPACE=/home TEST_RECOVER_AT=-dusage=50,limit=2 ${clean} > log 2>&1
  test -f "$TEST_STATE/recovered"
  grep -Fx 'btrfs balance start -dusage=50,limit=2 /home' "$TEST_CALLS"
  ! grep -F -- '-dusage=75' "$TEST_CALLS"
  balance_count 8

  # Repeat a productive filter when multiple chunks are needed for reserve.
  reset_calls
  TEST_LOW_SPACE=/home TEST_RECOVER_AT=-dusage=50,limit=2 TEST_RECOVER_STEP=1073741824 ${clean} > log 2>&1
  test "$(grep -c -- '-dusage=50,limit=2' "$TEST_CALLS")" -eq 2
  ! grep -F -- '-dusage=75' "$TEST_CALLS"
  balance_count 9

  # Even slow progress cannot create an unbounded recovery loop.
  reset_calls
  expect_failure env TEST_LOW_SPACE=/home TEST_RECOVER_AT=-dusage=5,limit=2 TEST_RECOVER_STEP=1048576 ${clean}
  balance_count 14

  # Empty chunks alone suffice: do not escalate.
  reset_calls
  TEST_LOW_SPACE=/home TEST_EMPTY_RECOVERS=/home ${clean} > log 2>&1
  balance_count 4
  ! grep -F -- '-dusage=10' "$TEST_CALLS"

  # Zero relocated chunks is not recovery; stop after six bounded attempts.
  reset_calls
  expect_failure env TEST_LOW_SPACE=/home ${clean}
  grep -F 'allocation reserve is still low after bounded recovery.' log
  balance_count 8
  ! grep -F -- '-musage=5' "$TEST_CALLS"

  # An actual relocation failure is fatal without retrying harder filters.
  reset_calls
  expect_failure env TEST_LOW_SPACE=/home TEST_FAIL_DATA5=/home ${clean}
  balance_count 3
  ! grep -F -- '-dusage=10' "$TEST_CALLS"

  # Inspection failures and unknown output must not look like healthy space.
  reset_calls
  expect_failure env TEST_USAGE_FAILURE=/home ${clean}
  balance_count 2
  expect_failure env TEST_USAGE_INVALID=yes ${clean}
  grep -F 'Cannot parse Btrfs allocation' log

  # Reserve target scales down for small filesystems.
  reset_calls
  TEST_LOW_SPACE=/home TEST_SMALL_FS=yes TEST_EMPTY_RECOVERS=/home ${clean} > log 2>&1
  grep -F 'reserve target 268435456 bytes.' log

  # Switch always performs lighter Btrfs work before rebuilding, without GC.
  reset_calls
  ${switch} switch --flake 'path:/repo with spaces#nas' --no-write-lock-file > log 2>&1
  balance_count 3
  ! grep -E '^gc |musage=5|limit=2' "$TEST_CALLS"
  test "$(tail -n 1 "$TEST_CALLS")" = 'rebuild switch --flake path:/repo with spaces#nas --no-write-lock-file'
  # Legacy switch also gets maintenance and forwards its arguments.
  reset_calls
  ${switch} switch --no-flake -I nixos-config=/repo/configuration.nix > log 2>&1
  balance_count 3
  grep -Fx 'rebuild switch --no-flake -I nixos-config=/repo/configuration.nix' "$TEST_CALLS"

  # Low-space switch recovers automatically and stops before build on failure.
  reset_calls
  TEST_LOW_SPACE=/home TEST_RECOVER_AT=-dusage=75,limit=2 ${switch} switch > log 2>&1
  test -f "$TEST_STATE/recovered"
  balance_count 8
  reset_calls
  expect_failure env TEST_LOW_SPACE=/home ${switch} switch
  ! grep -q '^rebuild' "$TEST_CALLS"
  reset_calls
  expect_failure env TEST_REBUILD_STATUS=7 ${switch} switch
  grep -Fx 'rebuild switch' "$TEST_CALLS"

  # GC failure stops before filesystem work.
  reset_calls
  expect_failure env TEST_GC_STATUS=7 ${clean}
  printf 'gc --delete-older-than 14d\n' > expected
  diff -u expected "$TEST_CALLS"

  # No writable Btrfs mounts is success; enumeration failure is an error.
  printf '/ ext4 root-uuid rw\n/ro btrfs root-uuid ro\n' > "$TEST_MOUNTS"
  reset_calls
  ${clean} > log 2>&1
  printf 'gc --delete-older-than 14d\nfindmnt\n' > expected
  diff -u expected "$TEST_CALLS"
  reset_calls
  expect_failure env TEST_FINDMNT_STATUS=1 ${clean}
  grep -F 'mounted filesystems could not be listed.' log
  reset_calls
  expect_failure env TEST_FINDMNT_STATUS=1 ${switch} switch
  ! grep -q '^rebuild' "$TEST_CALLS"
  touch "$out"
''
