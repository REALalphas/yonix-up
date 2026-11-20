# Install from a NixOS live ISO

Boot the NixOS installer in x86-64 UEFI mode, connect to the Internet, and run in a terminal:

```sh
sudo nix --extra-experimental-features 'nix-command flakes' run github:REALalphas/yonix-up#install
```

The installer asks for the target disk, hostname, administrator username and password, time zone, environment (0 for Headless by default, 1 for GNOME, 2 for KDE Plasma, or 3 for Xfce), and root filesystem (ext4 or Btrfs). It creates a GPT disk with the selected root filesystem and a FAT32 EFI partition. Both are mounted by filesystem label, with customizable defaults of `NIXOS` and `NIXBOOT`. The target must be at least 16 GiB. Confirm the displayed disk path to erase **all data on that disk**.

At the time zone prompt, press Enter for UTC, enter a name such as `Europe/Moscow`, or search by city or region and pick a numbered match.

For swap, choose no swap, zram using the NixOS zram generator, or a dedicated disk partition. For a disk partition, enter a size and swap label (default `NIXSWAP`). The installer reserves 16 GiB of disk capacity for EFI, root, and partition metadata.

The new system starts Yonix locally and grants the chosen administrator and the local Yonix API token holder permission to switch, roll back, and clean. The administrator owns the editable flake under `/etc/nixos`. Headless installations use the headless Yonix package.

To build a minimal installation ISO with the command already included, run `nix build .#installer-iso` from an x86-64 checkout. Boot the image under `result/iso/` and run `sudo yonix-install`. The installation uses an Internet connection to fetch inputs and system packages.
