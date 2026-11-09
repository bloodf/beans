import AppKit

@MainActor
enum CodingRunnerSettingsUI {
    static func show(store: AppStore) async throws {
        guard let owner = store.deviceNameOwner else { return }
        let runners = store.devices.filter { $0.isRunner }
        guard !runners.isEmpty else { return }
        let picker = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 460, height: 28))
        runners.forEach { picker.menu?.addItem(NSMenuItem(title: $0.name, action: nil, keyEquivalent: "")) }
        picker.setAccessibilityLabel(L("Runner"))
        let choose = NSAlert()
        choose.messageText = L("Coding agent settings")
        choose.accessoryView = picker
        choose.addButton(withTitle: L("Cancel"))
        choose.addButton(withTitle: L("Edit"))
        guard choose.runModal() == .alertSecondButtonReturn, store.deviceNameOwner == owner else { return }
        guard runners.indices.contains(picker.indexOfSelectedItem) else { return }
        guard runners.indices.contains(picker.indexOfSelectedItem) else { return }
        let runner = runners[picker.indexOfSelectedItem]
        let data = try await store.codingSettings(runnerID: runner.id)
        guard store.deviceNameOwner == owner, store.devices.contains(where: { $0.id == runner.id && $0.isRunner }) else { return }
        guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              var agents = body["agents"] as? [[String: Any]],
              agents.allSatisfy({ $0["kind"] is String && $0["enabled"] is Bool }) else { throw CocoaError(.coderReadCorrupt) }
        while store.deviceNameOwner == owner {
            let stack = NSStackView()
            stack.orientation = .vertical
            stack.alignment = .leading
            var switches: [NSButton] = []
            for agent in agents {
                let title = [agent["name"] as? String ?? agent["kind"] as? String ?? "", agent["installed"] as? Bool == true ? (agent["version"] as? String ?? L("Installed")) : L("Not installed")].joined(separator: " · ")
                let button = NSButton(checkboxWithTitle: title, target: nil, action: nil)
                button.state = agent["enabled"] as? Bool == true ? .on : .off
                switches.append(button)
                stack.addArrangedSubview(button)
            }
            let order = NSPopUpButton(frame: .zero)
            for agent in agents { order.menu?.addItem(NSMenuItem(title: agent["name"] as? String ?? agent["kind"] as? String ?? "", action: nil, keyEquivalent: "")) }
            order.setAccessibilityLabel(L("Agent to move"))
            stack.addArrangedSubview(order)
            stack.frame = NSRect(x: 0, y: 0, width: 500, height: max(100, agents.count * 32 + 35))
            let alert = NSAlert()
            alert.messageText = runner.name + " · " + L("Coding agents")
            alert.informativeText = L("The first enabled, installed agent is the default. Move a selected agent up or down to change preference.")
            alert.accessoryView = stack
            alert.addButton(withTitle: L("Cancel"))
            alert.addButton(withTitle: L("Save"))
            alert.addButton(withTitle: L("Move up"))
            alert.addButton(withTitle: L("Move down"))
            let answer = alert.runModal()
            guard store.deviceNameOwner == owner, store.devices.contains(where: { $0.id == runner.id && $0.isRunner }) else { return }
            for index in agents.indices { agents[index]["enabled"] = switches[index].state == .on }
            if answer == .alertFirstButtonReturn { return }
            if answer == .alertSecondButtonReturn {
                let choices = try agents.map { agent -> (kind: String, enabled: Bool) in
                    guard let kind = agent["kind"] as? String, let enabled = agent["enabled"] as? Bool else { throw CocoaError(.coderReadCorrupt) }
                    return (kind, enabled)
                }
                _ = try await store.setCodingSettings(runnerID: runner.id, agents: choices)
                return
            }
            let index = order.indexOfSelectedItem
            let next = index + (answer == .alertThirdButtonReturn ? -1 : 1)
            if agents.indices.contains(index), agents.indices.contains(next) { agents.swapAt(index, next) }
        }
    }
}
