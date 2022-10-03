{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  name = "rust-env";
  nativeBuildInputs = with pkgs; [
    rustc cargo freetype pkgconfig expat
  ];

  buildInputs = with pkgs; [ freetype expat fontconfig ];

  RUST_BACKTRACE = 1;
}
