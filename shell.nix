let
  # Download the Mozilla nixpkgs overlay for getting Rust nightly.
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz);
  pkgs = import <nixpkgs> {
    overlays = [ moz_overlay ];
  };
  lib = pkgs.lib;

  rust-nightly = (
    (pkgs.rustChannelOf { date = "2022-06-08"; channel = "nightly"; }).rust.override {
      extensions = [
        "rust-src"
        "rls-preview"
        "rust-analysis"
        "rustfmt-preview"
      ];
    }
  );
in
with pkgs; mkShell {
  buildInputs = [
    cargo
    openssl
    pkgconfig
    rnix-lsp
    rust-nightly
  ];
}

