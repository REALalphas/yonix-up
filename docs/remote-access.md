# Web Interface and Remote Access

Yonix includes an embedded web panel served directly by its Rust daemon. The service is designed for local-first administration and provides authenticated access via private API tokens.

## Architecture and Security

- **Default binding**: Listens on loopback (`127.0.0.1:9847`).
- **Authentication**: All API requests require a bearer gateway token generated at daemon initialization.
- **Header verification**: Enforces strict `Host` and `Origin` checks to prevent DNS rebinding and cross-origin attacks.
- **Self-contained assets**: Web frontend assets, fonts, and scripts are embedded in the binary and served locally without third-party network requests.
- **Encryption**: Yonix does not provide built-in TLS or multi-user access control. For untrusted networks, access must be secured through an SSH tunnel or encrypted VPN (e.g. WireGuard/Tailscale).

---

## Local Access

1. Open `http://127.0.0.1:9847` in your browser.
2. Obtain the administrator token by running:
   ```console
   yonix token
   ```
3. Enter the token into the sign-in form.

Upon successful verification, the browser saves the token in `localStorage` for that specific origin. Clicking **Disconnect** in Maintenance clears the stored token.

---

## Remote Access via SSH Tunnel (Recommended)

When connecting across untrusted networks or the Internet, use SSH port forwarding to ensure encrypted transit and server authentication.

1. On your client machine, open the tunnel:
   ```console
   ssh -N -L 9847:127.0.0.1:9847 operator@server
   ```
   *(If port `9847` is already in use locally, replace the first number with an unused port, e.g. `19847:127.0.0.1:9847`.)*

2. Retrieve the sign-in token from the server:
   ```console
   ssh operator@server yonix token
   ```

3. Open `http://127.0.0.1:9847` on your client machine and paste the token.

---

## Network Bindings (LAN / VPN)

For trusted private networks, Yonix can be configured to listen directly on a specific network interface.

### Binding to a Private IPv4 Address

In your NixOS configuration:

```nix
programs.yonix = {
  enable = true;
  listenAddress = "10.0.0.10";
  openFirewall = true; # opens TCP port 9847 in nixos firewall
};
```

Access the panel at `http://10.0.0.10:9847`. Token authentication and same-origin checks remain strictly enforced.

In standalone JSON configurations (`config.json`), set:
```json
{
  "listen_address": "10.0.0.10"
}
```

### Binding to All Interfaces (`0.0.0.0`)

To listen on all IPv4 interfaces:

```nix
programs.yonix.listenAddress = "0.0.0.0";
programs.yonix.openFirewall = true;
```

*(Or `"listen_address": "0.0.0.0"` in `config.json`.)*

When bound to a wildcard or remote address:
- Open the panel using the server's numeric IP address (e.g., `http://192.168.1.50:9847`) or `http://localhost:9847` when local.
- CLI and desktop tools on the local machine automatically connect via `127.0.0.1`.
- HTTP `Host` verification accepts numeric IPv4 addresses and `localhost`.

---

## Remote Native Desktop Client (GTK)

The native GTK 4/libadwaita desktop client can also connect to a remote Yonix daemon over a tunnel or private network.

1. Export a client configuration template:
   ```console
   yonix init > remote.json
   ```

2. Save the remote server's token to a private local file (permissions `0600`):
   ```console
   ssh operator@server yonix token > ~/.config/yonix/server.token
   chmod 600 ~/.config/yonix/server.token
   ```

3. Launch the desktop GUI targeting the remote endpoint:
   ```console
   yonix --config remote.json \
     --url http://127.0.0.1:9847 \
     --token-file ~/.config/yonix/server.token gui
   ```

---

## Token Management and Revocation

- **Storage**. Under NixOS, the daemon stores its API token in `/var/lib/yonix/token`. The `yonix` group can read it (mode `0640`).
- **Never commit tokens**: Do not place tokens in Nix store expressions or Git repositories.
- **Revocation**: To invalidate existing sessions:
  1. Stop the service: `sudo systemctl stop yonix`
  2. Remove the token file: `sudo rm /var/lib/yonix/token`
  3. Start the service: `sudo systemctl start yonix`
  4. Distribute the newly generated token via `yonix token` to authorized clients.
