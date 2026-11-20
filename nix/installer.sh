die() {
  printf 'yonix-install: %s\n' "$*" >&2
  exit 1
}

if [[ ${1-} == --help && $# == 1 ]]; then
  printf '%s\n' 'yonix-install — interactive full-disk NixOS installation (x86-64 UEFI)' \
    'Boot a NixOS live image, connect to the Internet and run as root in a terminal.'
  exit 0
fi

[[ $# == 0 ]] || die "Usage: yonix-install [--help]"
[[ $EUID == 0 && -t 0 && -t 1 ]] || die "Run as root in an interactive terminal."
[[ $(uname -m) == x86_64 ]] || die "Boot the live ISO in x86-64 UEFI mode."
efi_options=$(findmnt -n -t efivarfs -o OPTIONS --mountpoint /sys/firmware/efi/efivars) ||
  die "Boot the live ISO in x86-64 UEFI mode."
[[ ,$efi_options, == *,rw,* ]] || die "UEFI variables must be writable."
umask 022
shopt -s nullglob
[[ -r /etc/NIXOS && -e /run/booted-system && -r /iso/nix-store.squashfs ]] ||
  die "Run from a NixOS installation ISO."
command -v nixos-generate-config >/dev/null || die "nixos-generate-config is missing from the live system."

exec 9>/run/lock/yonix-install.lock
flock -n 9 || die "Another Yonix installation is running."

ask() {
  IFS= read -r -p "$1" reply || die "Input ended."
}

pick_timezone() {
  local LC_ALL=C
  local query country zone magic index
  local -a matches
  while true; do
    ask 'Time zone [Default: UTC] (enter a name or search by city or region): '
    query=${reply:-UTC}
    if [[ $query =~ ^[A-Za-z0-9_+-]+(/[A-Za-z0-9_+-]+)*$ && -f $ZONEINFO/$query ]] &&
       IFS= read -r -N 4 magic < "$ZONEINFO/$query" && [[ $magic == TZif ]]; then
      timezone=$query
      return
    fi

    query=${query,,}
    query=${query// /_}
    matches=()
    while IFS=$'\t' read -r country _ zone _; do
      if [[ $country != \#* && -n $zone && ${zone,,} == *"$query"* ]]; then
        matches+=("$zone")
      fi
    done < "$ZONEINFO/zone.tab"

    if ((${#matches[@]} == 0)); then
      printf '%s\n' 'No matching time zones. Try another search.'
    elif ((${#matches[@]} > 20)); then
      printf 'Found %d time zones. Enter a more specific search.\n' "${#matches[@]}"
    else
      for index in "${!matches[@]}"; do
        printf '%d) %s\n' "$((index + 1))" "${matches[index]}"
      done
      ask 'Choose a number or press Enter to search again: '
      if [[ $reply =~ ^[1-9][0-9]?$ ]] && ((reply <= ${#matches[@]})); then
        timezone=${matches[reply - 1]}
        return
      fi
      if [[ -n $reply ]]; then
        printf '%s\n' 'Invalid selection. Try another search.'
      fi
    fi
  done
}

disk_info() {
  lsblk --json --tree --bytes --paths --output PATH,KNAME,TYPE,RO,SIZE,MAJ:MIN,SERIAL,WWN,MOUNTPOINTS -- "$disk"
}

check_disk() {
  local info node pools=
  local -a holders btrfs_devices
  info=$(disk_info) || die "Cannot inspect $disk."
  jq -e '
    .blockdevices | length == 1 and
    (.[0] | .type == "disk" and (.ro | not) and .size >= 16 * 1024 * 1024 * 1024 and
      ([recurse(.children[]?) | select(
        (.type != "disk" and .type != "part") or any(.mountpoints[]?; . != null)
      )] | length == 0))
  ' <<< "$info" >/dev/null || die "$disk must be a writable disk of at least 16 GiB with no active volumes."
  if [[ -d /sys/module/zfs ]]; then
    pools=$(zpool status -LP 2>/dev/null) || die "Cannot inspect active ZFS pools."
  fi
  for node in $(jq -r '.blockdevices[] | recurse(.children[]?) | .kname' <<< "$info"); do
    if [[ $pools =~ (^|[[:space:]])"$node"([[:space:]]|$) ]]; then
      die "$node belongs to an active ZFS pool."
    fi
    node=${node##*/}
    holders=(/sys/class/block/"$node"/holders/*)
    btrfs_devices=(/sys/fs/btrfs/*/devices/"$node")
    if ((${#holders[@]} || ${#btrfs_devices[@]})); then
      die "$node belongs to an active volume."
    fi
  done
  jq -cS '.blockdevices[0] | del(.children, .mountpoints)' <<< "$info"
}

check_label() {
  local label=$1 matches device status=0
  matches=$(blkid -c /dev/null -o device -t "LABEL=$label") || status=$?
  [[ $status == 0 || $status == 2 ]] || die "Cannot check label $label."
  while IFS= read -r device; do
    [[ -z $device ]] && continue
    case "$device" in
      "$root_partition"|"$efi_partition"|"$swap_partition") ;;
      *) die "Filesystem label $label is already used by $device." ;;
    esac
  done <<< "$matches"
}

lsblk -d -e 7,11 -o PATH,SIZE,MODEL,SERIAL,TRAN,RO
ask 'Target disk path (for example /dev/nvme0n1): '
disk=$(realpath -e -- "$reply") || die "Unknown disk."
[[ $disk == /dev/* && -b $disk ]] || die "Select a block device under /dev."
fingerprint=$(check_disk)

ask 'Hostname [Default: nixos]: '
host=${reply:-nixos}
[[ $host =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] || die "Invalid hostname."
ask 'Administrator username [Default: operator]: '
user=${reply:-operator}
[[ $user =~ ^[a-z_][a-z0-9_-]{0,30}$ && $user != root ]] || die "Invalid username."
pick_timezone

printf '%s\n' '0) Headless' '1) GNOME' '2) KDE Plasma' '3) Xfce'
ask 'Environment [Default: 0]: '
case ${reply:-0} in
  0) desktop=Headless; desktop_config='programs.yonix.package = yonix.packages.x86_64-linux.yonix-headless;' ;;
  1) desktop=GNOME; desktop_config='services.displayManager.gdm.enable = true; services.desktopManager.gnome.enable = true;' ;;
  2) desktop='KDE Plasma'; desktop_config='services.displayManager.sddm.enable = true; services.desktopManager.plasma6.enable = true;' ;;
  3) desktop=Xfce; desktop_config='services.xserver = { enable = true; displayManager.lightdm.enable = true; desktopManager.xfce.enable = true; };' ;;
  *) die "Choose an environment from 0 to 3." ;;
esac

printf '%s\n' '1) ext4' '2) Btrfs'
ask 'Root filesystem [Default: 1]: '
case ${reply:-1} in
  1) root_fs=ext4; root_mkfs=mkfs.ext4 ;;
  2) root_fs=btrfs; root_mkfs=mkfs.btrfs ;;
  *) die "Choose a root filesystem from 1 to 2." ;;
esac

ask 'Root filesystem label [Default: NIXOS]: '
root_label=${reply:-NIXOS}
ask 'EFI filesystem label [Default: NIXBOOT]: '
efi_label=${reply:-NIXBOOT}
[[ $root_label =~ ^[A-Z0-9][A-Z0-9_-]{0,15}$ && $efi_label =~ ^[A-Z0-9][A-Z0-9_-]{0,10}$ &&
   $root_label != "$efi_label" ]] || die "Use distinct uppercase labels (root up to 16 characters, EFI up to 11)."
root_partition=
efi_partition=
swap_partition=
swap_label=
swap_config=
swap_description=None
root_number=2

printf '%s\n' '1) No swap' '2) zram (compressed swap in RAM)' '3) Dedicated swap partition'
ask 'Swap [Default: 1]: '
case ${reply:-1} in
  1) ;;
  2)
    swap_description=zram
    swap_config='zramSwap.enable = true;'
    ;;
  3)
    disk_bytes=$(jq -er '.size' <<< "$fingerprint")
    max_swap=$((disk_bytes / (1024 * 1024 * 1024) - 16))
    ((max_swap >= 1)) || die "The disk is too small for a separate swap partition."
    default_swap=$((max_swap < 4 ? max_swap : 4))
    ask "Swap partition size in GiB [Default: $default_swap] (max $max_swap): "
    swap_gib=${reply:-$default_swap}
    [[ $swap_gib =~ ^[1-9][0-9]{0,8}$ ]] || die "Enter a positive whole number of GiB."
    ((swap_gib <= max_swap)) || die "Swap may use at most $max_swap GiB."
    ask 'Swap partition label [Default: NIXSWAP]: '
    swap_label=${reply:-NIXSWAP}
    [[ $swap_label =~ ^[A-Z0-9][A-Z0-9_-]{0,14}$ && $swap_label != "$root_label" &&
       $swap_label != "$efi_label" ]] || die "Use a distinct uppercase swap label of up to 15 characters."
    swap_description="$swap_gib GiB partition ($swap_label)"
    swap_config="swapDevices = [ { label = \"$swap_label\"; } ];"
    root_number=3
    ;;
  *) die "Choose a swap mode from 1 to 3." ;;
esac

check_labels() {
  check_label "$root_label"
  check_label "$efi_label"
  if [[ -n $swap_label ]]; then check_label "$swap_label"; fi
}
check_labels

IFS= read -rs -p "Password for $user: " password || die "Input ended."
printf '\n'
IFS= read -rs -p "Repeat password: " repeat_password || die "Input ended."
printf '\n'
[[ -n $password && $password == "$repeat_password" ]] || die "Passwords are empty or differ."
password_hash=$(printf '%s\n' "$password" | openssl passwd -6 -stdin)
unset password repeat_password

printf '\n%s\n' "Disk: $disk (all existing partitions and data will be erased)" \
  "System: $host · $desktop · $timezone" "Administrator: $user" \
  "Filesystems: $root_fs ($root_label), vfat ($efi_label)" "Swap: $swap_description" \
  "Yonix: switch, rollback and clean enabled for $user and holders of the local API token."
ask "Type ERASE $disk to continue: "
[[ $reply == "ERASE $disk" ]] || die "Installation cancelled."

work=$(mktemp -d /run/yonix-install.XXXXXXXX)
target=$work/target
cleanup() {
  if mountpoint -q "$target"; then
    umount -R "$target" || { printf 'Could not unmount %s\n' "$target" >&2; return 1; }
  fi
  rm -f -- "$work/"{flake.nix,flake.lock,configuration.nix,hardware-configuration.nix}
  if [[ -d $target ]]; then rmdir "$target"; fi
  rmdir "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cat > "$work/flake.nix" <<CONFIG
{
  inputs.nixpkgs.url = "$NIXPKGS_FLAKE_URL";
  inputs.yonix.url = "$YONIX_FLAKE_URL";
  inputs.yonix.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { nixpkgs, yonix, ... }: {
    nixosConfigurations."$host" = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      specialArgs = { inherit yonix; };
      modules = [ yonix.nixosModules.default ./configuration.nix ];
    };
  };
}
CONFIG
cat > "$work/configuration.nix" <<CONFIG
{ pkgs, yonix, ... }:
{
  imports = [ ./hardware-configuration.nix ];
  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  fileSystems."/" = { device = "/dev/disk/by-label/$root_label"; fsType = "$root_fs"; };
  fileSystems."/boot" = {
    device = "/dev/disk/by-label/$efi_label";
    fsType = "vfat";
    options = [ "umask=0077" ];
  };

  networking.hostName = "$host";
  networking.networkmanager.enable = true;
  time.timeZone = "$timezone";
  i18n.defaultLocale = "en_US.UTF-8";

  users.users."$user" = { isNormalUser = true; extraGroups = [ "wheel" "networkmanager" "yonix" ]; };
  environment.systemPackages = [ pkgs.nano ];
  programs.yonix = {
    enable = true;
    configPath = "/etc/nixos";
    sourceMode = "path";
    allowSystemChanges = true;
  };
  $desktop_config
  $swap_config
  system.stateVersion = "26.05";
}
CONFIG

# Catch target configuration errors before touching the disk. Real hardware settings are generated below.
printf '{}\n' > "$work/hardware-configuration.nix"
export NIX_CONFIG="${NIX_CONFIG-}"$'\nextra-experimental-features = nix-command flakes'
nix flake lock "path:$work"
nix eval --raw "path:$work#nixosConfigurations.$host.config.system.build.toplevel.drvPath" >/dev/null
# nixos-install requires the mount point and every parent to be traversable.
chmod 0755 "$work"
mkdir "$target"

[[ $(check_disk) == "$fingerprint" ]] || die "The selected disk changed."
check_labels
{
  printf '%s\n' 'label: gpt' 'size=1GiB, type=uefi'
  if [[ -n $swap_label ]]; then printf 'size=%sGiB, type=swap\n' "$swap_gib"; fi
  printf '%s\n' 'type=linux'
} | sfdisk --lock=yes --wipe always --wipe-partitions always "$disk"
udevadm settle
[[ $(check_disk) == "$fingerprint" ]] || die "The selected disk changed."
partitions=$(lsblk --json --tree --paths --output PATH,TYPE,PARTN -- "$disk")

partition_path() {
  jq -er --argjson number "$1" '
    [.blockdevices[0].children[] | select(.type == "part" and .partn == $number) | .path]
    | if length == 1 then .[0] else error("missing partition \($number)") end
  ' <<< "$partitions"
}

efi_partition=$(partition_path 1)
root_partition=$(partition_path "$root_number")
if [[ -n $swap_label ]]; then
  swap_partition=$(partition_path 2)
  mkswap -L "$swap_label" "$swap_partition"
fi
mkfs.fat -F 32 -n "$efi_label" "$efi_partition"
"$root_mkfs" -L "$root_label" "$root_partition"
udevadm settle
check_labels
mount "$root_partition" "$target"
mkdir "$target/boot"
mount -o umask=0077 "$efi_partition" "$target/boot"

nixos-generate-config --root "$target" --no-filesystems
config_dir=$target/etc/nixos
mv "$work/"{flake.nix,flake.lock,configuration.nix} "$config_dir/"

nixos-install --root "$target" --flake "path:$config_dir#$host" --no-root-passwd --no-channel-copy
printf '%s:%s\n' "$user" "$password_hash" |
  nixos-enter --root "$target" -- /nix/var/nix/profiles/system/sw/bin/chpasswd -e
unset password_hash
# Resolve the installed account's numeric UID and GID inside the target system.
target_id=/nix/var/nix/profiles/system/sw/bin/id
uid=$(nixos-enter --root "$target" -- "$target_id" -u "$user")
gid=$(nixos-enter --root "$target" -- "$target_id" -g "$user")
chown -R "$uid:$gid" "$config_dir"
cleanup
trap - EXIT
printf '\nInstallation complete. Reboot and remove the live ISO.\n'
