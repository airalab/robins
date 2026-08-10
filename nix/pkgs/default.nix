{ pkgs, self }:

let
  # Common parameters
  revHash = if (self ? rev) then self.rev else self.dirtyRev;
in rec {
  default = robonomics;

  robonomics = pkgs.callPackage ./robonomics { inherit revHash; };
  libcps = pkgs.callPackage ./libcps {};
}
