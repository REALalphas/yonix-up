Read and follow [ALPHA_UI_AGENT_RULES.md](ALPHA_UI_AGENT_RULES.md) when working on the browser client. Use the packaged @alpha/core and @alpha/react APIs; do not override component internals in application CSS.

Record every missing Alpha UI component, property, state, or behavior in `ui-unspec/<ComponentName>.md` and tell the user. Include the installed library version, the limitation, required behavior, proposed public API, verification criteria, and status. Do not hide gaps with application-side component replacements or CSS overrides. When updating Alpha UI, review these records, verify resolved gaps against the new API, and update their status while preserving the requirements and resolution history.

The production app is an immutable Nix package. Frontend dependencies are installed and rebuilt at package-build time; updating Alpha UI produces a new complete package. Never download or replace production assets at daemon startup. The optional development bootstrap uses a configured release source and keeps the last working local build.

Use official Alpha UI releases as-is. Do not modify the Alpha UI repository, patch installed packages, or create application-specific/custom Alpha UI versions. Communicate all requested library changes only through `ui-unspec`; wait for an official release that provides the required public API, then update and verify the application.
