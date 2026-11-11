{
  writeShellScript,
  util-linux,
  gawk,
  btrfs-progs,
  reserveBytes ? 8589934592,
  sysfsRoot ? "/sys/fs/btrfs",
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
    $2 == "btrfs" && $4 !~ /(^|,)ro(,|$)/ && !seen[$3]++ {print $1, $3}
  ')

  balance() {
    printf '$ btrfs balance start %s %s\n' "$*" "$mount"
    if ! ${btrfs-progs}/bin/btrfs balance start "$@" "$mount"; then
      echo "Btrfs $mount: balance failed; file-level free space does not guarantee room for metadata relocation." >&2
      printf 'Inspect: btrfs filesystem usage --raw %q; journalctl -k -b\n' "$mount" >&2
      echo "If relocation fails with ENOSPC, restore allocation workspace before retrying; increasing limit or deleting small files may not help." >&2
      return 1
    fi
  }
  space() {
    local usage numbers metadata_dir counter value total used committed headroom minimum_headroom reserve
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
    # df and metadata used alone miss space already committed to transactions.
    # Read the same space_info counters that the allocator uses, including
    # bytes_may_use (which includes the global reserve).
    metadata_dir="${sysfsRoot}/$uuid/allocation/metadata"
    total=0 used=0 committed=0
    for counter in total_bytes bytes_used bytes_reserved bytes_pinned bytes_may_use bytes_readonly; do
      if ! IFS= read -r value < "$metadata_dir/$counter"; then
        echo "Cannot read Btrfs metadata counter $counter for $mount." >&2
        return 1
      fi
      if [[ ! "$value" =~ ^[0-9]+$ ]] || [ "''${#value}" -gt 18 ]; then
        echo "Cannot parse Btrfs metadata counter $counter for $mount." >&2
        return 1
      fi
      value=$((10#$value))
      case "$counter" in
        total_bytes) total=$value ;;
        bytes_used) used=$value; committed=$((committed + value)) ;;
        *) committed=$((committed + value)) ;;
      esac
    done
    if [ "$total" -le 0 ] || [ "$used" -gt "$total" ]; then
      echo "Invalid Btrfs metadata capacity for $mount." >&2
      return 1
    fi
    headroom=$((total - committed))
    minimum_headroom=$((total / 8))
    [ "$minimum_headroom" -le 268435456 ] || minimum_headroom=268435456
    reserve=${toString reserveBytes}
    if [ "$((used * 100 / total))" -ge 85 ] || [ "$headroom" -lt "$minimum_headroom" ]; then
      echo "WARNING: Btrfs $mount: metadata pressure (used=$used, committed=$committed, total=$total, headroom=$headroom bytes)." >&2
      # Leave room for more metadata chunks while writes are still possible.
      reserve=$((reserve * 2))
    else
      echo "Btrfs $mount: metadata headroom $headroom bytes (used=$used, total=$total)."
    fi
    # Reserve physical space for new chunks, including metadata. File-level
    # free space alone is not sufficient to avoid ENOSPC on fully allocated FSs.
    # Small filesystems still need room for DUP metadata plus system chunks.
    target=$((device_size / 4))
    [ "$target" -le "$reserve" ] || target=$reserve
    [ "$target" -gt 0 ] || target=1
    echo "Btrfs $mount: $unallocated bytes unallocated; reserve target $target bytes."
  }
  maintain() {
    # Empty chunks need no relocation workspace. Free data first, then empty
    # metadata. Keep allocated metadata space rather than compacting it.
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
    # A successful balance command can relocate zero chunks. Verify the
    # allocation reserve instead of treating its exit status as recovery.
    space || return 1
    if [ "$unallocated" -lt "$target" ]; then
      echo "Btrfs $mount: allocation reserve is low after maintenance." >&2
      return 1
    fi
  }

  failed=0
  while read -r mount uuid; do
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
