{
  description = "Backslash, a fast, extensible launcher for Linux";

  # The channel tarball: tested by Hydra, and fetched without the GitHub API.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        backslash = pkgs.callPackage ./packaging/nix/package.nix { };
        default = backslash;
      });

      overlays.default = final: _prev: {
        backslash = final.callPackage ./packaging/nix/package.nix { };
      };

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.backslash ];
          packages = [
            pkgs.deno
            pkgs.clippy
            pkgs.rustfmt
          ];
        };
      });

      formatter = forAll (pkgs: pkgs.nixfmt-rfc-style);
    };
}
