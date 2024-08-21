{ stdenv, fetchurl, unzip }:

let
  sk_gpu_zip = fetchurl {
    url =
      "https://github.com/StereoKit/sk_gpu/releases/download/v2024.8.16/sk_gpu.v2024.8.16.zip";
    sha256 = "sha256-Wk3PZFlWqhrsQ8xG0sQaV2xSasdg2D7TMiPvl/CgtGU=";
  };
in stdenv.mkDerivation rec {
  name = "sk_gpu";
  src = sk_gpu_zip;
  unpackPhase = ''
    unzip -d $out ${sk_gpu_zip}
  '';
  nativeBuildInputs = [ unzip ];
}
