{
  lib,
  pkgs,
  stdenv,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "mqtt-bridge";
  version = "0.1.0";

  cargoLock.lockFile = ../../../Cargo.lock;
  src = lib.cleanSource ../../..;

  buildType = "production";
  buildAndTestSubdir = "crates/mqtt-bridge";

  meta = with lib; {
    description = "MQTT publisher/subscriber for CPS pallet";
    license = licenses.asl20;
    homepage = "https://github.com/airalab/robins";
    maintainers = with maintainers; [ akru ];
    platforms = intersectLists platforms.unix (
      platforms.aarch64 ++ platforms.x86
    );
  };
}
