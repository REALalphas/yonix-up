# Official Alpha UI release inputs

The current frontend uses the unchanged official `@alpha/core` and `@alpha/react` 0.4.11 archives. `alpha-ui-source-0.4.11.tar.gz` contains the matching upstream source and build scripts. All three archives match the SHA-512 integrity values and sizes in `manifest.json`.

The manifest describes the complete upstream release, so it also lists packages not used or stored here (Qwik and project-server). Those entries are release metadata, not missing Yonix build inputs. The npm lock file determines the two package archives included in the Nix frontend build.

Alpha UI is separately licensed under AHCL-1.0. See [third-party notices](../../THIRD_PARTY_NOTICES.md) and preserve the source archive and notices with distributions. Do not patch these archives or create application-specific library versions; use an official upstream release for updates.
