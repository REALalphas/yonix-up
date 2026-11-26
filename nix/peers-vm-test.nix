{
  self,
  pkgs,
  system,
}:
pkgs.testers.runNixOSTest {
  name = "yonix-peers-and-cache";
  defaults = { lib, ... }: {
    imports = [ self.nixosModules.default ];
    programs.yonix = {
      enable = true;
      package = self.packages.${system}.yonix-headless;
      configPath = "/etc/nixos";
      openFirewall = true;
    };
    environment.etc."cache-fixture.nix".text = ''
      builtins.derivation {
        name = "yonix-shared-cache-fixture";
        system = "${system}";
        builder = "${pkgs.pkgsStatic.busybox}/bin/busybox";
        args = [ "sh" "-ec" "test -e /run/allow-local-build; echo shared-build > $out" ];
      }
    '';
    virtualisation.additionalPaths = [ pkgs.pkgsStatic.busybox ];
    virtualisation.memorySize = 1536;
    # No external cache or Internet; a fallback local build fails on web.
    nix.settings.substituters = lib.mkForce [ ];
    nix.settings.sandbox = false;
  };
  nodes.nas = {
    networking.firewall.allowedTCPPorts = [ 9851 ];
    environment.systemPackages = [ pkgs.jq ];
    programs.yonix = {
      listenAddress = "192.168.1.1";
      peers.web = {
        url = "http://192.168.1.2:9847";
        tokenFile = "/var/lib/yonix/web-token";
      };
      cache = {
        enable = true;
        openFirewall = true;
      };
    };
  };
  nodes.web = {
    programs.yonix = {
      listenAddress = "192.168.1.2";
      caches.nas = {
        url = "http://192.168.1.1:9848";
        # Deliberately untrusted fixture key. The test pins nas's generated key explicitly.
        publicKey = "untrusted-test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
      };
    };
  };
  testScript = ''
    import json, shlex
    start_all()
    nas.wait_for_unit("yonix.service")
    web.wait_for_unit("yonix.service")
    nas.wait_for_unit("nix-serve.service")
    nas.wait_for_open_port(9848, addr="192.168.1.1")
    nas.wait_for_open_port(9847, addr="192.168.1.1")
    web.wait_for_open_port(9847, addr="192.168.1.2")
    web_token = web.succeed("cat /var/lib/yonix/token").strip()
    nas.succeed("install -o yonix -g yonix -m 600 /dev/null /var/lib/yonix/web-token")
    nas.succeed("printf %s " + shlex.quote(web_token) + " > /var/lib/yonix/web-token")
    auth = '-H "Authorization: Bearer $(cat /var/lib/yonix/token)"'
    status = json.loads(nas.succeed("curl -fsS " + auth + " http://192.168.1.1:9847/api/hosts/web/status"))
    assert status["host"] == "web", status
    assert "token_file" not in nas.succeed("curl -fsS " + auth + " http://192.168.1.1:9847/api/hosts")
    # Exercise code-only pairing through both NixOS firewalls. Use an
    # independent daemon on nas so its declarative web peer stays separate.
    nas.succeed("jq '.port = 9851 | .peers = [] | .host = \"pair-client\" | .state_dir = \"/run/yonix-pair-test\"' /etc/yonix/config.json >/tmp/pair-client.json")
    nas.succeed("systemd-run --unit=yonix-pair-test /run/current-system/sw/bin/yonix --config /tmp/pair-client.json serve")
    nas.wait_for_open_port(9851, addr="192.168.1.1")
    code = json.loads(web.succeed("yonix pair --json"))["code"]
    nas.succeed("yonix --config /tmp/pair-client.json pair-accept " + shlex.quote(code))
    paired = json.loads(web.succeed("curl -fsS " + auth + " http://192.168.1.2:9847/api/connections"))
    client_id = next(entry["id"] for entry in paired if entry["url"] == "http://192.168.1.1:9851")
    client = json.loads(web.succeed("curl -fsS " + auth + " http://192.168.1.2:9847/api/hosts/" + client_id + "/status"))
    assert client["host"] == "pair-client"
    nas.succeed("touch /run/allow-local-build")
    path = nas.succeed("nix-build /etc/cache-fixture.nix --no-out-link").strip()
    pub = nas.succeed("cat /var/lib/yonix-cache/public-key").strip()
    nas.succeed("test $(stat -c %a /var/lib/yonix-cache/secret-key) = 600")
    nas.succeed("systemctl restart yonix-cache-key")
    assert nas.succeed("cat /var/lib/yonix-cache/public-key").strip() == pub
    web.fail("nix-store --check-validity " + shlex.quote(path))
    web.fail("nix-build /etc/cache-fixture.nix --no-out-link")
    # Signed substitution succeeds although this machine cannot execute the fixture builder.
    copied = web.succeed("nix-build /etc/cache-fixture.nix --no-out-link --option extra-trusted-public-keys " + shlex.quote(pub)).strip()
    assert copied == path
    web.succeed("grep -q shared-build " + shlex.quote(path))
    web.succeed("systemctl stop yonix")
    nas.succeed("test $(curl -s -o /dev/null -w '%{http_code}' " + auth + " http://192.168.1.1:9847/api/hosts/web/status) = 502")
    local_status = json.loads(nas.succeed("curl -fsS " + auth + " http://192.168.1.1:9847/api/hosts/local/status"))
    assert local_status["host"] == "nas"
  '';
}
