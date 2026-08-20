{
  description = "hew - config-driven viewer for mixed structured/plain log streams";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forEachSystem (pkgs:
        let
          crossCCs = pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.pkgsCross.musl64.stdenv.cc
            pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc
            pkgs.pkgsCross.aarch64-multiplatform-musl.stdenv.cc
            pkgs.pkgsCross.mingwW64.stdenv.cc
          ];

          llvmCovEnv = {
            LLVM_COV = "${pkgs.llvmPackages.llvm}/bin/llvm-cov";
            LLVM_PROFDATA = "${pkgs.llvmPackages.llvm}/bin/llvm-profdata";
          };

          crossCCEnv = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            CC_x86_64_unknown_linux_musl =
              "${pkgs.pkgsCross.musl64.stdenv.cc}/bin/x86_64-unknown-linux-musl-cc";
            CC_aarch64_unknown_linux_gnu =
              "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-cc";
            CC_aarch64_unknown_linux_musl =
              "${pkgs.pkgsCross.aarch64-multiplatform-musl.stdenv.cc}/bin/aarch64-unknown-linux-musl-cc";
          };
        in
        {
          default = pkgs.mkShell ({
            packages = (with pkgs; [
              rustc
              cargo
              clippy
              rustfmt
              rust-analyzer
            ])
            ++ crossCCs
            ++ (with pkgs; [
              cargo-nextest
              cargo-llvm-cov
              cargo-deny
              cargo-audit
              cargo-machete
              cargo-bloat

              hyperfine
              gawk
              binutils
            ]);
          } // llvmCovEnv // crossCCEnv);
        });

      packages = forEachSystem (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "hew";
          version = "1.0.0";
          src = self;
          cargoLock.lockFile = "${self}/Cargo.lock";
        };
      });
    };
}
