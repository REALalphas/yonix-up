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
      listenAddress = "0.0.0.0";
    };
    environment.systemPackages = [
      pkgs.curl
      pkgs.git
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
    virtualisation.memorySize = 1536;
  };
  testScript = ''
    import json
    start_all()
    machine.wait_for_unit("yonix.service")
    machine.wait_for_open_port(9847)
    machine.succeed("install -d -o operator -g users /home/operator/nixos")
    machine.succeed("install -o operator -g users /etc/yonix-test-flake.nix /home/operator/nixos/flake.nix")
    # Reproduce the original yonix checkout: the flake imports a Git-ignored config.nix.
    machine.succeed("printf '%s\\n' '{ name = \"yonix-test-system\"; }' > /home/operator/nixos/config.nix")
    machine.succeed("echo config.nix > /home/operator/nixos/.gitignore")
    machine.succeed("chown operator:users /home/operator/nixos/config.nix /home/operator/nixos/.gitignore")
    machine.succeed("su - operator -c 'cd /home/operator/nixos && git init && git add flake.nix .gitignore && nix flake lock path:.'")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert status["host"] == "test"
    assert status["system_changes_enabled"]
    machine.succeed("test $(stat -c %a /var/lib/yonix/token) = 600")
    machine.fail("su - nobody -s /bin/sh -c 'cat /var/lib/yonix/token'")
    machine.succeed("test $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9847/api/status) = 401")
    machine.succeed("su - operator -c 'yonix check'")
    machine.succeed("su - operator -c 'yonix build'")
    machine.succeed("test $(cat /var/lib/yonix/result) = tested")
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
    machine.succeed("su - operator -c 'yonix switch --yes'")
    switch_id = json.loads(machine.succeed("su - operator -c 'yonix history'"))[0]["id"]
    assert "Btrfs maintenance (switch)" in machine.succeed("su - operator -c 'yonix logs " + switch_id + "'")
    machine.succeed("test -e /run/yonix-bootloader-called")
    machine.succeed("su - operator -c 'yonix rollback --yes'")
    machine.succeed("su - operator -c 'yonix clean --yes'")
    # Clean runs the retention GC (like the original yonix minus its
    # destructive scratch/journals/prune steps); the log proves it ran.
    clean_id = json.loads(machine.succeed("su - operator -c 'yonix history'"))[0]["id"]
    assert "nix-collect-garbage --delete-older-than" in machine.succeed("su - operator -c 'yonix logs " + clean_id + "'")
    assert "Btrfs maintenance (clean)" in machine.succeed("su - operator -c 'yonix logs " + clean_id + "'")
    # Legacy mode: only configuration.nix is present. Check/build would need
    # an evaluatable legacy system (NIX_PATH/channels), which this offline VM
    # cannot provide; that path is covered by the workflow test with fake
    # binaries. Here assert detection, the flake-lock refusal, and the
    # config-independent actions (rollback/clean use the same sudo helper).
    machine.succeed("mv /home/operator/nixos/flake.nix /tmp/yonix-flake-backup")
    machine.succeed("printf '%s\\n' '{ }' > /home/operator/nixos/configuration.nix")
    machine.succeed("chown operator:users /home/operator/nixos/configuration.nix")
    status = json.loads(machine.succeed("su - operator -c 'yonix status'"))
    assert status["config_kind"] == "legacy", status
    machine.fail("su - operator -c 'yonix update-inputs --yes'")
    machine.succeed("su - operator -c 'yonix rollback --yes'")
    machine.succeed("su - operator -c 'yonix clean --yes'")
    machine.fail("su - operator -c 'sudo -n id'")
    machine.succeed("systemctl restart yonix")
    machine.wait_for_unit("yonix.service")
    machine.wait_for_open_port(9847)
    history = json.loads(machine.succeed("su - operator -c 'yonix history'"))
    assert len(history) == 8
    assert all(job["status"] == "succeeded" for job in history)
  '';
}
