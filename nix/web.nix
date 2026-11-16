{ lib, stdenvNoCC, nodejs, fetchurl }:
let
  lock = builtins.fromJSON (builtins.readFile ../package-lock.json);
  # Include only the release archives actually referenced by the lock file.
  localArchives = lib.mapAttrsToList (name: pkg:
    ../. + "/${lib.removePrefix "file:" pkg.resolved}")
    (lib.filterAttrs (name: pkg: pkg ? resolved && lib.hasPrefix "file:" pkg.resolved) lock.packages);
  archives = lib.mapAttrsToList (name: pkg: fetchurl { url = pkg.resolved; hash = pkg.integrity; })
    (lib.filterAttrs (name: pkg: pkg ? resolved && lib.hasPrefix "https:" pkg.resolved
      && (!(pkg ? libc) || builtins.elem "glibc" pkg.libc)
      && (!(pkg ? os) || builtins.elem "linux" pkg.os)
      && (!(pkg ? cpu) || builtins.elem (if stdenvNoCC.hostPlatform.isAarch64 then "arm64" else "x64") pkg.cpu)) lock.packages);
in stdenvNoCC.mkDerivation {
  pname = "yonix-web";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions ([ ../package.json ../package-lock.json ../tsconfig.json ../frontend ../locales ../tests/frontend.test.mjs ] ++ localArchives);
  };
  nativeBuildInputs = [ nodejs ];
  buildPhase = ''
    export HOME=$TMPDIR/home
    mkdir -p "$HOME"
    npm cache add ${lib.escapeShellArgs archives} --ignore-scripts --offline >/dev/null
    npm ci --offline --ignore-scripts --no-audit --no-fund
    patchShebangs node_modules
    npm run build
    npm test
  '';
  installPhase = ''mkdir -p $out; cp -r web/. $out/'';
}
