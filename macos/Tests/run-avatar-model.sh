#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
output=$(mktemp -d "${TMPDIR:-/tmp}/beans-avatar-model.XXXXXX")
# Models.swift's unrelated display helpers need only these names.
cat > "$output/stubs.swift" <<'EOF'
import AppKit
func L(_ text: String, _ arguments: CVarArg...) -> String { text }
func L(_ text: String, context: String) -> String { text }
enum Accent: String, Hashable { case indigo }
final class AppStore { static let shared = AppStore(); func credential(for kind: ProviderCredential.Kind) -> ProviderCredential? { nil } }
struct RenderedMessage { init(_ text: String, textColor: NSColor) { plainText = text }; let plainText: String }
enum Format {
    static func daySeparator(_ date: Date) -> String { "" }
    static func time(_ date: Date) -> String { "" }
    static func formatter(_ format: String) -> DateFormatter { DateFormatter() }
}
EOF
swiftc -parse-as-library -swift-version 5 \
  "$root/macos/Tests/AvatarModel.swift" \
  "$root/macos/Sources/Lorca/Model/Models.swift" \
  "$root/macos/Sources/Lorca/Model/BotLook.swift" \
  "$root/macos/Sources/Lorca/Model/BotActivity.swift" \
  "$root/macos/Sources/Lorca/Sheets/BotLookDraft.swift" \
  "$output/stubs.swift" \
  -o "$output/avatar-model"
"$output/avatar-model"
