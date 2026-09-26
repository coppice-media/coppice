{
  inputs = {
    nixpkgs.url = "nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    android-nixpkgs.url = "github:tadfisher/android-nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        libraries = with pkgs; [
          webkitgtk_4_1
          gtk3
          cairo
          gdk-pixbuf
          glib
          dbus
          openssl
        ];

        rustVersion = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
        rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        bunVersion = "1.4.1";
        bunSources = {
          aarch64-darwin = pkgs.fetchurl {
            url = "https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-darwin-aarch64.zip";
            hash = "sha256-2Jc86DX6eGflzHmv7m/G8a4BF6pL1fwlRv0AxRL3E4Y=";
          };
          x86_64-darwin = pkgs.fetchurl {
            url = "https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-darwin-x64.zip";
            hash = "sha256-jzQjnydqPw0nv80f/s/l0hJ+dPsKpMCXGgzex7IlyWU=";
          };
          aarch64-linux = pkgs.fetchurl {
            url = "https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-linux-aarch64.zip";
            hash = "sha256-WAzndTMQjcaxC+wXITl+T1qkTpCXJtokUdSD38XlgdY=";
          };
          x86_64-linux = pkgs.fetchurl {
            url = "https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-linux-x64.zip";
            hash = "sha256-dMHDvufNmYUAyPlpzYlyNVrGoHIH6Uo57s4ZmbVv+r8=";
          };
        };
        bunToolchain = pkgs.bun.overrideAttrs (previousAttrs: {
          version = bunVersion;
          src = bunSources.${system} or
            (throw "Bun ${bunVersion} is not available for ${system}");
          passthru = previousAttrs.passthru // { sources = bunSources; };
          meta = previousAttrs.meta // {
            platforms = builtins.attrNames bunSources;
          };
        });

        packages = with pkgs; [
          git

          # JavaScript toolchain
          bunToolchain
          nodejs_22

          # rust
          rustToolchain
          cargo-deny
          cargo-edit
          cargo-watch
          bacon

          # Tauri deps
          curl
          wget
          pkg-config
          dbus
          openssl
          glib
          gtk3
          webkitgtk_4_1
        ];

        genericShellConfig = {
          buildInputs = packages;

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            export LD_LIBRARY_PATH=${
              pkgs.lib.makeLibraryPath libraries
            }:$LD_LIBRARY_PATH
          '';
        };

      in {
        devShells.default = pkgs.mkShell genericShellConfig;

      });
}
