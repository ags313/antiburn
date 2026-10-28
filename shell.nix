{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  packages = with pkgs; [
    nodejs
    pnpm
    eslint
    prettier
    typescript
  ];
}
