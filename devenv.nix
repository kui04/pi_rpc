{ pkgs, ... }:

{
  packages = [ pkgs.git ];

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "latest";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
    mold.enable = true;
  };

  enterShell = ''
    rustc --version
    cargo --version
  '';

  enterTest = ''
    echo "Running tests"
    rustc --version
    cargo --version
    echo "$RUSTFLAGS" | grep -q "fuse-ld=mold"
  '';
}
