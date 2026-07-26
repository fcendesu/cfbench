# Installation

## Linux x86_64 installer

The installer fetches the requested GitHub Release binary, verifies it using the release's SHA-256 checksum file, and installs it to `~/.local/bin` by default.

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

Pin a version:

```bash
CFBENCH_VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

Install to a custom directory:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sudo env CFBENCH_INSTALL_DIR=/usr/local/bin sh
```

The script supports only Linux x86_64 in 0.1.0. It requires `curl`, `tar`, and either `sha256sum` or `shasum`.

## Debian and RPM packages

Each GitHub Release supplies a `.deb`, `.rpm`, and `cfbench-<version>-SHA256SUMS.txt`. Download the package and checksum file from the same release, verify the checksum, and install with the distribution package manager.

Debian/Ubuntu example:

```bash
sha256sum -c cfbench-v0.1.0-SHA256SUMS.txt --ignore-missing
sudo apt install ./cfbench_0.1.0_amd64.deb
```

Fedora/RHEL example:

```bash
sha256sum -c cfbench-v0.1.0-SHA256SUMS.txt --ignore-missing
sudo dnf install ./cfbench-0.1.0-1.x86_64.rpm
```

## Source install

Rust 1.95 or newer is required.

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo install --path .
```
