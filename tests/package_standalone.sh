#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-archive-test.XXXXXX")
cleanup() { rm -rf "$test_root"; }
trap cleanup EXIT HUP INT TERM

binary="$test_root/cfbench"
dist="$test_root/dist"
printf '#!/usr/bin/env sh\nexit 0\n' > "$binary"
chmod 755 "$binary"
mkdir "$dist"

sh "$project_root/scripts/package-standalone.sh" \
    "$binary" v9.8.7 x86_64-unknown-linux-gnu "$dist"

archive="$dist/cfbench-v9.8.7-x86_64-unknown-linux-gnu.tar.gz"
[ -f "$archive" ]

tar -tzf "$archive" | LC_ALL=C sort > "$test_root/actual"
cat > "$test_root/expected" <<'EOF'
cfbench
completions/
completions/_cfbench
completions/_cfbench.ps1
completions/cfbench.bash
completions/cfbench.fish
man/
man/cfbench.1
EOF

diff -u "$test_root/expected" "$test_root/actual"
