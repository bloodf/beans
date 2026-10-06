#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-settings-layout.XXXXXX")
swiftc -parse-as-library -swift-version 5 \
  "$root/macos/Tests/SettingsLayout.swift" \
  "$root/macos/Sources/Lorca/Design/Controls.swift" \
  "$root/macos/Sources/Lorca/Settings/SettingsRows.swift" \
  -o "$output/settings-layout"
"$output/settings-layout"
