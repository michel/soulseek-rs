{
  description = "Soulseek client for the terminal, with a scriptable CLI and daemon";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      version = (lib.importTOML ./Cargo.toml).workspace.package.version;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    in
    {
      packages = lib.genAttrs systems (system: {
        default = nixpkgs.legacyPackages.${system}.callPackage (
          { rustPlatform, installShellFiles }:
          rustPlatform.buildRustPackage {
            pname = "soulseek-rs";
            inherit version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--package"
              "soulseek-rs"
            ];
            doCheck = false;
            nativeBuildInputs = [ installShellFiles ];
            env.SOULSEEK_RS_VERSION = "${version}+git${lib.substring 0 12 self.lastModifiedDate}.${self.shortRev or "dirty"}";
            postInstall = ''
              installShellCompletion --cmd soulseek-rs \
                --bash <($out/bin/soulseek-rs completions print bash) \
                --zsh <($out/bin/soulseek-rs completions print zsh) \
                --fish <($out/bin/soulseek-rs completions print fish)
              $out/bin/soulseek-rs man > soulseek-rs.1
              installManPage soulseek-rs.1
            '';
            meta = {
              description = "Soulseek client for the terminal, with a scriptable CLI and daemon";
              homepage = "https://re-invention.nl/soulseek-rs/";
              license = lib.licenses.mit;
              mainProgram = "soulseek-rs";
            };
          }
        ) { };
      });
    };
}
