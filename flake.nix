{
  inputs.nixpkgs.url = github:NixOS/nixpkgs/nixos-22.11;

  inputs.fenix.url = github:nix-community/fenix;
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  inputs.flake-utils.url = github:numtide/flake-utils;

  outputs = { self, nixpkgs, fenix, flake-utils }:
    flake-utils.lib.simpleFlake {
      inherit self nixpkgs;
      name = "stardust-xr";
      overlay = pkgs: prev:
        let
          toolchain = fenix.packages.${pkgs.system}.minimal.toolchain;

          name = "server";
          pkg = (pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          }).buildRustPackage rec {
            pname = "stardust-xr-${name}";
            src = ./.;

            # ---- START package specific settings ----
            version = "20230314";
            cargoSha256 = "sha256-H6qhpvm6Dqn6EETCtgAcT/iof9ZZHm0ahTkX9cChows=";

            postPatch = ''
              sk=/build/${pname}-${version}-vendor.tar.gz/stereokit-sys/StereoKit
              mkdir -p $sk/build/cpm
              cp ${pkgs.fetchurl {
                url = "https://github.com/cpm-cmake/CPM.cmake/releases/download/v0.32.2/CPM.cmake";
                hash = "sha256-yDHlpqmpAE8CWiwJRoWyaqbuBAg0090G8WJIC2KLHp8=";
              }} $sk/build/cpm/CPM_0.32.2.cmake
            '';

            CPM_SOURCE_CACHE = "./build";

            nativeBuildInputs = with pkgs; [
              cmake pkg-config llvmPackages.libcxxClang
            ];

            buildInputs = with pkgs; [
              openxr-loader libGL mesa xorg.libX11 fontconfig libxkbcommon
            ];

            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            # ---- END package specific settings ----
          };
        in
        {
          stardust-xr.${name} = pkg;
          stardust-xr.defaultPackage = pkg;
        };
      shell = { pkgs }: pkgs.mkShell {
        inputsFrom = [ pkgs.stardust-xr.defaultPackage ];

        # ---- START package specific dev settings ----
        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        # ---- END package specific dev settings ----
      };
    };
}
