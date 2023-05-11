{
  # 22.11 does not include PR #218472, hence we use the unstable version
  inputs.nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;

  inputs.fenix.url = github:nix-community/fenix;
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, fenix }:
    let
      name = "server";
      pkgs = system: import nixpkgs {
        inherit system;
      };
      shell = pkgs: pkgs.mkShell {
        inputsFrom = [ self.packages.${pkgs.system}.default ];

        # ---- START package specific dev settings ----
        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        # ---- END package specific dev settings ----
      };
      package = pkgs:
        let
          toolchain = fenix.packages.${pkgs.system}.minimal.toolchain;
        in
          (pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          }).buildRustPackage rec {
            pname = "stardust-xr-${name}";
            src = builtins.path {
                name = "stardust-xr-source";
                path = toString ./.;
                filter = path: type:
                  nixpkgs.lib.all
                  (n: builtins.baseNameOf path != n)
                  [
                    "flake.nix"
                    "flake.lock"
                    "nix"
                    "README.md"
                  ];
              };

            # ---- START package specific settings ----
            version = "0.10.2";

            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            postPatch = ''
              sk=$(echo $cargoDepsCopy/stereokit-sys-*/StereoKit)
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
      overlays.default = final: prev: {
        stardust-xr = (prev.stardust-xr or {}) // {
          ${name} = package final;
        };
      };

      packages."x86_64-linux".default = package (pkgs "x86_64-linux");
      packages."aarch64-linux".default = package (pkgs "aarch64-linux");

      devShells."x86_64-linux".default = shell (pkgs "x86_64-linux");
      devShells."aarch64-linux".default = shell (pkgs "aarch64-linux");
    };
}
