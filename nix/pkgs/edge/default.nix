{
  lib,
  pkgs,
  stdenv,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "edge";
  version = "0.1.0";

  cargoLock.lockFile = ../../../Cargo.lock;
  src = lib.cleanSource ../../..;
  doCheck = false; # Skip test phase (tests fails in sandbox)

  buildType = "production";
  buildAndTestSubdir = "crates/edge";

  meta = with lib; {
    description = "Ingress for embedded devices and connectivity toolbox";
    license = licenses.asl20;
    homepage = "https://github.com/airalab/robonomics";
    maintainers = with maintainers; [ akru ];
    platforms = intersectLists platforms.unix (
      platforms.aarch64 ++ platforms.x86
    );
  };
}
