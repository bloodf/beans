#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-avatar-renderer.XXXXXX")
swiftc -parse-as-library -swift-version 5 \
  "$root/macos/Tests/AvatarRenderer.swift" \
  "$root/macos/Sources/Lorca/Model/BotLook.swift" \
  "$root/macos/Sources/Lorca/Design/AvatarGeometry.swift" \
  "$root/macos/Sources/Lorca/Design/AvatarRenderLayer.swift" \
  -o "$output/avatar-renderer"
# The committed bundle the app copies into its resources.
"$output/avatar-renderer" "$root/packages/beans-blobatar/dist/blobatar.jsc.js"
