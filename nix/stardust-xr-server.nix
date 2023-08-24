{ rustPlatform
, src
, name
, openxr-loader
, libGL
, mesa
, xorg
, fontconfig
, libxkbcommon
, libclang
, cmake
, cpm-cmake
, pkg-config
, llvmPackages
}:

rustPlatform.buildRustPackage rec {
  inherit src name;
  cargoLock = {
    lockFile = (src + "/Cargo.lock");
    allowBuiltinFetchGit = true;
  };
  CPM_SOURCE_CACHE = "./build";
  postPatch = ''
    sk=$(echo $cargoDepsCopy/stereokit-sys-*/StereoKit)
    mkdir -p $sk/build/cpm

    # This is not ideal, the original approach was to fetch the exact cmake
    # file version that was wanted from GitHub directly, but at least this way it comes from Nixpkgs.. so meh
    cp ${cpm-cmake}/share/cpm/CPM.cmake $sk/build/cpm/CPM_0.32.2.cmake
  '';
  nativeBuildInputs = [
    cmake pkg-config llvmPackages.libcxxClang
  ];
  buildInputs = [
    openxr-loader libGL mesa xorg.libX11 fontconfig libxkbcommon
  ];
  LIBCLANG_PATH = "${libclang.lib}/lib";
}

