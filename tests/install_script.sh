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
    -m) printf '%s\n' "${CFBENCH_TEST_MACHINE:-x86_64}" ;;
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
printf 'curl %s\n' "$url" >> "$CFBENCH_TEST_LOG"
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

cat > "$mock_bin/chmod" <<'EOF'
#!/usr/bin/env sh
printf 'chmod %s\n' "$*" >> "$CFBENCH_TEST_LOG"
EOF

cat > "$mock_bin/tar" <<'EOF'
#!/usr/bin/env sh
set -eu
destination=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -C) destination=$2; shift 2 ;;
        *) shift ;;
    esac
done
[ -n "$destination" ]
: > "$destination/cfbench"
printf 'tar %s\n' "$destination" >> "$CFBENCH_TEST_LOG"
EOF

cat > "$mock_bin/install" <<'EOF'
#!/usr/bin/env sh
printf 'install %s\n' "$*" >> "$CFBENCH_TEST_LOG"
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

assert_not_contains() {
    needle=$1
    file=$2
    if grep -F "$needle" "$file" >/dev/null; then
        printf 'did not expect %s in %s\n' "$needle" "$file" >&2
        exit 1
    fi
}

run_case() {
    case_name=$1
    machine=$2
    os_id=$3
    artifact=$4
    expected=$5
    os_release="$test_root/$case_name-os-release"
    log="$test_root/$case_name.log"
    status="$test_root/$case_name.status"
    install_dir="$test_root/$case_name-install"
    printf 'ID=%s\n' "$os_id" > "$os_release"
    : > "$log"

    PATH="$mock_bin:$PATH" \
        TMPDIR="$test_root/$case_name-tmp" \
        CFBENCH_INSTALL_DIR="$install_dir" \
        CFBENCH_OS_RELEASE="$os_release" \
        CFBENCH_VERSION=0.1.0 \
        CFBENCH_TEST_MACHINE="$machine" \
        CFBENCH_TEST_ARTIFACT="$artifact" \
        CFBENCH_TEST_LOG="$log" \
        sh "$project_root/scripts/install.sh" > /dev/null 2> "$status"

    assert_contains "$expected" "$log"
    assert_contains "Downloading cfbench v0.1.0 ($artifact)..." "$status"
    assert_contains "Verifying $artifact..." "$status"
    assert_contains "Installing $artifact..." "$status"

    case "$artifact" in
        *.deb)
            assert_contains "chmod 755 $test_root/$case_name-tmp/work" "$log"
            assert_contains "chmod 644 $test_root/$case_name-tmp/work/$artifact" "$log"
            ;;
    esac
}

run_case ubuntu-x86_64 x86_64 ubuntu cfbench_0.1.0-1_amd64.deb \
    "apt-get install -y $test_root/ubuntu-x86_64-tmp/work/cfbench_0.1.0-1_amd64.deb"
run_case fedora-x86_64 x86_64 fedora cfbench-0.1.0-1.x86_64.rpm \
    "dnf install -y $test_root/fedora-x86_64-tmp/work/cfbench-0.1.0-1.x86_64.rpm"
run_case ubuntu-amd64 amd64 ubuntu cfbench_0.1.0-1_amd64.deb \
    "apt-get install -y $test_root/ubuntu-amd64-tmp/work/cfbench_0.1.0-1_amd64.deb"
run_case ubuntu-aarch64 aarch64 ubuntu cfbench-v0.1.0-aarch64-unknown-linux-gnu.tar.gz \
    "install -m 0755 $test_root/ubuntu-aarch64-tmp/work/cfbench $test_root/ubuntu-aarch64-install/cfbench"
run_case fedora-arm64 arm64 fedora cfbench-v0.1.0-aarch64-unknown-linux-gnu.tar.gz \
    "install -m 0755 $test_root/fedora-arm64-tmp/work/cfbench $test_root/fedora-arm64-install/cfbench"

unsupported_log="$test_root/unsupported.log"
unsupported_status="$test_root/unsupported.status"
: > "$unsupported_log"
set +e
PATH="$mock_bin:$PATH" \
    TMPDIR="$test_root/unsupported-tmp" \
    CFBENCH_VERSION=0.1.0 \
    CFBENCH_TEST_MACHINE=riscv64 \
    CFBENCH_TEST_LOG="$unsupported_log" \
    sh "$project_root/scripts/install.sh" > /dev/null 2> "$unsupported_status"
result=$?
set -e
[ "$result" -ne 0 ] || {
    printf 'expected unsupported architecture to fail\n' >&2
    exit 1
}
assert_contains "unsupported Linux architecture: riscv64" "$unsupported_status"
assert_not_contains "curl " "$unsupported_log"
