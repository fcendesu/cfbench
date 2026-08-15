#!/usr/bin/env sh
# Build a Unix standalone archive containing cfbench and its generated CLI assets.
set -eu

fail() {
    printf '%s\n' "cfbench standalone packaging: $*" >&2
    exit 1
}

[ "$#" -eq 4 ] || fail "usage: package-standalone.sh <binary> <tag> <target> <dist-directory>"

binary=$1
tag=$2
target=$3
[ -f "$binary" ] || fail "binary does not exist: $binary"
[ -d "$4" ] || fail "distribution directory does not exist: $4"
dist=$(CDPATH= cd -- "$4" && pwd)
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    || fail "expected a vMAJOR.MINOR.PATCH tag"
[ -n "$target" ] || fail "target must not be empty"

for asset in \
    assets/completions/cfbench.bash \
    assets/completions/_cfbench \
    assets/completions/cfbench.fish \
    assets/completions/_cfbench.ps1 \
    assets/man/cfbench.1
do
    [ -f "$project_root/$asset" ] || fail "generated asset does not exist: $asset"
done

stage=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-standalone.XXXXXX")
cleanup() { rm -rf "$stage"; }
trap cleanup EXIT HUP INT TERM

mkdir -p "$stage/completions" "$stage/man"
cp "$binary" "$stage/cfbench"
cp "$project_root"/assets/completions/* "$stage/completions/"
cp "$project_root/assets/man/cfbench.1" "$stage/man/"

tar -C "$stage" -czf "$dist/cfbench-$tag-$target.tar.gz" \
    cfbench completions man
