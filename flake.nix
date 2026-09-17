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

          # the snapshot cases under tests/cli run the built binary through run.sh
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./rustfmt.toml
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

          # dependencies built once and shared by the package and the check
          cargoArtifacts = crane.buildDepsOnly args;

          things = crane.buildPackage (args // {
            inherit cargoArtifacts;
            doCheck = false;
            meta.mainProgram = "things";
          });
        in
        {
          packages = { inherit things; default = things; };

          # run.sh wraps every snapshot case and pretty prints commit payloads with jq
          checks.default = crane.cargoNextest (args // {
            inherit cargoArtifacts;
            nativeBuildInputs = [ pkgs.jq ];
            preBuild = ''
              export HOME="$TMPDIR"
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
