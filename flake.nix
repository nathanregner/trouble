{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      fenix,
      flake-parts,
      nixpkgs,
      treefmt-nix,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        treefmt-nix.flakeModule
      ];
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      perSystem =
        {
          pkgs,
          lib,
          system,
          ...
        }:
        rec {
          # https://flake.parts/overlays#consuming-an-overlay
          _module.args.pkgs = import nixpkgs {
            inherit system;
            overlays = [ fenix.overlays.default ];
          };

          treefmt = {
            projectRootFile = "flake.nix";
            programs = {
              nixfmt.enable = true;
              rustfmt = {
                enable = true;
                package = pkgs.fenix.complete.rustfmt;
              };
              taplo.enable = true;
            };
          };

          devShells.default =
            let
              toolchain =
                with pkgs.fenix;
                combine [
                  targets.thumbv6m-none-eabi.latest.rust-std
                  targets.aarch64-unknown-linux-gnu.latest.rust-std
                  (complete.withComponents [
                    "cargo"
                    "clippy"
                    "rust-analyzer"
                    "rust-src"
                    "rustfmt"
                  ])
                ];
            in
            pkgs.mkShell {
              buildInputs = with pkgs; [
                cargo-insta
                flip-link
                openssl
                openssl.dev
                pkg-config
                probe-rs-tools
                rust-analyzer
              ];
              env.RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
            };
        };

      flake = {
      };
    };
}
