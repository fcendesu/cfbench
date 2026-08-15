# Installation

## Linux installer

The installer supports Linux x86_64 and ARM64. It fetches a release artifact and
verifies it using the release's SHA-256 checksum file before installing it. On
x86_64 Debian/Ubuntu it installs the `.deb`; on x86_64 Fedora/RHEL-family
distributions it installs the `.rpm`. It invokes `sudo` only for those
package-manager installs, which prompts normally for a password when required.
Linux ARM64 and other supported Linux distributions receive the standalone
binary in `~/.local/bin` by default.

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | sh
```

Pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | CFBENCH_VERSION=0.4.0 sh
```

Linux ARM64 artifacts are available starting with v0.4.0.

For Linux ARM64 or any distribution using the standalone fallback, install to
a writable custom directory:

```bash
curl -fsSL https://raw.githubusercontent.com/fcendesu/cfbench/main/scripts/install.sh | CFBENCH_INSTALL_DIR="$HOME/bin" sh
```

The script requires `curl` and either `sha256sum` or `shasum`; standalone
installs also require `tar`. ARM64 `.deb` or `.rpm` packages are not published.

## Published binaries

| Platform | Rust target | Release format |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz`, `.deb`, `.rpm` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Releases produced by the current workflow publish
`cfbench-<tag>-SHA256SUMS.txt`, containing one SHA-256 digest for every binary
archive and Linux x86_64 package. Releases through v0.3.2 contain only the Linux
x86_64 artifacts; the five-platform artifact set begins with v0.4.0.

## Debian and RPM packages (Linux x86_64)

Each GitHub Release supplies a `.deb`, `.rpm`, and `cfbench-<version>-SHA256SUMS.txt`. Download the package and checksum file from the same release, verify the checksum, and install with the distribution package manager.

Debian/Ubuntu example:

```bash
sha256sum -c cfbench-v0.4.0-SHA256SUMS.txt --ignore-missing
sudo apt install ./cfbench_0.4.0-1_amd64.deb
```

Fedora/RHEL example:

```bash
sha256sum -c cfbench-v0.4.0-SHA256SUMS.txt --ignore-missing
sudo dnf install ./cfbench-0.4.0-1.x86_64.rpm
```

## Manual Linux and macOS archive installation

Choose the target from the table above. The example below uses Linux ARM64;
replace the target in both commands for another Unix platform.

```bash
tag=vX.Y.Z # Select a release that lists the target archive.
target=aarch64-unknown-linux-gnu
curl -LO "https://github.com/fcendesu/cfbench/releases/download/$tag/cfbench-$tag-$target.tar.gz"
curl -LO "https://github.com/fcendesu/cfbench/releases/download/$tag/cfbench-$tag-SHA256SUMS.txt"
sha256sum -c "cfbench-$tag-SHA256SUMS.txt" --ignore-missing
tar -xzf "cfbench-$tag-$target.tar.gz"
mkdir -p "$HOME/.local/bin"
install -m 0755 cfbench "$HOME/.local/bin/cfbench"
```

On macOS, use `shasum -a 256 -c` instead of `sha256sum -c`. The macOS binaries
are not code-signed or notarized.

## Manual Windows installation

Download these two files from the same GitHub Release:

```text
cfbench-<tag>-x86_64-pc-windows-msvc.zip
cfbench-<tag>-SHA256SUMS.txt
```

Confirm that `Get-FileHash -Algorithm SHA256` reports the digest listed for the
ZIP in the manifest, extract `cfbench.exe`, and place it in a directory on
`PATH`. The Windows binary is not code-signed.

## Shell completions and manual page

Debian and RPM packages install Bash, Zsh, and Fish completions and the
`cfbench(1)` manual in the conventional system directories. Standalone archives
contain `completions/` and `man/` alongside the binary; copy the desired files
to a user-level directory or generate fresh copies from the installed binary.

Bash:

```bash
mkdir -p "$HOME/.local/share/bash-completion/completions"
cfbench completions bash > "$HOME/.local/share/bash-completion/completions/cfbench"
```

Zsh:

```zsh
mkdir -p "$HOME/.zfunc"
cfbench completions zsh > "$HOME/.zfunc/_cfbench"
fpath=("$HOME/.zfunc" $fpath)
autoload -Uz compinit && compinit
```

Fish:

```fish
mkdir -p ~/.config/fish/completions
cfbench completions fish > ~/.config/fish/completions/cfbench.fish
```

PowerShell, current session:

```powershell
cfbench completions powershell | Out-String | Invoke-Expression
```

Generate and view the manual without installing it system-wide:

```bash
cfbench man > cfbench.1
man ./cfbench.1
```

## Source install

Rust 1.95 or newer is required.

Install the crates.io release:

```bash
cargo install cfbench
```

Or build directly from the repository:

```bash
git clone https://github.com/fcendesu/cfbench.git
cd cfbench
cargo install --path .
```
