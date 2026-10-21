{ pkgs }:

rec {
  default = edge;

  edge = pkgs.callPackage ./edge {};
  libcps = pkgs.callPackage ./libcps {};
  mqtt-bridge = pkgs.callPackage ./mqtt-bridge {};
}
