Very simple CLI to install packages from https://github.com/microsoft/winget-pkgs using Wine.

Work in progress.

# Quick Start

```bash
cargo install --path .
# Set up local clone of https://github.com/microsoft/winget-pkgs
wine-winget init --repo-path ~/winget-pkgs
# Search for a package
wine-winget search llvm
# Install a package
wine-winget install llvm.llvm
```

# Requirements

- git in `PATH`
- wine in `PATH` (if running as a Linux executable, or specify using path to wine using `--wine`)
