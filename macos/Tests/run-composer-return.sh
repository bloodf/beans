#!/bin/sh
# Usage: run-composer-return.sh [BASELINE_FILE]
# BASELINE_FILE: ComposerView.swift from before the fix (git show <rev>:...); runs the old routing.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-composer-return.XXXXXX")
trap 'rm -rf "$output"' EXIT
source=${1:-"$root/macos/Sources/Beans/Chat/ComposerView.swift"}
flags=""
[ $# -gt 0 ] && flags="-D BASELINE"
# ComposerTextView is the file's first type; the rest of ComposerView.swift needs the whole app.
sed '/^\/\/\/ Round symbol button/,$d' "$source" > "$output/ComposerTextView.swift"
swiftc -parse-as-library -swift-version 5 $flags "$root/macos/Tests/ComposerReturn.swift" "$output/ComposerTextView.swift" -o "$output/composer-return"
HOME="$output" "$output/composer-return"
