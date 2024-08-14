{ lib, stdenv, fetchFromGitHub, cmake }:

stdenv.mkDerivation rec {
  pname = "meshoptimizer";
  version = "0.20";

  src = fetchFromGitHub {
    owner = "zeux";
    repo = "meshoptimizer";
    rev = "c21d3be6ddf627f8ca852ba4b6db9903b0557858";
    sha256 = "sha256-QCxpM2g8WtYSZHkBzLTJNQ/oHb5j/n9rjaVmZJcCZIA=";
  };

  nativeBuildInputs = [ cmake ];

  cmakeFlags = [
    "-DCMAKE_POSITION_INDEPENDENT_CODE=ON"
  ];

  meta = with lib; {
    description = "Mesh optimization library that makes meshes smaller and faster to render";
    homepage = "https://github.com/zeux/meshoptimizer";
    license = licenses.mit;
    platforms = platforms.all;
  };
}
  