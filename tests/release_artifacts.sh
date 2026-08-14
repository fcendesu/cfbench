#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-release-test.XXXXXX")
cleanup() { rm -rf "$test_root"; }
trap cleanup EXIT HUP INT TERM

tag=v9.8.7
version=9.8.7

create_fixture() {
    directory=$1
    mkdir -p "$directory"
    for artifact in \
        "cfbench-$tag-aarch64-apple-darwin.tar.gz" \
        "cfbench-$tag-aarch64-unknown-linux-gnu.tar.gz" \
        "cfbench-$tag-x86_64-apple-darwin.tar.gz" \
        "cfbench-$tag-x86_64-pc-windows-msvc.zip" \
        "cfbench-$tag-x86_64-unknown-linux-gnu.tar.gz" \
        "cfbench-$version-1.x86_64.rpm" \
        "cfbench_${version}-1_amd64.deb"
    do
        printf '%s\n' "$artifact" > "$directory/$artifact"
    done
}

assert_fails() {
    directory=$1
    stderr_file=$2
    set +e
    sh "$project_root/scripts/assemble-release.sh" "$tag" "$directory" \
        > /dev/null 2> "$stderr_file"
    result=$?
    set -e
    [ "$result" -ne 0 ] || {
        printf 'expected artifact assembly to fail for %s\n' "$directory" >&2
        exit 1
    }
    grep -F "release artifact set does not match" "$stderr_file" >/dev/null
}

success_dist="$test_root/success"
create_fixture "$success_dist"
sh "$project_root/scripts/assemble-release.sh" "$tag" "$success_dist"
manifest="$success_dist/cfbench-$tag-SHA256SUMS.txt"
[ -f "$manifest" ]
[ "$(wc -l < "$manifest" | tr -d ' ')" -eq 7 ]
(cd "$success_dist" && sha256sum -c "${manifest##*/}" >/dev/null)

missing_dist="$test_root/missing"
create_fixture "$missing_dist"
rm "$missing_dist/cfbench-$tag-x86_64-pc-windows-msvc.zip"
assert_fails "$missing_dist" "$test_root/missing.stderr"

unexpected_dist="$test_root/unexpected"
create_fixture "$unexpected_dist"
: > "$unexpected_dist/unexpected.txt"
assert_fails "$unexpected_dist" "$test_root/unexpected.stderr"
