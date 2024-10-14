{
  nixConfig = {
    extra-substituters = [ "https://stardustxr.cachix.org" ];
    extra-trusted-public-keys = [
      "stardustxr.cachix.org-1:mWSn8Ap2RLsIWT/8gsj+VfbJB6xoOkPaZpbjO+r9HBo="
    ];
  };
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    hercules-ci-effects.url = "github:hercules-ci/hercules-ci-effects";

    # Since we do not have a monorepo, we have to fetch Flatland in order to use
    # it to create VM Tests
    flatland.url = "github:StardustXR/flatland";
  };
  outputs =
    inputs@{
      self,
      flake-parts,
      nixpkgs,
      hercules-ci-effects,
      flatland,
      ...
    }:
    let
      name = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.name;
      src = builtins.path {
        name = "${name}-source";
        path = toString ./.;
        filter =
          path: type:
          nixpkgs.lib.all (n: builtins.baseNameOf path != n) [
            "flake.nix"
            "flake.lock"
            "nix"
            "README.md"
          ];
      };
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ flake-parts.flakeModules.easyOverlay ];
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "riscv64-linux"
      ];
      perSystem =
        {
          config,
          self',
          inputs',
          pkgs,
          system,
          ...
        }:
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.self.overlays.default ];
          };
          overlayAttrs = config.packages;
          packages =
            let
              sk_gpu = pkgs.callPackage ./nix/sk_gpu.nix { };
            in
            {
              default = self'.packages.${name};
              gnome-graphical-test = self'.checks.gnome-graphical-test;
              "${name}" = pkgs.callPackage ./nix/stardust-xr-server.nix { inherit name src sk_gpu; };
            };
          apps.default = {
            type = "app";
            program = self'.packages.${name} + "/bin/stardust-xr-server";
          };
          checks.gnome-graphical-test = pkgs.nixosTest (
            import ./nix/gnome-graphical-test.nix { inherit pkgs self; }
          );
          devShells.default = pkgs.mkShell {
            inputsFrom = [ self'.packages.default ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
        };
      flake = {
        herculesCI.ciSystems = [ "x86_64-linux" ];
        effects =
          let
            pkgs = nixpkgs.legacyPackages.x86_64-linux;
            hci-effects = hercules-ci-effects.lib.withPkgs pkgs;
          in
          { ref, rev, ... }:
          {
            gnome-graphical-test = hci-effects.mkEffect {
              secretsMap."stardustxrDiscord" = "stardustxrDiscord";
              secretsMap."stardustxrIpfs" = "stardustxrIpfs";
              effectScript = ''
                readSecretString stardustxrDiscord .webhook > .webhook
                readSecretString stardustxrIpfs .basicauth > .basicauth
                set -x
                export RESPONSE=$(curl -H @.basicauth -F file=@${
                  self.packages."x86_64-linux".gnome-graphical-test
                }/screen.png https://ipfs-api.stardustxr.org/api/v0/add)
                export CID=$(echo "$RESPONSE" | ${pkgs.jq}/bin/jq -r .Hash)
                set +x
                export ADDRESS="https://ipfs.stardustxr.org/ipfs/$CID"
                ${pkgs.discord-sh}/bin/discord.sh \
                  --description "\`stardustxr/server\` has been modified, here's how it renders \`weston-cliptest\` on \`flatland\` via \`monado-service\` inside of the \`gnome-graphical-test\`" \
                  --field "Ref;${ref}" \
                  --field "Commit ID;${rev}" \
                  --field "Flatland Revision;${flatland.rev}" \
                  --field "Reproducer;\`nix build github:stardustxr/server/${rev}#gnome-graphical-test\`" \
                  --image "$ADDRESS"
              '';
            };
          };
      };
    };
}
