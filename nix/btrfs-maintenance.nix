{
  writeShellScript,
  util-linux,
  gawk,
  btrfs-progs,
}:
writeShellScript "yonix-btrfs-maintenance" ''
  set -euo pipefail
  export LC_ALL=C
  case "''${1:-}" in
    switch) limit=1 ;;
    clean) limit=2 ;;
    *) exit 64 ;;
  esac
  echo "Btrfs maintenance ($1)"

  # An unfiltered query distinguishes no Btrfs mounts from findmnt failure.
  if ! mounts=$(${util-linux}/bin/findmnt -rn -o TARGET,FSTYPE,UUID,OPTIONS); then
    echo "Btrfs maintenance failed: mounted filesystems could not be listed." >&2
    exit 1
  fi
  mounts=$(printf '%s\n' "$mounts" | ${gawk}/bin/awk '
    $2 == "btrfs" && $4 !~ /(^|,)ro(,|$)/ && !seen[$3]++ {print $1}
  ')

  balance() {
    printf '$ btrfs balance start %s %s\n' "$*" "$mount"
    ${btrfs-progs}/bin/btrfs balance start "$@" "$mount"
  }
  space() {
    local usage numbers
    if ! usage=$(${btrfs-progs}/bin/btrfs filesystem usage --raw "$mount"); then
      echo "Cannot inspect Btrfs allocation for $mount." >&2
      return 1
    fi
    if ! numbers=$(printf '%s\n' "$usage" | ${gawk}/bin/awk '
      $1 == "Device" && $2 == "size:" {size=$3}
      $1 == "Device" && $2 == "unallocated:" {free=$3}
      END {
        if (size !~ /^[0-9]+$/ || free !~ /^[0-9]+$/ || size <= 0 || free > size) exit 1
        printf "%.0f %.0f\n", size, free
      }
    '); then
      echo "Cannot parse Btrfs allocation for $mount." >&2
      return 1
    fi
    read -r device_size unallocated <<< "$numbers"
    # Reserve physical space for new chunks, including metadata. File-level
    # free space alone is not sufficient to avoid ENOSPC on fully allocated FSs.
    # Small filesystems still need room for DUP metadata plus system chunks.
    target=$((device_size / 4))
    [ "$target" -le 2147483648 ] || target=2147483648
    [ "$target" -gt 0 ] || target=1
    echo "Btrfs $mount: $unallocated bytes unallocated; reserve target $target bytes."
  }
  maintain() {
    # Empty chunks need no relocation workspace. Free data first, then empty
    # metadata; postpone relocation of used metadata until reserve recovery.
    balance -dusage=0 || return 1
    balance -musage=0 || return 1
    space || return 1
    if [ "$unallocated" -lt "$target" ]; then
      echo "Btrfs $mount: recovering allocation space with bounded data balance."
      # At most twelve two-chunk passes. Repeat a useful filter, escalate
      # only on no progress, and never run an unfiltered or unlimited balance.
      attempts=0
      for usage in 5 10 25 50 75 90; do
        while [ "$attempts" -lt 12 ]; do
          before=$unallocated
          balance "-dusage=$usage,limit=2" || return 1
          attempts=$((attempts + 1))
          space || return 1
          [ "$unallocated" -lt "$target" ] || break 2
          [ "$unallocated" -gt "$before" ] || break
        done
        [ "$attempts" -lt 12 ] || break
      done
      if [ "$unallocated" -lt "$target" ]; then
        echo "Btrfs $mount: allocation reserve is still low after bounded recovery." >&2
        return 1
      fi
    fi
    balance "-dusage=5,limit=$limit" || return 1
    if [ "$limit" -gt 1 ]; then
      balance "-musage=5,limit=$limit" || return 1
    fi
    # A successful balance command can relocate zero chunks. Verify the
    # allocation reserve instead of treating its exit status as recovery.
    space || return 1
    if [ "$unallocated" -lt "$target" ]; then
      echo "Btrfs $mount: allocation reserve is low after maintenance." >&2
      return 1
    fi
  }

  failed=0
  while IFS= read -r mount; do
    [ -n "$mount" ] || continue
    # findmnt --raw hex-escapes unsafe characters, including spaces.
    mount=$(printf '%b' "$mount")
    if ! maintain; then
      echo "WARNING: Btrfs maintenance failed for $mount." >&2
      failed=1
    fi
  done <<< "$mounts"
  if [ "$failed" -ne 0 ]; then
    echo "Btrfs maintenance incomplete; inspect the allocation and kernel log for the affected filesystems." >&2
    exit 1
  fi
''
