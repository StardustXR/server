{
  nixConfig = {
    extra-substituters = [ "https://stardustxr.cachix.org" ];
    extra-trusted-public-keys = [ "stardustxr.cachix.org-1:mWSn8Ap2RLsIWT/8gsj+VfbJB6xoOkPaZpbjO+r9HBo=" ];
  };

  # 22.11 does not include PR #218472, hence we use the unstable version
  inputs.nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;

  # Since we do not have a monorepo, we have to fetch Flatland in order to use
  # it to create VM Tests
  inputs.flatland.url = "github:StardustXR/flatland";

  inputs.fenix.url = github:nix-community/fenix;
  inputs.fenix.inputs.nixpkgs.follows = "nixpkgs";

  inputs.hercules-ci-effects.url = "github:hercules-ci/hercules-ci-effects";

  outputs = { self, nixpkgs, fenix, hercules-ci-effects, ... }:
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

      packages."x86_64-linux".gnome-graphical-test = self.checks.x86_64-linux.gnome-graphical-test;
      packages."aarch64-linux".gnome-graphical-test = self.checks.aarch64-linux.gnome-graphical-test;

      checks."x86_64-linux".gnome-graphical-test = (pkgs "x86_64-linux").nixosTest (import ./nix/gnome-graphical-test.nix { pkgs = (pkgs "x86_64-linux"); inherit self; });
      checks."aarch64-linux".gnome-graphical-test = (pkgs "aarch64-linux").nixosTest (import ./nix/gnome-graphical-test.nix { pkgs = (pkgs "aarch64-linux"); inherit self; });

      devShells."x86_64-linux".default = shell (pkgs "x86_64-linux");
      devShells."aarch64-linux".default = shell (pkgs "aarch64-linux");

      herculesCI.ciSystems = [ "x86_64-linux" ];

      effects = let
        pkgs = nixpkgs.legacyPackages.x86_64-linux;
        hci-effects = hercules-ci-effects.lib.withPkgs pkgs;
      in { branch, rev, ... }: {
        gnome-graphical-test = hci-effects.mkEffect {
          secretsMap."stardustxrDiscord" = "stardustxrDiscord";
          secretsMap."stardustxrIpfs" = "stardustxrIpfs";
          effectScript = ''
            readSecretString stardustxrDiscord .webhook > .webhook
            readSecretString stardustxrIpfs .basicauth > .basicauth
            set -x
            export RESPONSE=$(curl -H @.basicauth -F file=@${self.packages."x86_64-linux".gnome-graphical-test}/screen.png https://ipfs-api.stardustxr.org/api/v0/add)
            export CID=$(echo "$RESPONSE" | ${pkgs.jq}/bin/jq -r .Hash)
            set +x
            export ADDRESS="https://ipfs.stardustxr.org/ipfs/$CID"
            ${pkgs.discord-sh}/bin/discord.sh \
              --description "\`stardustxr/server\` has been modified, here's how it renders inside of the \`gnome-graphical-test\`" \
              --field "Branch;${branch}" \
              --field "Commit ID;${rev}" \
              --field "Reproducer;\`nix build github:stardustxr/server/${rev}#gnome-graphical-test\`" \
              --image "$ADDRESS"
          '';
        };
      };
    };
}
