# Security policy

Yonix is experimental system-administration software. The development branch is the current maintenance target; older revisions have no guaranteed security backports or response times.

## Reporting a vulnerability

Do not disclose exploitable details, credentials or private logs in a public issue. If GitHub private vulnerability reporting is enabled, use [Report a vulnerability](https://github.com/REALalphas/yonix-up/security/advisories/new).

If that form is unavailable, open an issue requesting a private security contact **without vulnerability details**. Wait for the maintainer to establish a private channel before sharing a reproduction. This repository does not publish a dedicated security email address.

A private report should include the affected revision, prerequisites, impact, reproduction and any proposed fix. Use disposable test environments and dummy credentials. Coordinate public disclosure with the maintainer after a fix is available.

## Trust boundaries

- The configured user and anyone holding the gateway token are system administrators when system-changing actions are enabled. An authorized NixOS configuration can execute code as root during activation.
- The gateway token also grants access to its configured peers. Peer tokens, runtime connection files and binary-cache private keys are secrets.
- The API uses HTTP. Keep it on loopback or a trusted network, and use SSH or an encrypted VPN for untrusted networks. Opening the firewall does not add encryption or multi-user authorization.
- Nix source paths and store contents can be world-readable. Keep credentials outside configuration checkouts and Nix expressions.
- Cleanup can remove old generations, logs and eligible temporary files. Review retention settings and keep independent backups.

See the [README](README.md) and [peer trust model](docs/peering.md) for configuration and token rotation. The archived Bash script in `old/` is not maintained.
