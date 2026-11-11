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
    printf 'btrfs %s\n' "$*" >> "$TEST_CALLS"
    if [ "''${TEST_BALANCE_FAILURE:-}" = all ] ||
       { [ "''${TEST_BALANCE_FAILURE:-}" = root ] && [ "$*" = 'balance start -musage=5 /' ]; }; then
      echo 'ERROR: error during balancing: No space left on device' >&2
      exit 1
    fi
  '';
  clean = pkgs.callPackage ./clean.nix {
    nix = fakeNix;
    util-linux = fakeUtilLinux;
    btrfs-progs = fakeBtrfs;
    keepGenerations = "14d";
  };
in
pkgs.runCommand "yonix-clean-check" { } ''
  export TEST_CALLS="$PWD/calls" TEST_MOUNTS="$PWD/mounts"
  cat > "$TEST_MOUNTS" <<'EOF'
  / btrfs root-uuid
  /nix btrfs root-uuid
  /home btrfs home-uuid
  /boot vfat boot-uuid
  /space\x20mount btrfs space-uuid
  EOF
  ${clean} > log 2>&1
  cat > expected <<'EOF'
  gc --delete-older-than 14d
  findmnt
  btrfs balance start -dusage=0 -musage=0 /
  btrfs balance start -musage=5 /
  btrfs balance start -dusage=5 /
  btrfs balance start -dusage=0 -musage=0 /home
  btrfs balance start -musage=5 /home
  btrfs balance start -dusage=5 /home
  btrfs balance start -dusage=0 -musage=0 /space mount
  btrfs balance start -musage=5 /space mount
  btrfs balance start -dusage=5 /space mount
  EOF
  diff -u expected "$TEST_CALLS"

  # Relocation failure must be visible and must not skip data or other mounts.
  : > "$TEST_CALLS"
  if TEST_BALANCE_FAILURE=root ${clean} > log 2>&1; then
    echo 'Expected cleanup failure after ENOSPC' >&2
    exit 1
  fi
  diff -u expected "$TEST_CALLS"
  grep -F 'WARNING: Btrfs balance -musage=5 failed for /.' log
  grep -F 'Cleanup incomplete: Nix garbage collection succeeded, but Btrfs balancing failed.' log

  # Also propagate failures in the empty-chunk pass.
  : > "$TEST_CALLS"
  if TEST_BALANCE_FAILURE=all ${clean} > log 2>&1; then exit 1; fi
  diff -u expected "$TEST_CALLS"
  grep -F 'WARNING: Btrfs balance -dusage=0 -musage=0 failed for /.' log

  # A GC failure stops before enumerating or balancing mounts.
  : > "$TEST_CALLS"
  if TEST_GC_STATUS=7 ${clean} > log 2>&1; then exit 1; fi
  printf 'gc --delete-older-than 14d\n' > expected
  diff -u expected "$TEST_CALLS"

  # No Btrfs mounts is success; a mount enumeration error is failure.
  printf '/ ext4 root-uuid\n' > "$TEST_MOUNTS"
  : > "$TEST_CALLS"
  ${clean} > log 2>&1
  printf 'gc --delete-older-than 14d\nfindmnt\n' > expected
  diff -u expected "$TEST_CALLS"
  : > "$TEST_CALLS"
  if TEST_FINDMNT_STATUS=1 ${clean} > log 2>&1; then exit 1; fi
  diff -u expected "$TEST_CALLS"
  grep -F 'mounted filesystems could not be listed.' log
  touch "$out"
''
