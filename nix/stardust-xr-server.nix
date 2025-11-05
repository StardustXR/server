{
  rustPlatform,
  src,
  name,
  vulkan-loader,
  vulkan-headers,
  libxkbcommon,

  pkg-config,

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

  postFixup = ''
    patchelf $out/bin/stardust-xr-server --add-rpath ${vulkan-loader}/lib
    patchelf $out/bin/stardust-xr-server --add-rpath ${openxr-loader}/lib
    patchelf $out/bin/stardust-xr-server --add-rpath ${libxkbcommon}/lib
  '';

  nativeBuildInputs = [
    pkg-config
  ];
  buildInputs = [
    vulkan-loader
    vulkan-headers
    openxr-loader
    wayland
    alsa-lib
  ];
}
