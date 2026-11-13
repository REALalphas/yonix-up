# Third-party notices

AHCL-1.0 applies only to material its licensor is entitled to license. Dependencies, fonts, icons, and other third-party material retain their original terms. Each Alpha's UI package includes this notice and verbatim license texts in `licenses/`.

| Material | Locked package | License |
| --- | --- | --- |
| Lucide, including icons derived from Feather | `lucide-static@1.52.0` | [ISC and Feather MIT](licenses/lucide-LICENSE.txt) |
| Noto Sans Variable | `@fontsource-variable/noto-sans@5.3.0` | [SIL OFL 1.1](licenses/noto-sans-OFL.txt) |
| Manrope Variable | `@fontsource-variable/manrope@5.3.0` | [SIL OFL 1.1](licenses/manrope-OFL.txt) |
| Fira Code Variable | `@fontsource-variable/fira-code@5.3.0` | [SIL OFL 1.1](licenses/fira-code-OFL.txt) |

Core includes a subset of Lucide SVGs in `dist/icons/`. Regular-weight copies change `stroke-width="2"` to `stroke-width="1.5"`; the other copies retain the upstream stroke. The combined Lucide/Feather license is also included at `dist/icons/LICENSE`.

Fonts are unmodified dependencies imported by `@alpha/core/fonts.css`. Keep their copyright and OFL notices when redistributing font files, including files bundled into an application. The OFL does not apply to application code or text rendered with the fonts.

React, React DOM, Qwik, Vite, TypeScript, tsc-rs, and other dependencies remain separately licensed. The adapters externalize their framework runtimes; applications that bundle them must preserve the applicable notices. This is not an exhaustive inventory of an application's transitive dependencies.

## Design resources

The only non-Lucide runtime asset is the repository owner's logo, whose ownership the owner has confirmed. It is distributed as `dist/assets/hub_logo.svg` in `@alpha/core`. In the source repository, the original is `packages/core/assets/hub_logo.svg` and its Figma provenance is recorded in `source/resources/assets.json`; these source paths are not part of the installed packages. Figma access alone does not grant rights in third-party material.

Historical VPN-brand images, cover artwork, and unused runtime SVGs have been removed. Test avatars are private local demo inputs under the ignored `apps/shared/local-assets/`; they are excluded from production builds and packages.

Temporary reference captures and raw Figma exports have also been removed from the source repository. This notice does not itself grant trademark rights or rights in separately licensed third-party material.
