{
  lib,
  pkgs,
  stdenv,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "libcps";
  version = "0.8.0";

  cargoLock.lockFile = ../../../Cargo.lock;
  src = lib.cleanSource ../../..;

  buildType = "production";
  buildAndTestSubdir = "crates/libcps";

  meta = with lib; {
    mainProgram = "cps";
    description = "Interacting with Robonomics CPS (Cyber-Physical Systems) pallet";
    license = licenses.asl20;
    homepage = "https://github.com/airalab/robins";
    maintainers = with maintainers; [ akru ];
    platforms = intersectLists platforms.unix (
      platforms.aarch64 ++ platforms.x86
    );
  };
}
