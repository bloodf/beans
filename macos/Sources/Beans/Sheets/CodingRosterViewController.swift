import AppKit

@MainActor
enum CodingRosterUI {
    static func show(store: AppStore, open: @escaping (String, String) -> Void) async throws {
        guard let owner = store.deviceNameOwner else { return }
        let data = try await store.codingRoster()
        guard store.deviceNameOwner == owner else { return }
        guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let agents = body["agents"] as? [[String: Any]] else { throw CocoaError(.coderReadCorrupt) }
        let picker = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 540, height: 28))
        for row in agents {
            let agent = row["agent"] as? [String: Any] ?? [:]
            let parts = [agent["title"], row["bot_name"], row["chat_title"], agent["device"], agent["state"], agent["branch"], agent["folder"]]
            picker.menu?.addItem(NSMenuItem(title: parts.compactMap { $0 as? String }.joined(separator: " · "), action: nil, keyEquivalent: ""))
        }
        picker.setAccessibilityLabel(L("Account coding agents"))
        let alert = NSAlert()
        alert.messageText = L("Coding agents")
        alert.informativeText = agents.isEmpty ? L("No coding agents") : L("Choose an agent to open its transcript.")
        alert.accessoryView = picker
        alert.addButton(withTitle: L("Close"))
        if !agents.isEmpty { alert.addButton(withTitle: L("Open transcript")) }
        alert.addButton(withTitle: L("Runner settings…"))
        let response = alert.runModal()
        guard store.deviceNameOwner == owner else { return }
        if !agents.isEmpty && response == .alertSecondButtonReturn {
            guard agents.indices.contains(picker.indexOfSelectedItem) else { return }
            guard agents.indices.contains(picker.indexOfSelectedItem) else { return }
            let row = agents[picker.indexOfSelectedItem]
            guard let chat = row["chat_id"] as? String, let message = row["message_id"] as? String else {
                throw CocoaError(.coderReadCorrupt)
            }
            open(chat, message)
        } else if response.rawValue == NSApplication.ModalResponse.alertFirstButtonReturn.rawValue + (agents.isEmpty ? 1 : 2) {
            try await CodingRunnerSettingsUI.show(store: store)
        }
    }
}
