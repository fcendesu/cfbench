#!/usr/bin/env sh
# Validate a complete cfbench release artifact set and write its checksum manifest.
set -eu

fail() {
    printf '%s\n' "cfbench release assembly: $*" >&2
    exit 1
}

[ "$#" -eq 2 ] || fail "usage: assemble-release.sh <vMAJOR.MINOR.PATCH> <dist-directory>"
tag=$1
[ -d "$2" ] || fail "distribution directory does not exist: $2"
dist=$(CDPATH= cd -- "$2" && pwd)

printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    || fail "expected a vMAJOR.MINOR.PATCH tag"
version=${tag#v}
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required"

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-release-assembly.XXXXXX")
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT HUP INT TERM

expected="$work_dir/expected"
actual="$work_dir/actual"
{
    printf '%s\n' "cfbench-$tag-aarch64-apple-darwin.tar.gz"
    printf '%s\n' "cfbench-$tag-aarch64-unknown-linux-gnu.tar.gz"
    printf '%s\n' "cfbench-$tag-x86_64-apple-darwin.tar.gz"
    printf '%s\n' "cfbench-$tag-x86_64-pc-windows-msvc.zip"
    printf '%s\n' "cfbench-$tag-x86_64-unknown-linux-gnu.tar.gz"
    printf '%s\n' "cfbench-$version-1.x86_64.rpm"
    printf '%s\n' "cfbench_${version}-1_amd64.deb"
} | LC_ALL=C sort > "$expected"

find "$dist" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; \
    | LC_ALL=C sort > "$actual"

if ! diff -u "$expected" "$actual" >&2; then
    fail "release artifact set does not match the expected files"
fi

manifest_name="cfbench-$tag-SHA256SUMS.txt"
manifest="$dist/$manifest_name"
: > "$manifest"
while IFS= read -r artifact; do
    (cd "$dist" && sha256sum "$artifact") >> "$manifest"
done < "$expected"

(cd "$dist" && sha256sum -c "$manifest_name" >/dev/null) \
    || fail "checksum manifest verification failed"
