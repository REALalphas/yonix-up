# Security policy

Yonix is experimental system-administration software. The `main` branch is the current security maintenance target; security fixes for other branches or revisions are not guaranteed.

## Reporting a vulnerability

Do not disclose exploitable details, credentials or private logs in a public issue. If private vulnerability reporting is enabled for this repository, use GitHub's [Report a vulnerability](https://github.com/REALalphas/yonix-up/security/advisories/new) form.

If that form is unavailable, open an issue asking for a private security contact without including vulnerability details. Wait for the maintainer to establish a private channel before sharing a reproduction. This repository does not publish a dedicated security email address.

Include the affected revision, prerequisites, impact and reproduction steps in a private report. Remove credentials and personal data; use disposable environments and dummy credentials. Coordinate public disclosure with the maintainer after a fix is available.

## Trust boundaries

- When system-changing actions are enabled, members of the `yonix` group and anyone holding the gateway token can administer the system. An authorized NixOS configuration can execute code as root during activation.
- The gateway token also grants access to configured peers. Peer tokens, runtime connection files and binary-cache private keys are secrets.
- The API uses HTTP. Keep it on loopback or a trusted network; use SSH or an encrypted VPN for untrusted networks. Opening the firewall does not add encryption or multi-user authorization.
- Nix source paths and store contents can be world-readable. Keep credentials outside configuration checkouts and Nix expressions.
- Cleanup can remove old generations, logs and eligible temporary files. Review retention settings and keep independent backups.

See the [README](README.md) for configuration guidance and the [peer trust model](docs/peering.md) for peer setup and token rotation.
