{ pkgs }:
let
  maintenance = pkgs.callPackage ./btrfs-maintenance.nix { };
in
pkgs.testers.runNixOSTest {
  name = "yonix-btrfs-maintenance";
  nodes.machine = {
    boot.supportedFilesystems = [ "btrfs" ];
    virtualisation.emptyDiskImages = [ 4096 ];
    virtualisation.memorySize = 768;
    environment.systemPackages = [ pkgs.btrfs-progs ];
  };
  testScript = ''
    import re
    start_all()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("mkfs.btrfs -f /dev/vdb")
    machine.succeed("mkdir -p /mnt/btrfs /mnt/alias /mnt/readonly")
    machine.succeed("mount /dev/vdb /mnt/btrfs")
    machine.succeed("btrfs subvolume create /mnt/btrfs/alias")
    machine.succeed("mount -o subvol=alias /dev/vdb /mnt/alias")
    machine.succeed("mount -o ro,subvol=alias /dev/vdb /mnt/readonly")
    # Keep actual file data across maintenance, not just empty filesystems.
    machine.succeed("dd if=/dev/urandom of=/mnt/btrfs/keep bs=1M count=64 status=none")
    checksum = machine.succeed("sha256sum /mnt/btrfs/keep").strip()
    for mode in ["switch", "clean"]:
        output = machine.succeed("${maintenance} " + mode)
        assert "reserve target" in output, output
        assert output.count("$ btrfs balance start -dusage=0 ") == 1, output
        assert "/mnt/readonly" not in output, output
        assert machine.succeed("sha256sum /mnt/btrfs/keep").strip() == checksum
    # Manufacture low allocation reserve on a disposable filesystem. Keeping
    # alternating files makes data chunks partly occupied after deletion.
    machine.succeed("for i in $(seq 1 80); do dd if=/dev/zero of=/mnt/btrfs/fill-$i bs=1M count=64 status=none 2>/dev/null || break; done")
    machine.succeed("for i in $(seq 1 2 80); do rm -f /mnt/btrfs/fill-$i; done; btrfs filesystem sync /mnt/btrfs")
    before = machine.succeed("btrfs filesystem usage --raw /mnt/btrfs")
    size_match = re.search(r"Device size:\s+(\d+)", before)
    unallocated_match = re.search(r"Device unallocated:\s+(\d+)", before)
    assert size_match is not None and unallocated_match is not None, before
    size = int(size_match.group(1))
    unallocated = int(unallocated_match.group(1))
    assert unallocated < size // 4, before
    output = machine.succeed("${maintenance} clean")
    assert machine.succeed("sha256sum /mnt/btrfs/keep").strip() == checksum
    assert "recovering allocation space" in output, output
    machine.log(before)
    machine.log(output)
  '';
}
