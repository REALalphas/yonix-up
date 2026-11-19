{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  wrapGAppsHook4,
  gtk4,
  libadwaita,
  nix,
  nixos-rebuild,
  git,
  xdg-utils,
  callPackage,
  withGui ? true,
}:
let webAssets = callPackage ./web.nix {}; in
rustPlatform.buildRustPackage {
  pname = "yonix";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../build.rs
      ../Cargo.lock
      ../src
      ../locales
      (lib.fileset.fileFilter (file: file.hasExt "rs") ../tests)
      ../data
      ../LICENSE
      ../LICENSES
      ../THIRD_PARTY_NOTICES.md
      ../vendor/alpha-ui/README.md
      ../vendor/alpha-ui/manifest.json
      ../vendor/alpha-ui/alpha-ui-source-0.4.11.tar.gz
    ];
  };
  postPatch = ''
    rm -rf web
    cp -r ${webAssets} web
  '';
  cargoLock.lockFile = ../Cargo.lock;
  buildNoDefaultFeatures = !withGui;
  nativeBuildInputs = [
    makeWrapper
  ]
  ++ lib.optionals withGui [ pkg-config wrapGAppsHook4 ];
  buildInputs = lib.optionals withGui [
    gtk4
    libadwaita
  ];
  # Nix and Git are available to workers even outside an interactive shell.
  postInstall = ''
    install -Dm644 LICENSE $out/share/doc/yonix/LICENSE
    install -Dm644 THIRD_PARTY_NOTICES.md $out/share/doc/yonix/THIRD_PARTY_NOTICES.md
    cp -r LICENSES $out/share/doc/yonix/LICENSES
    install -Dm644 vendor/alpha-ui/README.md $out/share/doc/yonix/vendor/alpha-ui/README.md
    install -Dm644 vendor/alpha-ui/manifest.json $out/share/doc/yonix/vendor/alpha-ui/manifest.json
    install -Dm644 vendor/alpha-ui/alpha-ui-source-0.4.11.tar.gz $out/share/doc/yonix/vendor/alpha-ui/alpha-ui-source-0.4.11.tar.gz
  '' + lib.optionalString withGui ''
    install -Dm644 data/com.alphas.yonix.desktop $out/share/applications/com.alphas.yonix.desktop
    install -Dm644 data/com.alphas.yonix.svg $out/share/icons/hicolor/scalable/apps/com.alphas.yonix.svg
  '';
  # wrapGAppsHook wraps the GUI build; headless needs its own PATH wrapper.
  # nixos-rebuild covers legacy check/build plans outside the systemd service.
  postFixup = ''
    wrapProgram $out/bin/yonix --set YONIX_LAUNCHER "$out/bin/yonix" --prefix PATH : ${
      lib.makeBinPath (
        [
          nix
          nixos-rebuild
          git
        ]
        ++ lib.optionals withGui [ xdg-utils ]
      )
    }:/run/wrappers/bin
  '';
  meta = {
    description = "Native and remote NixOS system manager";
    homepage = "https://github.com/REALalphas/yonix-up";
    # Application license; the embedded UI retains the separate terms in LICENSES.
    license = lib.licenses.agpl3Plus;
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    mainProgram = "yonix";
  };
}
