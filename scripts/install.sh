#!/usr/bin/env sh
# Install a verified cfbench Linux x86_64 release artifact without requiring root.
set -eu

repository=${CFBENCH_REPOSITORY:-fcendesu/cfbench}
version=${CFBENCH_VERSION:-latest}
install_dir=${CFBENCH_INSTALL_DIR:-$HOME/.local/bin}

fail() {
    printf '%s\n' "cfbench installer: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || fail "sha256sum or shasum is required"

[ "$(uname -s)" = "Linux" ] || fail "only Linux is supported by this installer; use cargo install --path . on other platforms"
[ "$(uname -m)" = "x86_64" ] || fail "only Linux x86_64 release artifacts are currently published"

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

artifact="cfbench-${version}-x86_64-unknown-linux-gnu.tar.gz"
checksum_file="cfbench-${version}-SHA256SUMS.txt"
base_url="https://github.com/$repository/releases/download/$version"
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-install.XXXXXX")
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT HUP INT TERM

curl -fsSL "$base_url/$artifact" -o "$tmp_dir/$artifact"
curl -fsSL "$base_url/$checksum_file" -o "$tmp_dir/$checksum_file"

expected=$(awk -v name="$artifact" '$2 == name { print $1; exit }' "$tmp_dir/$checksum_file")
[ -n "$expected" ] || fail "checksum for $artifact is missing"

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp_dir/$artifact" | awk '{ print $1 }')
else
    actual=$(shasum -a 256 "$tmp_dir/$artifact" | awk '{ print $1 }')
fi
[ "$actual" = "$expected" ] || fail "checksum verification failed"

tar -xzf "$tmp_dir/$artifact" -C "$tmp_dir" cfbench
mkdir -p "$install_dir"
install -m 0755 "$tmp_dir/cfbench" "$install_dir/cfbench"

printf 'Installed cfbench %s to %s/cfbench\n' "$version" "$install_dir"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) printf 'Add %s to PATH to run cfbench directly.\n' "$install_dir" ;;
esac
