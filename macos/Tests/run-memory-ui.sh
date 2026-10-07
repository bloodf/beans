#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-memory-ui.XXXXXX")
trap 'rm -rf "$output"' EXIT
swiftc -parse-as-library -swift-version 5 -D BEANS_MEMORY_UI_STANDALONE \
  "$root/macos/Sources/Beans/Model/MemoryService.swift" \
  "$root/macos/Sources/Beans/Model/MemorySetup.swift" \
  "$root/macos/Sources/Beans/Model/MemoryUI.swift" \
  "$root/macos/Sources/Beans/Design/Controls.swift" \
  "$root/macos/Sources/Beans/Sheets/SheetViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemoryFormViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemoryConnectionViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemoryEmbeddingViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemoryLocalAssetsViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemoryBotSetupViewController.swift" \
  "$root/macos/Sources/Beans/Settings/MemorySettingsViewController.swift" \
  "$root/macos/Sources/Beans/Sheets/MemoryAdvancedViewController.swift" \
  "$root/macos/Sources/Beans/Sheets/BotMemoryServiceViewController.swift" \
  "$root/macos/Tests/MemoryUIControllerChecks.swift" \
  "$root/macos/Tests/MemoryUIFlows.swift" \
  -o "$output/memory-ui"
"$output/memory-ui" "$@"
for fixture in MemoryServiceDraft MemorySetupDraft MemoryUIPreviewExpiry; do
  printf 'Compiling Foundation fixture: %s\n' "$fixture"
  swiftc -parse-as-library -swift-version 5 \
    "$root/macos/Sources/Beans/Model/MemoryService.swift" \
    "$root/macos/Sources/Beans/Model/MemorySetup.swift" \
    "$root/macos/Tests/$fixture.swift" -o "$output/$fixture"
  "$output/$fixture"
done
