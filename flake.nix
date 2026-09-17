{
  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.systems.url = "github:nix-systems/triplet";
  inputs.parts.url = "github:hercules-ci/flake-parts";
  inputs.parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  inputs.crane.url = "github:ipetkov/crane";

  outputs =
    inputs:
    inputs.parts.lib.mkFlake { inherit inputs; } {
      systems = import inputs.systems;

      perSystem =
        { lib
        , pkgs
        , ...
        }:
        let
          crane = inputs.crane.mkLib pkgs;

          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              (crane.fileset.commonCargoSources ./src)
              (crane.fileset.commonCargoSources ./tests)
              ./tests/cli
            ];
          };

          args = {
            inherit src;
            pname = "things";
            version = (lib.importTOML ./Cargo.toml).package.version;
            strictDeps = true;
          };

          cargoArtifacts = crane.buildDepsOnly args;
        in
        {
          packages = {
            default = crane.buildPackage (args // {
              inherit cargoArtifacts;
              doCheck = false;
              meta.mainProgram = "things";
            });
          };

          checks.default = crane.cargoNextest (args // {
            inherit cargoArtifacts;
            nativeBuildInputs = with pkgs; [ jq writableTmpDirAsHomeHook ];
            preBuild = ''
              patchShebangs tests/cli/run.sh
            '';
          });

          devShells.default = crane.devShell {
            packages = with pkgs; [
              cargo-nextest
              deno
              jq
              nixpkgs-fmt
              rust-analyzer
              taplo
            ];
          };

          formatter = pkgs.writeShellScriptBin "formatter" ''
            pushd "$(${lib.getExe pkgs.git} rev-parse --show-toplevel)" > /dev/null
            set -eoux pipefail
            shopt -s globstar

            cargo clippy --all-targets -- -D warnings
            cargo fmt --all
            ${lib.getExe pkgs.deno} fmt readme.md .github
            ${lib.getExe pkgs.nixpkgs-fmt} .
            ${lib.getExe pkgs.taplo} format **/*.toml

            popd
          '';
        };
    };
}
