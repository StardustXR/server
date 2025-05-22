{ stdenv, fetchurl, unzip }:

let
  sk_gpu_zip = fetchurl {
    url =
      "https://github.com/StereoKit/sk_gpu/releases/download/v2024.9.26/sk_gpu.v2024.9.26.zip";
    sha256 = "sha256-W32RveeCszioWGtbCsvAqB28YHvOsw2xJ15MosYLFXk=";
  };
in stdenv.mkDerivation rec {
  name = "sk_gpu";
  src = sk_gpu_zip;
  unpackPhase = ''
    unzip -d $out ${sk_gpu_zip}
  '';
  nativeBuildInputs = [ unzip ];
}
