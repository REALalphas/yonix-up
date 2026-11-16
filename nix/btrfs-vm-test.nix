{ pkgs }:
let
  maintenance = pkgs.callPackage ./btrfs-maintenance.nix { };
  # Force the production queue through multiple process lifetimes on 4 GiB.
  limitedMaintenance = pkgs.callPackage ./btrfs-maintenance.nix { cleanLimit = 1; };
  inspect = pkgs.writeScript "inspect-block-groups" ''
    #!${pkgs.python3}/bin/python3
    import importlib.util, json, sys
    spec = importlib.util.spec_from_file_location("maintenance", "${./btrfs-maintenance.py}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    print(json.dumps([vars(g) for g in module.block_groups("/mnt/btrfs")]))
  '';
in
pkgs.testers.runNixOSTest {
  name = "yonix-btrfs-maintenance";
  nodes.machine = {
    boot.supportedFilesystems = [ "btrfs" ];
    virtualisation.emptyDiskImages = [ 4096 ];
    virtualisation.memorySize = 1024;
    environment.systemPackages = [ pkgs.btrfs-progs ];
  };
  testScript = ''
    import json
    import re
    start_all()
    machine.wait_for_unit("multi-user.target")

    def groups():
        return json.loads(machine.succeed("${inspect}"))

    def unallocated():
        output = machine.succeed("btrfs filesystem usage --raw /mnt/btrfs")
        match = re.search(r"Device unallocated:\s+(\d+)", output)
        assert match is not None, output
        return int(match.group(1))

    # Exercise both the dedicated block-group tree and legacy extent tree.
    for feature in ["block-group-tree", "^block-group-tree"]:
        machine.succeed("mkfs.btrfs -f -O " + feature + " /dev/vdb")
        machine.succeed("mkdir -p /mnt/btrfs /mnt/alias /mnt/readonly")
        machine.succeed("mount /dev/vdb /mnt/btrfs")
        machine.succeed("btrfs subvolume create /mnt/btrfs/alias")
        machine.succeed("mount -o subvol=alias /dev/vdb /mnt/alias")
        machine.succeed("mount -o ro,subvol=alias /dev/vdb /mnt/readonly")
        machine.succeed("rm -rf /var/lib/yonix-btrfs-maintenance")
        machine.succeed("dd if=/dev/urandom of=/mnt/btrfs/keep bs=1M count=64 status=none")
        machine.succeed("btrfs filesystem sync /mnt/btrfs")
        checksum = machine.succeed("sha256sum /mnt/btrfs/keep").strip()
        inventory = groups()
        assert any(g["flags"] & 1 and g["used"] > 0 for g in inventory), inventory
        assert any(g["flags"] & 4 for g in inventory), inventory
        output = machine.succeed("${maintenance} switch")
        assert "reserve target" in output, output
        assert output.count("$ btrfs balance start -dusage=0 ") == 1, output
        assert "/mnt/readonly" not in output and "/mnt/alias" not in output, output
        assert machine.succeed("sha256sum /mnt/btrfs/keep").strip() == checksum

        # Manufacture several partly occupied data chunks and a low reserve.
        machine.succeed("for i in $(seq 1 80); do dd if=/dev/zero of=/mnt/btrfs/fill-$i bs=1M count=64 status=none 2>/dev/null || break; done")
        machine.succeed("for i in $(seq 1 2 80); do rm -f /mnt/btrfs/fill-$i; done; btrfs filesystem sync /mnt/btrfs")
        machine.succeed("sha256sum /mnt/btrfs/keep /mnt/btrfs/fill-* > /tmp/survivors.sha256")
        machine.succeed("ln -s /mnt/btrfs/keep /nix/var/nix/gcroots/yonix-test-keep")
        profile_before = sorted(g["flags"] for g in groups() if g["flags"] & 1)[0]
        before = unallocated()
        assert before < 1024**3, before
        # Repeated bounded runs must converge, including after reserve is met.
        total_attempts = 0
        saw_queued = False
        for attempt in range(40):
            status, output = machine.execute("${limitedMaintenance} clean")
            machine.log(output)
            assert status == 0 or "allocation reserve remains low" in output, output
            match = re.search(r"data attempts=(\d+)", output)
            assert match is not None, output
            total_attempts += int(match.group(1))
            assert int(match.group(1)) <= 1, output
            saw_queued = saw_queued or "queued groups remain" in output
            machine.succeed("sha256sum -c /tmp/survivors.sha256")
            if "nothing" in output:
                terminal_status = status
                break
        else:
            raise AssertionError("maintenance did not converge within forty single-group runs")
        assert total_attempts > 1 and saw_queued and attempt > 0, output
        assert unallocated() > before, (before, unallocated())
        # No relocation churn on an unchanged, already visited layout.
        status, output = machine.execute("${maintenance} clean")
        assert status == terminal_status, output
        assert "data attempts=0" in output and "nothing" in output, output
        if status != 0:
            # Safe compaction can exhaust candidates below the reserve. That
            # must remain a failure rather than inventing reclaimed bytes.
            assert "allocation reserve remains low" in output, output
        assert "before=" in output and "after=" in output and "delta=" in output, output
        machine.succeed("sha256sum -c /tmp/survivors.sha256")
        usage = machine.succeed("btrfs filesystem usage /mnt/btrfs")
        assert "Metadata,DUP" in usage, usage
        assert all(g["flags"] == profile_before for g in groups() if g["flags"] & 1)
        machine.succeed("test $(readlink /nix/var/nix/gcroots/yonix-test-keep) = /mnt/btrfs/keep")
        machine.succeed("rm /nix/var/nix/gcroots/yonix-test-keep")
        machine.succeed("umount /mnt/readonly /mnt/alias /mnt/btrfs")
  '';
}
