#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-memory-ui.XXXXXX")
trap 'rm -rf "$output"' EXIT
swiftc -parse-as-library -swift-version 5 -D BEANS_MEMORY_UI_STANDALONE \
  "$root/macos/Sources/Lorca/Model/MemoryService.swift" \
  "$root/macos/Sources/Lorca/Model/MemorySetup.swift" \
  "$root/macos/Sources/Lorca/Model/MemoryUI.swift" \
  "$root/macos/Sources/Lorca/Design/Controls.swift" \
  "$root/macos/Sources/Lorca/Sheets/SheetViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemoryFormViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemoryConnectionViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemoryEmbeddingViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemoryLocalAssetsViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemoryBotSetupViewController.swift" \
  "$root/macos/Sources/Lorca/Settings/MemorySettingsViewController.swift" \
  "$root/macos/Sources/Lorca/Sheets/MemoryAdvancedViewController.swift" \
  "$root/macos/Sources/Lorca/Sheets/BotMemoryServiceViewController.swift" \
  "$root/macos/Tests/MemoryUIControllerChecks.swift" \
  "$root/macos/Tests/MemoryUIFlows.swift" \
  -o "$output/memory-ui"
"$output/memory-ui" "$@"
for fixture in MemoryServiceDraft MemorySetupDraft MemoryUIPreviewExpiry; do
  printf 'Compiling Foundation fixture: %s\n' "$fixture"
  swiftc -parse-as-library -swift-version 5 \
    "$root/macos/Sources/Lorca/Model/MemoryService.swift" \
    "$root/macos/Sources/Lorca/Model/MemorySetup.swift" \
    "$root/macos/Tests/$fixture.swift" -o "$output/$fixture"
  "$output/$fixture"
done
