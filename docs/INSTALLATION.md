# Installation

## Linux x86_64 installer

The installer fetches a release artifact and verifies it using the release's SHA-256 checksum file before installing it. On Debian/Ubuntu it installs the `.deb`; on Fedora/RHEL-family distributions it installs the `.rpm`. It invokes `sudo` only for those package-manager installs, which prompts normally for a password when required. Other supported Linux distributions receive the standalone binary in `~/.local/bin` by default.

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

Pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | CFBENCH_VERSION=0.2.0 sh
```

On distributions that use the standalone fallback, install to a writable custom directory:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | CFBENCH_INSTALL_DIR="$HOME/bin" sh
```

The script supports only Linux x86_64 in 0.2.0. It requires `curl` and either `sha256sum` or `shasum`; standalone installs also require `tar`.

## Debian and RPM packages

Each GitHub Release supplies a `.deb`, `.rpm`, and `cfbench-<version>-SHA256SUMS.txt`. Download the package and checksum file from the same release, verify the checksum, and install with the distribution package manager.

Debian/Ubuntu example:

```bash
sha256sum -c cfbench-v0.2.0-SHA256SUMS.txt --ignore-missing
sudo apt install ./cfbench_0.2.0-1_amd64.deb
```

Fedora/RHEL example:

```bash
sha256sum -c cfbench-v0.2.0-SHA256SUMS.txt --ignore-missing
sudo dnf install ./cfbench-0.2.0-1.x86_64.rpm
```

## Source install

Rust 1.95 or newer is required.

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo install --path .
```
