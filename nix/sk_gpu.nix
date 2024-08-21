{ stdenv, fetchurl, unzip }:

let
  sk_gpu_zip = fetchurl {
    url =
      "https://github.com/StereoKit/sk_gpu/releases/download/v2024.8.12/sk_gpu.v2024.8.12.zip";
    sha256 = "sha256-NPjOFzu7AlpQKJ8PZYeUuegrR6TXtRyg+Hm2BxIAMLI=";
  };
in stdenv.mkDerivation rec {
  name = "sk_gpu";
  src = sk_gpu_zip;
  unpackPhase = ''
    unzip -d $out ${sk_gpu_zip}
  '';
  nativeBuildInputs = [ unzip ];
}
