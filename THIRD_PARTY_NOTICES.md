# Third-party notices

Yonix application code is licensed under AGPL-3.0-or-later; see [LICENSE](LICENSE). This grant does not replace licenses of dependencies or assets. Copyright notices and license texts copied below are preserved verbatim in `LICENSES/` and are installed under `share/doc/yonix/` by the Nix package.

## Browser runtime and assets

| Material | Version | Terms / notice |
| --- | --- | --- |
| Alpha UI core and React adapter | 0.4.11 | [AHCL-1.0](LICENSES/Alpha-UI-AHCL-1.0.txt), [upstream notices](LICENSES/Alpha-UI-THIRD_PARTY_NOTICES.md) |
| React | 19.3.0 | [MIT](LICENSES/react-MIT.txt) |
| React DOM | 19.3.0 | [MIT](LICENSES/react-dom-MIT.txt) |
| Scheduler | 0.28.0 | [MIT](LICENSES/scheduler-MIT.txt) |
| Lucide / Feather icons (`lucide-static`) | 1.52.0 | [ISC / MIT](LICENSES/licenses/lucide-LICENSE.txt) |
| Noto Sans Variable (`@fontsource-variable/noto-sans`) | 5.3.0 | [SIL OFL 1.1](LICENSES/licenses/noto-sans-OFL.txt) |
| Manrope Variable (`@fontsource-variable/manrope`) | 5.3.0 | [SIL OFL 1.1](LICENSES/licenses/manrope-OFL.txt) |
| Fira Code Variable (`@fontsource-variable/fira-code`) | 5.3.0 | [SIL OFL 1.1](LICENSES/licenses/fira-code-OFL.txt) |

Alpha UI is a separately licensed, source-available library with use restrictions; AHCL-1.0 is not an open-source license. Its terms include different conditions for individual and organizational use, Hub integration and distribution of corresponding source. Its licensor exception applies only to rights owned or controlled by the licensor and does not grant additional rights in third-party material. Recipients must review its terms for their own use.

Official archives are kept unchanged in `vendor/alpha-ui/`. The 0.4.11 manifest records archive sizes and SHA-512 digests. The matching official `alpha-ui-source-0.4.11.tar.gz` is included alongside the core and React packages. The Nix package installs this source archive, manifest and release-input notes under `share/doc/yonix/vendor/alpha-ui/`. Unused historical archives and the unused project-server package are excluded from the current tree.

## Other dependencies

`Cargo.lock` and `package-lock.json` record the dependency versions. Rust crates, build tools and system libraries retain their upstream licenses. This document inventories the bundled browser runtime and assets; it is not a complete license inventory of the Nix closure. Preserve applicable upstream notices when redistributing those dependencies.
