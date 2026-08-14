#!/usr/bin/env sh
# Install a verified cfbench Linux release artifact.
set -eu

repository=${CFBENCH_REPOSITORY:-fcendesu/cfbench}
version=${CFBENCH_VERSION:-latest}
install_dir=${CFBENCH_INSTALL_DIR:-$HOME/.local/bin}
os_release=${CFBENCH_OS_RELEASE:-/etc/os-release}

fail() {
    printf '%s\n' "cfbench installer: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || fail "sha256sum or shasum is required"

[ "$(uname -s)" = "Linux" ] || fail "only Linux is supported by this installer; use cargo install --path . on other platforms"

machine=$(uname -m)
case "$machine" in
    x86_64|amd64)
        target=x86_64-unknown-linux-gnu
        native_packages=true
        ;;
    aarch64|arm64)
        target=aarch64-unknown-linux-gnu
        native_packages=false
        ;;
    *) fail "unsupported Linux architecture: $machine" ;;
esac

if [ "$version" = "latest" ]; then
    version=$(curl -fsSL "https://api.github.com/repos/$repository/releases/latest" \
        | sed -n 's/.*"tag_name": "\([^"]*\)".*/\1/p' \
        | head -n 1)
    [ -n "$version" ] || fail "could not determine the latest release tag"
fi

case "$version" in
    v*) ;;
    *) version="v$version" ;;
esac

package_type=standalone
if [ "$native_packages" = true ] && [ -r "$os_release" ]; then
    # shellcheck disable=SC1090
    . "$os_release"
    distribution_ids="${ID:-} ${ID_LIKE:-}"
    case " $distribution_ids " in
        *" debian "*|*" ubuntu "*) package_type=deb ;;
        *" fedora "*|*" rhel "*|*" centos "*) package_type=rpm ;;
    esac
fi

release_version=${version#v}
case "$package_type" in
    deb) artifact="cfbench_${release_version}-1_amd64.deb" ;;
    rpm) artifact="cfbench-${release_version}-1.x86_64.rpm" ;;
    *) artifact="cfbench-${version}-${target}.tar.gz" ;;
esac
checksum_file="cfbench-${version}-SHA256SUMS.txt"
base_url="https://github.com/$repository/releases/download/$version"
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-install.XXXXXX")
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT HUP INT TERM

printf 'Downloading cfbench %s (%s)...\n' "$version" "$artifact" >&2
curl -fsSL "$base_url/$artifact" -o "$tmp_dir/$artifact"
curl -fsSL "$base_url/$checksum_file" -o "$tmp_dir/$checksum_file"

expected=$(awk -v name="$artifact" '$2 == name { print $1; exit }' "$tmp_dir/$checksum_file")
[ -n "$expected" ] || fail "checksum for $artifact is missing"

printf 'Verifying %s...\n' "$artifact" >&2
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp_dir/$artifact" | awk '{ print $1 }')
else
    actual=$(shasum -a 256 "$tmp_dir/$artifact" | awk '{ print $1 }')
fi
[ "$actual" = "$expected" ] || fail "checksum verification failed"

run_privileged() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
        return
    fi

    command -v sudo >/dev/null 2>&1 || fail "sudo is required to install a system package"
    sudo "$@"
}

case "$package_type" in
    deb)
        command -v apt-get >/dev/null 2>&1 || fail "apt-get is required to install the Debian package"
        # APT drops privileges to the _apt user while reading local packages.
        # The artifact has already passed checksum verification at this point.
        chmod 755 "$tmp_dir"
        chmod 644 "$tmp_dir/$artifact"
        printf 'Installing %s...\n' "$artifact" >&2
        run_privileged apt-get install -y "$tmp_dir/$artifact"
        printf 'Installed cfbench %s using the Debian package.\n' "$version"
        ;;
    rpm)
        command -v dnf >/dev/null 2>&1 || fail "dnf is required to install the RPM package"
        printf 'Installing %s...\n' "$artifact" >&2
        run_privileged dnf install -y "$tmp_dir/$artifact"
        printf 'Installed cfbench %s using the RPM package.\n' "$version"
        ;;
    *)
        command -v tar >/dev/null 2>&1 || fail "tar is required for the standalone binary"
        printf 'Installing %s...\n' "$artifact" >&2
        tar -xzf "$tmp_dir/$artifact" -C "$tmp_dir" cfbench
        mkdir -p "$install_dir"
        install -m 0755 "$tmp_dir/cfbench" "$install_dir/cfbench"

        printf 'Installed cfbench %s to %s/cfbench\n' "$version" "$install_dir"
        case ":$PATH:" in
            *":$install_dir:"*) ;;
            *) printf 'Add %s to PATH to run cfbench directly.\n' "$install_dir" ;;
        esac
        ;;
esac
