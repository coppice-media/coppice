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
        androidPkgs = import nixpkgs {
          inherit system;
          config = {
            android_sdk.accept_license = true;
            allowUnfree = true;
          };
        };

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

        # Frozen Expo compatibility environment; not part of the active Bun workspace.
        pinnedJDK = androidPkgs.jdk17;
        androidComposition = androidPkgs.androidenv.composeAndroidPackages {
          buildToolsVersions = [ "35.0.0" "36.0.0" ];
          platformVersions = [ "35" "36" ];
          cmakeVersions = [ "3.10.2" "3.22.1" ];
          includeNDK = true;
          ndkVersions = [ "27.0.12077973" "27.1.12297006" ];
        };
        androidSdk = androidComposition.androidsdk;

        android-sdk-root =
          "${androidComposition.androidsdk}/libexec/android-sdk";

        androidPackages =
          (with androidPkgs; [ pinnedJDK androidSdk pkg-config ]);
        androidLibraries = (with androidPkgs; [ libxml2.out ]);

      in {
        devShells.default = pkgs.mkShell genericShellConfig;

        devShells.android = pkgs.mkShell (genericShellConfig // {
          buildInputs = genericShellConfig.buildInputs ++ androidPackages;

          JAVA_HOME = pinnedJDK;
          JAVA_OPTS = "-Xms8g -Xmx8g";
          ANDROID_HOME =
            "${androidComposition.androidsdk}/libexec/android-sdk";
          ANDROID_SDK_ROOT =
            "${androidComposition.androidsdk}/libexec/android-sdk";
          ANDROID_NDK_ROOT = "${android-sdk-root}/ndk-bundle";

          shellHook = ''
            export LD_LIBRARY_PATH=${
              pkgs.lib.makeLibraryPath (libraries ++ androidLibraries)
            }:$LD_LIBRARY_PATH
          '';
        });

      });
}
