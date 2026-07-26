#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/cfbench-install-test.XXXXXX")
cleanup() { rm -rf "$test_root"; }
trap cleanup EXIT HUP INT TERM

mock_bin="$test_root/bin"
mkdir "$mock_bin"

cat > "$mock_bin/uname" <<'EOF'
#!/usr/bin/env sh
case "$1" in
    -s) printf '%s\n' Linux ;;
    -m) printf '%s\n' x86_64 ;;
esac
EOF

cat > "$mock_bin/curl" <<'EOF'
#!/usr/bin/env sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o) output=$2; shift 2 ;;
        *) url=$1; shift ;;
    esac
done
name=${url##*/}
if [ "$name" = "cfbench-v0.1.0-SHA256SUMS.txt" ]; then
    printf 'verified  %s\n' "$CFBENCH_TEST_ARTIFACT" > "$output"
else
    : > "$output"
fi
EOF

cat > "$mock_bin/sha256sum" <<'EOF'
#!/usr/bin/env sh
printf 'verified  %s\n' "$1"
EOF

cat > "$mock_bin/sudo" <<'EOF'
#!/usr/bin/env sh
printf '%s\n' "$*" >> "$CFBENCH_TEST_LOG"
EOF

cat > "$mock_bin/apt-get" <<'EOF'
#!/usr/bin/env sh
exit 0
EOF

cat > "$mock_bin/dnf" <<'EOF'
#!/usr/bin/env sh
exit 0
EOF

cat > "$mock_bin/mktemp" <<'EOF'
#!/usr/bin/env sh
mkdir -p "$TMPDIR/work"
printf '%s\n' "$TMPDIR/work"
EOF

chmod 755 "$mock_bin"/*

assert_contains() {
    needle=$1
    file=$2
    grep -F "$needle" "$file" >/dev/null || {
        printf 'expected %s in %s\n' "$needle" "$file" >&2
        exit 1
    }
}

run_case() {
    os_id=$1
    artifact=$2
    expected=$3
    os_release="$test_root/$os_id-os-release"
    log="$test_root/$os_id.log"
    status="$test_root/$os_id.status"
    printf 'ID=%s\n' "$os_id" > "$os_release"

    PATH="$mock_bin:$PATH" \
        TMPDIR="$test_root/$os_id-tmp" \
        CFBENCH_OS_RELEASE="$os_release" \
        CFBENCH_VERSION=0.1.0 \
        CFBENCH_TEST_ARTIFACT="$artifact" \
        CFBENCH_TEST_LOG="$log" \
        sh "$project_root/scripts/install.sh" > /dev/null 2> "$status"

    assert_contains "$expected" "$log"
    assert_contains "Downloading cfbench v0.1.0 ($artifact)..." "$status"
    assert_contains "Verifying $artifact..." "$status"
    assert_contains "Installing $artifact..." "$status"
}

run_case ubuntu cfbench_0.1.0-1_amd64.deb "apt-get install -y $test_root/ubuntu-tmp/work/cfbench_0.1.0-1_amd64.deb"
run_case fedora cfbench-0.1.0-1.x86_64.rpm "dnf install -y $test_root/fedora-tmp/work/cfbench-0.1.0-1.x86_64.rpm"
