{
  description = "Prebuilt Topiary Markdown formatter";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/0dc498bfe49baeb18b0f9891e800b53bb2603193";
    release = {
      # Lock to a Git revision, not to a mutable releases/latest URL.
      url = "github:InnocentZero/topiary-markdown-fmt/nix-release";
      flake = false;
    };
  };

  outputs = { nixpkgs, release, ... }:
    let
      targets = {
        x86_64-linux = "x86_64-unknown-linux-gnu";
        aarch64-linux = "aarch64-unknown-linux-gnu";
        x86_64-darwin = "x86_64-apple-darwin";
        aarch64-darwin = "aarch64-apple-darwin";
      };
      manifest = builtins.fromJSON (builtins.readFile (release + "/release.json"));
    in {
      packages = nixpkgs.lib.mapAttrs (system: target:
        let
          pkgs = import nixpkgs { inherit system; };
          name = "topiary-markdown-${target}.tar.gz";
          hash = manifest.hashes.${target};
          localArchive = release + "/${name}";
          src = pkgs.fetchurl {
            inherit name hash;
            url = if builtins.pathExists localArchive then "file://${localArchive}" else
              "https://github.com/${manifest.repository}/releases/download/${manifest.tag}/${name}";
          };
          binary = pkgs.stdenvNoCC.mkDerivation {
            pname = "topiary-markdown";
            version = pkgs.lib.removePrefix "v" manifest.tag;
            inherit src;
            sourceRoot = ".";
            strictDeps = true;
            dontBuild = true;
            dontStrip = true;
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.autoPatchelfHook ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.stdenv.cc.cc.lib ];

            installPhase = ''
              runHook preInstall
              install -Dm755 topiary-markdown "$out/bin/topiary-markdown"
              runHook postInstall
            '';

            meta = {
              description = "Markdown formatter built on Topiary (prebuilt binary)";
              mainProgram = "topiary-markdown";
            };
          };
        in {
          default = binary;
          topiary-markdown = binary;
        }) targets;
    };
}
