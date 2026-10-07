#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-settings-layout.XXXXXX")
trap 'rm -rf "$output"' EXIT
swiftc -parse-as-library -swift-version 5 -D BEANS_CHROME_STANDALONE \
  "$root/macos/Tests/SettingsLayout.swift" \
  "$root/macos/Tests/Notifications/NativeChromeRenderingChecks.swift" \
  "$root/macos/Sources/Beans/Design/Controls.swift" \
  "$root/macos/Sources/Beans/Settings/SettingsRows.swift" \
  -o "$output/settings-layout"
"$output/settings-layout"
