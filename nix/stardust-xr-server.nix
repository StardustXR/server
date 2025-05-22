{ rustPlatform
, src
, name
, libGL
, mesa
, xorg
, fontconfig
, libxkbcommon
, xkeyboard_config
, libclang

, cmake
, cpm-cmake
, pkg-config
, llvmPackages
, fetchFromGitHub
, sk_gpu
, libXau
, libgbm

, libXdmcp
, stdenv
, lib
, openxr-loader
}:

rustPlatform.buildRustPackage rec {
  inherit src name;
  cargoLock = {
    lockFile = (src + "/Cargo.lock");
    allowBuiltinFetchGit = true;
  };
  buildFeatures = [ "local_deps" ];
  FORCE_LOCAL_DEPS = true;
  CPM_LOCAL_PACKAGES_ONLY = true;
  CPM_SOURCE_CACHE = "./build";
  CPM_USE_LOCAL_PACKAGES = true;
  CPM_DOWNLOAD_ALL = false;

  meshoptimizer = fetchFromGitHub {
    owner = "zeux";
    repo = "meshoptimizer";
    rev = "c21d3be6ddf627f8ca852ba4b6db9903b0557858";
    sha256 = "sha256-QCxpM2g8WtYSZHkBzLTJNQ/oHb5j/n9rjaVmZJcCZIA=";
  };
  basis_universal = fetchFromGitHub {
    owner = "BinomialLLC";
    repo = "basis_universal";
    rev = "900e40fb5d2502927360fe2f31762bdbb624455f";
    sha256 = "sha256-zBRAXgG5Fi6+5uPQCI/RCGatY6O4ELuYBoKrPNn4K+8=";
  };
  openxr_loader = fetchFromGitHub {
    owner = "KhronosGroup";
    repo = "OpenXR-SDK";
    rev = "288d3a7ebc1ad959f62d51da75baa3d27438c499";
    sha256 = "sha256-RdmnBe26hqPmqwCHIJolF6bSmZRmIKVlGF+TXAY35ig=";
  };

  DEP_MESHOPTIMIZER_SOURCE = "${meshoptimizer}";
  DEP_BASIS_UNIVERSAL_SOURCE = "${basis_universal}";
  DEP_SK_GPU_SOURCE = "${sk_gpu}";
  DEP_OPENXR_LOADER_SOURCE = "${openxr_loader}";

  postPatch = let libPath = lib.makeLibraryPath [ stdenv.cc.cc.lib ];
  in ''
    sk=$(echo $cargoDepsCopy/stereokit-rust-*/StereoKit)
    mkdir -p $sk/build/cpm

    # This is not ideal, the original approach was to fetch the exact cmake
    # file version that was wanted from GitHub directly, but at least this way it comes from Nixpkgs.. so meh
    cp ${cpm-cmake}/share/cpm/CPM.cmake $sk/build/cpm/CPM_0.38.7.cmake
    mkdir -p $sk/sk_gpu
    cp -R ${sk_gpu}/* $sk/sk_gpu
    chmod -R 755 $sk/sk_gpu/tools/linux_x64/*
    export DEP_SK_GPU_SOURCE=$sk/sk_gpu
    export LD_LIBRARY_PATH="${stdenv.cc.cc.lib}/lib";
                    patchelf --set-interpreter "$(cat $NIX_CC/nix-support/dynamic-linker)" \
                      --set-rpath "${libPath}" \
                    $sk/sk_gpu/tools/linux_x64/skshaderc
  '';
  nativeBuildInputs = [ cmake pkg-config llvmPackages.libcxxClang ];
  buildInputs = [
    libGL
    mesa
    xorg.libX11.dev
    xorg.libXft
    xorg.libXfixes
    fontconfig
    libxkbcommon
    xkeyboard_config
    libXau
    libXdmcp
    openxr-loader
    libgbm
  ];
  LIBCLANG_PATH = "${libclang.lib}/lib";
}
