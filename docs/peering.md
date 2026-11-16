# Yonix hosts and shared builds

One Yonix panel can manage the local host and peers added through the panel or Nix configuration. The browser connects only to the panel's gateway. The gateway reads each peer's private credentials and forwards a bounded set of status, job and log requests. Connections are single-hop: peers do not inherit each other's peer lists. A disconnected peer remains selected and reports an error; no operation falls back to the gateway.

The selector applies to the web panel. CLI and the native GTK window continue to manage their configured local instance.

## Connect a host from the panel

Open **Connections** in the sidebar (or the bottom navigation on a small screen). To pair two hosts:

1. Copy the generated **Pairing link** from the initiator host.
2. On the connecting host, paste the link under **Connections** and click **Accept and pair** (or enter a 6-digit code with the initiator's address).
3. Confirm if connecting over a public address.

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

This version does not synchronize configuration repositories, schedule updates, or automatically activate systems. Build/check/apply actions are initiated in the selected host's panel.

## Verification

- `cargo test --no-default-features`: real gateway/peer daemon processes; routing, logs, confirmations, credential permissions/rotation, unknown peers and disconnects.
- `npm test`: translations, host URL scoping, policy and API requests.
- `nix build .#checks.x86_64-linux.module`: NixOS peer and signed-cache options.
- `nix build .#checks.x86_64-linux.peers`: two VMs; nas→web management, stable signing identity, rejection of an untrusted cache key, and reuse of a signed build on web where local building is deliberately disabled for the fixture.
