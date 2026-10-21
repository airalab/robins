{
  description = "Robonomics Network Flakes";

  nixConfig = {
    extra-substituters = [
      "https://fenix.cachix.org"
      "https://polkadot.cachix.org"
      "https://robonomics.cachix.org"
    ];
    extra-trusted-public-keys = [
      "fenix.cachix.org-1:ecJhr+RdYEdcVgUkjruiYhjbBloIEGov7bos90cZi0Q="
      "polkadot.cachix.org-1:qOFthM8M0DTotg8A48wWTZBgJD6h1rV9Jaszt6QE/N0="
      "robonomics.cachix.org-1:H3FwZ3khWXfEZ2OlPEiqRenpW1pDMAgRRRXMoksO2Bw="
    ];
  };

  inputs = {
    systems.url = "github:nix-systems/default";
    nixpkgs.url = "github:NixOS/nixpkgs/f20cd90e28fe0403873f473b29e42662cd86e8c4";
    polkadot.url = "github:andresilva/polkadot.nix";
    polkadot.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      systems,
      fenix,
      polkadot,
      ...
    }:
    let 
      mkPkgs = system: nixpkgs.legacyPackages.${system}.appendOverlays [polkadot.overlays.default];
      eachSystem = f: nixpkgs.lib.genAttrs (import systems) (system: f system (mkPkgs system));
    in {
      checks = eachSystem (
        system: pkgs: {
          buildAll = pkgs.symlinkJoin {
            name = "build-all-packages";
            paths = builtins.attrValues self.packages.${system};
          };
        }
      );

      lib = eachSystem (system: pkgs: {
        mkDevShell = args: import ./shell.nix ({ inherit pkgs; } // args);
      });

      devShells = eachSystem (
        system: pkgs: rec {
          default = self.lib.${system}.mkDevShell {
            packages = with pkgs; [
              taplo actionlint cargo-nextest cargo-audit cargo-machete mdbook
              pkgs.polkadot polkadot-parachain polkadot-omni-node
            ];
            env.RUSTC_WRAPPER = pkgs.lib.getExe pkgs.sccache;
          }; 
        }
      );

      packages = eachSystem (system: pkgs: import ./nix/pkgs { inherit pkgs self; });
    }
    // {
      overlays = {
        default = final: prev: import ./overlay.nix final prev;
      };
    };
}
