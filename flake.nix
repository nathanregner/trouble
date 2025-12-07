{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";

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
      nixpkgs-unstable,
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
            overlays = [
              fenix.overlays.default
              (stableFinal: _stablePrev: {
                unstable = import nixpkgs-unstable {
                  inherit system;
                  overlays = [
                    (
                      final: prev:
                      lib.optionalAttrs prev.stdenv.hostPlatform.isDarwin {
                        # https://github.com/NixOS/nixpkgs/pull/457704
                        inherit (stableFinal) uhd;
                      }
                    )
                  ];
                };
              })
            ];
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

          packages =
            let
              toolchain = pkgs.fenix.complete.withComponents [
                "cargo"
                "rustc"
              ];
              rustPlatform = (
                pkgs.makeRustPlatform {
                  cargo = toolchain;
                  rustc = toolchain;
                }
              );
            in
            {
              # inherit (pkgs.unstable) uhd;

              pdu-utils = pkgs.unstable.gnuradioPackages.callPackage ./packages/pdu-utils/package.nix { };

              # default = rustPlatform.buildRustPackage {
              #   pname = "";
              #   version = "0.1.0";
              #   src = lib.fileset.toSource {
              #     root = ./.;
              #     fileset = lib.fileset.unions [
              #       ./Cargo.lock
              #       ./Cargo.toml
              #       ./src
              #       ./tests
              #     ];
              #   };
              #
              #   cargoLock.lockFile = ./Cargo.lock;
              # };
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
                pkg-config
                openssl
                openssl.dev
                cargo-insta
                flip-link
                probe-rs
                pkgs.unstable.rust-analyzer
              ];
              env.RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
            };
        };

      flake = {
      };
    };
}
