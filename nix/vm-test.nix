{
  self,
  pkgs,
  system,
}:
pkgs.testers.runNixOSTest {
  name = "yonix-service";
  nodes.machine = { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default ];
    system.switch.enable = true;
    # QEMU boots this test directly from the kernel; exercise the install hook without writing a disk bootloader.
    boot.loader.grub.enable = lib.mkForce false;
    boot.loader.external = {
      enable = true;
      installHook = pkgs.writeShellScript "yonix-vm-bootloader" "${pkgs.coreutils}/bin/touch /run/yonix-bootloader-called";
    };
    users.users.operator = {
      isNormalUser = true;
    };
    programs.yonix = {
      enable = true;
      user = "operator";
      configPath = "/home/operator/nixos";
      host = "test";
      sourceMode = "path";
      package = self.packages.${system}.yonix-headless;
      allowSystemChanges = true;
    };
    environment.systemPackages = [
      pkgs.curl
      pkgs.git
      pkgs.acl
    ];
    environment.etc."yonix-test-flake.nix".text = ''
      {
        outputs = { self }: let fixture = import ./config.nix; in {
          nixosConfigurations.test.config.system.build.toplevel = builtins.derivation {
            name = fixture.name;
            system = "${system}";
            # A static builder keeps this fixture independent of nixpkgs inputs and dynamic loaders.
            builder = builtins.appendContext "${pkgs.pkgsStatic.busybox}/bin/busybox" {
              "${pkgs.pkgsStatic.busybox}" = { path = true; };
            };
            args = [ "sh" "-c" "echo tested > $out" ];
          };
        };
      }
    '';
    virtualisation.additionalPaths = [ pkgs.pkgsStatic.busybox ];
    nix.settings.substituters = lib.mkForce [ ];
    # Store optimisation hardlinks files: a local image avoids stale 9p file
    # handles when overlayfs copies up the shared host store during relinking.
    virtualisation.useNixStoreImage = true;
    virtualisation.writableStore = true;
    virtualisation.writableStoreUseTmpfs = false;
    virtualisation.diskSize = 4096;
    virtualisation.memorySize = 1536;
  };
  testScript = ''
    import json
    import shlex
    start_all()
    machine.wait_for_unit("yonix.service")
    machine.wait_for_open_port(9847)
    machine.succeed("install -d -m 0700 -o operator -g users /home/operator/nixos")
    machine.succeed("install -m 0600 -o operator -g users /etc/yonix-test-flake.nix /home/operator/nixos/flake.nix")
    # Reproduce the original yonix checkout: the flake imports a Git-ignored config.nix.
    machine.succeed("printf '%s\\n' '{ name = \"yonix-test-system\"; }' > /home/operator/nixos/config.nix")
    machine.succeed("echo config.nix > /home/operator/nixos/.gitignore")
    machine.succeed("chown operator:users /home/operator/nixos/config.nix /home/operator/nixos/.gitignore")
    machine.succeed("su - operator -c 'cd /home/operator/nixos && git init && git add flake.nix .gitignore && nix flake lock path:.'")
    # The checkout is created after boot, so apply the module's ACL rule now.
    machine.succeed("systemd-tmpfiles --create --prefix=/home/operator/nixos")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert status["config_kind"] == "unknown", status
    assert "Permission denied" in status["config_error"], status
    assert "Permission denied" in machine.fail("su - operator -c 'yonix update-inputs --yes' 2>&1"), status
    assert json.loads(machine.succeed("su - operator -c 'yonix history'")) == []
    # Grant only directory search on the private home.
    machine.succeed("setfacl -m u:yonix:--x /home/operator")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert status["host"] == "test"
    assert status["config_kind"] == "flake" and "config_error" not in status, status
    assert status["system_changes_enabled"]
    settings = json.loads(machine.succeed("cat /etc/yonix/config.json"))
    helper = shlex.quote(settings["privileged_helper"])
    machine.succeed("runuser -u yonix -- sudo -n -l -- " + helper + " switch")
    machine.fail("runuser -u yonix -- sudo -n id")
    # Simulate a running old-account daemon re-execing the new launcher after
    # activation has updated the unit. Restart must pick up the new credentials.
    fixture = "/run/yonix-upgrade-test"
    unit = "yonix-upgrade-test.service"
    def write_fixture(path, contents):
        machine.succeed("printf %s " + shlex.quote(contents) + " > " + path)

    machine.succeed("mkdir -p " + fixture + " /run/systemd/system/" + unit + ".d")
    upgrade_settings = dict(settings, port=9846, state_dir="/var/lib/yonix-upgrade-test")
    write_fixture(fixture + "/config.json", json.dumps(upgrade_settings))
    write_fixture(fixture + "/service-user", "operator\n")
    write_fixture(fixture + "/start", f"""#!${pkgs.runtimeShell}
    until [ -f {fixture}/exec ]; do ${pkgs.coreutils}/bin/sleep 0.1; done
    exec /run/current-system/sw/bin/yonix --config {fixture}/config.json serve
    """)
    machine.succeed("chmod 755 " + fixture + "/start")
    write_fixture("/run/systemd/system/" + unit, f"""[Service]
    User=operator
    StateDirectory=yonix-upgrade-test
    Environment=PATH=/run/current-system/sw/bin
    ExecStart={fixture}/start
    Restart=on-failure
    RestartSec=1
    """)
    machine.succeed("systemctl daemon-reload && systemctl start " + unit)
    old_pid = machine.succeed("systemctl show -p MainPID --value " + unit).strip()
    assert machine.succeed("ps -o user= -p " + old_pid).strip() == "operator"
    write_fixture("/run/systemd/system/" + unit + ".d/user.conf", "[Service]\nUser=yonix\nGroup=yonix\n")
    write_fixture(fixture + "/service-user", "yonix\n")
    machine.succeed("systemctl daemon-reload && touch " + fixture + "/exec")
    machine.wait_for_open_port(9846)
    new_pid = machine.succeed("systemctl show -p MainPID --value " + unit).strip()
    assert old_pid != new_pid and machine.succeed("ps -o user= -p " + new_pid).strip() == "yonix"
    upgrade_log = machine.succeed("journalctl -u " + unit + " -o cat")
    assert "running as operator after a service account change to yonix" in upgrade_log, upgrade_log
    machine.succeed("systemctl stop " + unit)
    machine.succeed("test $(stat -c %a /var/lib/yonix/token) = 640")
    machine.fail("su - nobody -s /bin/sh -c 'cat /var/lib/yonix/token'")
    machine.succeed("test $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9847/api/status) = 401")
    machine.succeed("su - operator -c 'yonix check'")
    machine.succeed("su - operator -c 'yonix build'")
    machine.succeed("test $(cat /var/lib/yonix/result) = tested")
    build_root = machine.succeed("readlink -f /var/lib/yonix/result").strip()
    machine.succeed("su - operator -c 'yonix diff'")
    compared = json.loads(machine.succeed("su - operator -c 'yonix history'"))[0]
    assert compared["action"] == "diff" and compared["status"] == "succeeded", compared
    assert "diff-closures" in machine.succeed("su - operator -c 'yonix logs " + compared["id"] + "'")
    # Exercise the real rebuild/activation path using this VM's existing system closure.
    # A separate profile entry gives rollback a valid previous generation without building another OS.
    current = machine.succeed("readlink -f /run/current-system").strip()
    machine.succeed("cp -a " + current + " /tmp/yonix-previous")
    previous = machine.succeed("nix-store --add /tmp/yonix-previous").strip()
    machine.succeed("nix-env --profile /nix/var/nix/profiles/system --set " + previous)
    def store_string(path):
        quoted = json.dumps(path)
        return "builtins.appendContext " + quoted + " { " + quoted + " = { path = true; }; }"
    activation_flake = "{ outputs = { self }: { nixosConfigurations.test.config.system.build = { toplevel = " + store_string(current) + "; nixos-rebuild = " + store_string("${pkgs.nixos-rebuild}") + "; }; }; }"
    machine.succeed("cat > /home/operator/nixos/flake.nix <<'YONIX_FLAKE'\n" + activation_flake + "\nYONIX_FLAKE")
    before_decline = json.loads(machine.succeed("su - operator -c 'yonix history'"))
    decline = machine.fail("su - operator -c 'yonix switch' </dev/null 2>&1")
    assert "Operation cancelled." in decline, decline
    assert json.loads(machine.succeed("su - operator -c 'yonix history'")) == before_decline
    machine.fail("test -e /run/yonix-bootloader-called")
    machine.succeed("su - operator -c 'yonix switch --yes'")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert not status["needs_restart"], status
    switch_id = json.loads(machine.succeed("su - operator -c 'yonix history'"))[0]["id"]
    assert "Btrfs maintenance (switch)" in machine.succeed("su - operator -c 'yonix logs " + switch_id + "'")
    machine.succeed("test -e /run/yonix-bootloader-called")
    machine.succeed("su - operator -c 'yonix rollback --yes'")
    machine.succeed("install -d /var/cache/yonix-clean-fixture /home/operator/.cache/yonix-clean-fixture")
    machine.succeed("printf 'd /var/cache/yonix-clean-fixture 0755 root root m:1d -\\n' > /etc/tmpfiles.d/yonix-clean-fixture.conf")
    machine.succeed("touch -d '3 days ago' /var/cache/yonix-clean-fixture/old /home/operator/.cache/yonix-clean-fixture/old")
    machine.succeed("touch /var/cache/yonix-clean-fixture/recent")
    def run_clean():
        # The VM disk is disposable. Avoid host flushes for every hardlink
        # while exercising the real Nix store optimisation.
        machine.succeed("mount -o remount,nobarrier /")
        machine.succeed("su - operator -c 'yonix clean --yes'")
    run_clean()
    machine.fail("test -e /var/cache/yonix-clean-fixture/old")
    machine.succeed("test -e /var/cache/yonix-clean-fixture/recent && test -e /home/operator/.cache/yonix-clean-fixture/old")
    # The real maintenance pipeline must be visible through the CLI job log.
    clean_id = json.loads(machine.succeed("su - operator -c 'yonix history'"))[0]["id"]
    assert "nix-collect-garbage --delete-older-than" in machine.succeed("su - operator -c 'yonix logs " + clean_id + "'")
    clean_log = machine.succeed("su - operator -c 'yonix logs " + clean_id + "'")
    for stage in ["Btrfs maintenance (clean)", "systemd-tmpfiles --clean", "--vacuum-time=14d", "store verify --all --no-trust", "nix-store --optimise", "fstrim --fstab", "Yonix clean summary:"]:
        assert stage in clean_log, stage
    machine.succeed("test $(readlink -f /var/lib/yonix/result) = " + build_root)
    machine.succeed("test $(cat /var/lib/yonix/result) = tested")
    # Legacy mode: only configuration.nix is present. Check/build would need
    # an evaluatable legacy system (NIX_PATH/channels), which this offline VM
    # cannot provide; that path is covered by the workflow test with fake
    # binaries. Here assert detection, the flake-lock refusal, and the
    # config-independent actions (rollback/clean use the same sudo helper).
    machine.succeed("mv /home/operator/nixos/flake.nix /tmp/yonix-flake-backup")
    machine.succeed("printf '%s\\n' '{ }' > /home/operator/nixos/configuration.nix")
    machine.succeed("chown operator:users /home/operator/nixos/configuration.nix")
    # NixOS activation reapplies the home mode, masking the traversal ACL.
    machine.succeed("setfacl -m u:yonix:--x /home/operator")
    machine.succeed("systemd-tmpfiles --create --prefix=/home/operator/nixos")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert status["config_kind"] == "legacy", status
    machine.fail("su - operator -c 'yonix update-inputs --yes'")
    # The first rollback selected generation 1. Select the newer system again
    # so this independent legacy rollback has an older generation to select.
    machine.succeed("nix-env --profile /nix/var/nix/profiles/system --set " + current)
    machine.succeed("su - operator -c 'yonix rollback --yes'")
    run_clean()
    machine.fail("su - operator -c 'sudo -n id'")
    before_restart = json.loads(machine.succeed("su - operator -c 'yonix history'"))
    machine.succeed("systemctl restart yonix")
    machine.wait_for_unit("yonix.service")
    machine.wait_for_open_port(9847)
    history = json.loads(machine.succeed("su - operator -c 'yonix history'"))
    assert history == before_restart, history
    assert "Yonix clean summary:" in machine.succeed("su - operator -c 'yonix logs " + clean_id + "'")
    assert "Re-executing from" not in machine.succeed("journalctl -u yonix.service -o cat")
    # Legacy update-inputs is rejected before a job is created.
    assert sorted(job["action"] for job in history) == sorted([
        "check", "build", "diff", "switch", "rollback", "clean", "rollback", "clean"
    ]), history
    assert all(job["status"] == "succeeded" for job in history)
  '';
}
