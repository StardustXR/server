{
  rustPlatform,
  src,
  name,
  vulkan-loader,
  vulkan-headers,
  mesa,
  xorg,
  fontconfig,
  libxkbcommon,
  libclang,

  cmake,
  pkg-config,
  llvmPackages,
  fetchFromGitHub,
  libXau,

  libXdmcp,
  stdenv,
  lib,
  openxr-loader,
  wayland,
  alsa-lib,
}:

rustPlatform.buildRustPackage rec {
  inherit src name;
  cargoLock = {
    lockFile = (src + "/Cargo.lock");
    allowBuiltinFetchGit = true;
  };

  preBuild = ''
    substituteInPlace /build/cargo-vendor-dir/bevy_gltf-0.16.1/Cargo.toml \
      --replace-fail '[lints]' "" \
      --replace-fail 'workspace = true' ""
  '';

  postFixup = ''
    patchelf $out/bin/stardust-xr-server --add-rpath ${vulkan-loader}/lib
    patchelf $out/bin/stardust-xr-server --add-rpath ${openxr-loader}/lib
    patchelf $out/bin/stardust-xr-server --add-rpath ${libxkbcommon}/lib
  '';

  nativeBuildInputs = [
    cmake
    pkg-config
    llvmPackages.libcxxClang
  ];
  buildInputs = [
    vulkan-loader
    vulkan-headers
    mesa
    xorg.libX11.dev
    xorg.libXft
    xorg.libXfixes
    fontconfig
    libxkbcommon
    libXau
    libXdmcp
    openxr-loader
    wayland
    alsa-lib
  ];

  LIBCLANG_PATH = "${libclang.lib}/lib";
}
