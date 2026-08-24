{
  description = "Hologram Live development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems =
        function:
        nixpkgs.lib.genAttrs systems (
          system:
          function {
            pkgs = import nixpkgs { inherit system; };
          }
        );
    in
    {
      devShells = forAllSystems (
        { pkgs }:
        {
          default = pkgs.mkShellNoCC {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              just
              watchexec
              nixfmt
              nodejs_22
            ];
          };
        }
      );

      formatter = forAllSystems ({ pkgs }: pkgs.nixfmt);
    };
}