{
  description = "Tauri v2 music-player PoC devShell (NixOS)";

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
          pkgs.webkitgtk_4_1
          pkgs.gtk3
          pkgs.cairo
          pkgs.gdk-pixbuf
          pkgs.glib
          pkgs.pango
          pkgs.librsvg
          pkgs.libsoup_3
          pkgs.dbus
        ];
        shellHook = ''
          echo "Tauri devShell ready: provides webkit2gtk-4.1 and GTK system deps."
        '';
      };
    };
}