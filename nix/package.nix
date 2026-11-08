{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  wrapGAppsHook4,
  gtk4,
  libadwaita,
  nix,
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
  postInstall = lib.optionalString withGui ''
    install -Dm644 data/io.github.realalphas.Yonix.desktop $out/share/applications/io.github.realalphas.Yonix.desktop
    install -Dm644 data/io.github.realalphas.Yonix.svg $out/share/icons/hicolor/scalable/apps/io.github.realalphas.Yonix.svg
  '';
  # wrapGAppsHook wraps the GUI build; headless needs its own PATH wrapper.
  postFixup = ''
    wrapProgram $out/bin/yonix --prefix PATH : ${
      lib.makeBinPath (
        [
          nix
          git
        ]
        ++ lib.optionals withGui [ xdg-utils ]
      )
    }:/run/wrappers/bin
  '';
  meta = {
    description = "Native and remote NixOS system manager";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "yonix";
  };
}
