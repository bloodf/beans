#!/bin/sh
# Fast check of ComposerTextView.returnIntent alone. The whole-ComposerView check is
# ComposerReturnTests in the BeansTests target, run by the full native build.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-composer-return.XXXXXX")
trap 'rm -rf "$output"' EXIT
# ComposerTextView is the file's first type; the rest of ComposerView.swift needs the whole app.
sed '/^\/\/\/ Round symbol button/,$d' "$root/macos/Sources/Beans/Chat/ComposerView.swift" > "$output/ComposerTextView.swift"
swiftc -parse-as-library -swift-version 5 "$root/macos/Tests/ComposerReturn.swift" "$output/ComposerTextView.swift" -o "$output/composer-return"
HOME="$output" "$output/composer-return"
