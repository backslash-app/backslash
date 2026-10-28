# Backslash for Nix, built from this checkout (see flake.nix).
{
  lib,
  rustPlatform,
  pkg-config,
  wrapGAppsHook4,
  gtk4,
  gtk4-layer-shell,
  deno,
  git,
  bubblewrap,
  libsecret,
  wl-clipboard,
  python3,
}:

let
  root = ../..;
  version = (lib.importTOML (root + "/Cargo.toml")).workspace.package.version;
in
rustPlatform.buildRustPackage {
  pname = "backslash";
  inherit version;

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/crates")
      (root + "/sdk")
      (root + "/extensions")
      (root + "/data")
    ];
  };
  cargoLock.lockFile = root + "/Cargo.lock";

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
  ];
  buildInputs = [ gtk4 ];

  # The GTK-free core; the window's tests need a display.
  cargoTestFlags = [
    "-p"
    "backslash-core"
    "--lib"
  ];
  nativeCheckInputs = [ git ];

  postInstall = ''
    install -d $out/share/backslash/sdk/python
    install -m644 sdk/*.ts $out/share/backslash/sdk/
    install -m644 sdk/python/*.py $out/share/backslash/sdk/python/
    cp -r extensions $out/share/backslash/extensions
    install -Dm644 data/app.backslash.Backslash.desktop -t $out/share/applications
    install -Dm644 data/icons/backslash.png $out/share/icons/hicolor/512x512/apps/app.backslash.Backslash.png
    install -Dm644 data/icons/backslash.svg $out/share/icons/hicolor/scalable/apps/app.backslash.Backslash.svg
    install -Dm644 data/app.backslash.Backslash.metainfo.xml -t $out/share/metainfo
  '';

  # Tools it runs come after the user's own (their compositor's, their Deno).
  # gtk4-layer-shell is loaded at runtime; not through LD_LIBRARY_PATH, which
  # Deno refuses in the environment of the programs extensions run.
  preFixup = ''
    gappsWrapperArgs+=(
      --suffix PATH : ${
        lib.makeBinPath [
          deno
          git
          bubblewrap
          libsecret
          wl-clipboard
          python3
        ]
      }
      --set-default BACKSLASH_LAYER_SHELL_LIB ${gtk4-layer-shell}/lib/libgtk4-layer-shell.so.0
    )
  '';

  meta = {
    description = "A fast, extensible launcher for Linux";
    homepage = "https://github.com/backslash-app/backslash";
    license = lib.licenses.mit;
    mainProgram = "backslash";
    platforms = lib.platforms.linux;
  };
}
