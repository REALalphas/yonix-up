# Yonix hosts and shared builds

One Yonix panel can manage the local host and peers added through the panel or Nix configuration. The browser connects only to the panel's gateway. The gateway reads each peer's private credentials and forwards a bounded set of status, job and log requests. Connections are single-hop: peers do not inherit each other's peer lists. A disconnected peer remains selected and reports an error; no operation falls back to the gateway.

The selector applies to the web panel. CLI and the native GTK window continue to manage their configured local instance.

## Connect a host from the panel

Open **Connections** in the sidebar (or the bottom navigation on a small screen). To pair two hosts:

1. Open **Add connection** on either host. It shows six compact characters. **Copy code** copies all six as one value.
2. On the other host, open **Add connection → Add another instance**, enter or paste the code, and choose **Accept and pair**. Both hosts save a connection. The code works once and expires after five minutes. The sharing screen shows the remaining time.
3. For a routed network or IPv6, copy the full invitation from **Advanced** and paste it into the code fields on the other host. The invitation carries the address and port. Both daemons need a reachable return path.

Code-only discovery uses IPv4 multicast on the same trusted LAN (UDP 9847). Each daemon also needs its configured TCP port open. The NixOS module opens both ports and binds the daemon to all IPv4 interfaces by default. A loopback-only daemon can pair with another instance on the same machine. The new pairing handshake uses SPAKE2 and ChaCha20-Poly1305 to protect the credential exchange. Subsequent management requests still use HTTP over the trusted LAN, VPN or tunnel.

The paired host immediately appears in **Managed host**. Panel-created connections and their credentials are saved in `connections.json` inside the private state directory (`/var/lib/yonix` with the NixOS module), with mode `0600`. This runtime state survives package rebuilds and daemon restarts; treat its backups as secrets.

The list automatically refreshes reachability every 15 seconds and offers another check or removal with confirmation. The host selector, status and history refresh every 1.5 seconds. The gateway sign-in token is saved separately in the browser for this origin; disconnecting in Maintenance or receiving a 401 removes it. Removing a connection revokes routing through this gateway; it does not stop jobs already running remotely. Connections defined by Nix are also listed, marked **Managed by Nix**, and edited in the configuration. They take precedence over panel-created entries with the same ID. The gateway supports at most 32 connections in total. No restart or Nix rebuild is needed for panel-created connections.

## Connect web to the nas panel

Both hosts need this Yonix release. Use concrete private IPv4 addresses over a trusted LAN/VPN, or an SSH tunnel bound to loopback. HTTP API endpoints carry administrator tokens, so route Internet connections through an encrypted VPN or SSH tunnel.

On web:

```nix
programs.yonix = {
  enable = true;
  user = "operator";
  configPath = "/home/operator/nixos";
  listenAddress = "10.0.0.20";
  openFirewall = true;
  allowSystemChanges = true;
};
```

On nas:

```nix
programs.yonix = {
  enable = true;
  user = "operator";
  configPath = "/home/operator/nixos";
  peers.web = {
    label = "Web server";
    url = "http://10.0.0.20:9847";
    tokenFile = "/home/operator/.local/state/yonix/peers/web-token";
  };
};
```

Place web's API token in that runtime file on nas, readable by `operator`, mode `0600` or `0400`. For example, as operator on nas, using an existing SSH connection to operator on web:

```sh
umask 077
mkdir -p ~/.local/state/yonix/peers
ssh operator@web 'yonix token' > ~/.local/state/yonix/peers/web-token
chmod 600 ~/.local/state/yonix/peers/web-token
```

A secret manager can provision the same file instead. `tokenFile` is a quoted absolute string; do not use a Nix path literal or embed its contents in a Nix expression. Tokens read from configured secret files are never returned to the browser or included in generated Nix settings, the hosts API, or localStorage. When adding a connection through the panel, the administrator supplies its token explicitly; the browser sends it only to this gateway. Possession of the nas token authorizes management of its configured peers. Restrict that gateway to the same administrators who manage those peers.

Rebuild the two NixOS configurations. The module preserves an active daemon during rebuilds to avoid interrupting activation. When each instance has no active job, explicitly restart `yonix.service` to load new peer settings. Sign in to nas and select **Managed host → Web server**. The target name also appears in operation confirmations. Jobs keep running on their original host when you switch views.

If the remote token is rotated, replace the file on nas; it is read on each request. Revoke gateway access by removing the peer and restarting the idle gateway. No network discovery or automatic trust is performed.

## Share a signed binary cache

Yonix configures [Nix's signed HTTP binary cache](https://nix.dev/tutorials/nixos/binary-cache-setup.html). Every enabled cache serves builds already present in its local Nix store. Other hosts request matching store objects automatically during builds. Their usual upstream caches remain configured.

On nas:

```nix
programs.yonix.cache = {
  enable = true;
  listenAddress = "10.0.0.10";
  openFirewall = true;
  # port = 9848;
};
```

On first activation, the module generates a signing key pair under `/var/lib/yonix-cache`. The private key stays root-owned with mode `0600`; systemd passes it to nix-serve as a credential. Rebuilds and service restarts preserve this identity. Back up this directory if peers must keep trusting the same cache after reinstalling nas.

Get the **public** key from the cache host's panel settings, or:

```sh
cat /var/lib/yonix-cache/public-key
```

Then pin that key on web:

```nix
programs.yonix.caches.nas = {
  url = "http://10.0.0.10:9848";
  publicKey = "nas-yonix:REPLACE_WITH_THE_ACTUAL_PUBLIC_KEY";
};
```

Repeat the publisher/client settings in the opposite direction for bidirectional reuse. Pin keys only after verifying them on the intended host. Cache signing keys and management API tokens are independent. The cache endpoint provides read access to store contents; restrict its network access accordingly.

Rebuild once to configure the relationship. Subsequent Nix builds automatically check the cache for matching paths. This is on-demand substitution: availability follows the publisher's local store and garbage collection. Source/lock inputs and architecture determine reuse. A cache does not make different configurations identical. Cache failures remain ordinary Nix substitution failures, with the normal local/upstream fallback.

## Populate a cache from GitHub Actions

GitHub Actions' own cache is scoped to workflow runs and is not a Nix
substituter that an installed NixOS client can query. Use a Nix binary-cache
service, such as Cachix or a self-hosted Attic/Harmonia instance, and let a
GitHub Actions job build the exact NixOS system closure and publish it there.

The Yonix NixOS module already trusts the project cache:

```nix
programs.yonix.enable = true;
```

It adds `https://yonix.cachix.org` and its signing key to Nix. Set
`programs.yonix.useOfficialCache = false` to opt out. Standalone Nix clients can
add the same URL and key manually:

```nix
nix.settings.extra-substituters = [ "https://yonix.cachix.org" ];
nix.settings.extra-trusted-public-keys = [
  "yonix.cachix.org-1:tuJfS1bMNuVctofrHgbmUcfYcqLfjieOmOI42ccjaqg="
];
```

The repository includes a publishing workflow at
`.github/workflows/binary-cache.yml`. Set the `CACHIX_CACHE` repository
variable and `CACHIX_AUTH_TOKEN` repository secret before enabling it. The
workflow waits for the `CI` workflow to pass on a push to `main`, then builds
the public `.#default` and `.#yonix-headless` packages and pushes their
closures. To
publish a system configuration instead, change the build target to the same
flake revision and `nixosConfigurations.<host>.config.system.build.toplevel`
target used by the client:

```yaml
- uses: cachix/install-nix-action@v31
- uses: cachix/cachix-action@v15
  with:
    name: ${{ vars.CACHIX_CACHE }}
    authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}
- run: >-
    nix build ".#nixosConfigurations.${{ vars.NIXOS_HOST }}.config.system.build.toplevel"
    --no-link --print-out-paths | cachix push "${{ vars.CACHIX_CACHE }}"
```

After the first successful publish, Nix fetches matching package paths from the
cache before compiling locally. When the workflow targets a system
configuration, the same mechanism applies to that system closure. GitHub's
runner performs the expensive build while the client downloads the result.

This version does not synchronize configuration repositories, schedule updates, or automatically activate systems. Build/check/apply actions are initiated in the selected host's panel.

## Verification

- `cargo test --no-default-features`: real gateway/peer daemon processes; routing, logs, confirmations, credential permissions/rotation, unknown peers and disconnects.
- `npm test`: translations, host URL scoping, policy and API requests.
- `nix build .#checks.x86_64-linux.module`: NixOS peer and signed-cache options.
- `nix build .#checks.x86_64-linux.peers`: two VMs; nas→web management, stable signing identity, rejection of an untrusted cache key, and reuse of a signed build on web where local building is deliberately disabled for the fixture.
