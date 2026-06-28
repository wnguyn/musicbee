{
  description = "iced music-player devShell (NixOS)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [
          pkgs.fontconfig
          pkgs.libxkbcommon
          pkgs.vulkan-loader
          pkgs.wayland
          pkgs.xorg.libX11
          pkgs.xorg.libXcursor
          pkgs.xorg.libXi
          pkgs.xorg.libXrandr
          pkgs.dbus
        ];
        shellHook = ''
          echo "iced devShell ready: provides winit/wgpu desktop runtime deps."
        '';
      };
    };
}