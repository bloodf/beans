import AppKit

/// Edits only displayed fields. Unknown keys and masked secret placeholders retain their original values.
@MainActor
final class ReviewDraftFields {
    let view = NSStackView()
    private let original: [String: Any]
    private var fields: [(String, NSTextField)] = []
    private var attachments: NSTextView?
    private var longFields: [(String, NSTextView)] = []
    private var text: NSTextView?
    private var generic: NSTextView?
    private let plugin: Bool

    init(payload: [String: Any], editable: Bool) throws {
        original = payload
        plugin = payload["kind"] as? String == "plugin"
        view.orientation = .vertical
        view.alignment = .leading
        view.spacing = 8
        if payload["kind"] as? String == "draft", let value = payload["text"] as? String {
            text = area(label: L("Draft text"), value: value, editable: editable)
        } else if plugin, let args = payload["arguments"] as? [String: Any] {
            for key in args.keys.sorted() {
                guard key != "attachments", let value = args[key] as? String else { continue }
                // Credentials are not editable in a review. Preserve their exact placeholder bytes.
                let sensitive = ["secret", "password", "token", "authorization", "api_key"].contains { key.lowercased().contains($0) }
                if sensitive {
                    view.addArrangedSubview(NSTextField(labelWithString: key + ": ••••••••"))
                    continue
                }
                if ["body", "text", "content", "message", "html"].contains(key.lowercased()) {
                    longFields.append((key, area(label: key, value: value, editable: editable)))
                    continue
                }
                view.addArrangedSubview(NSTextField(labelWithString: key))
                let field = NSTextField(string: value)
                field.isEditable = editable
                field.setAccessibilityLabel(key)
                field.widthAnchor.constraint(equalToConstant: 510).isActive = true
                view.addArrangedSubview(field)
                fields.append((key, field))
            }
            let details = try JSONSerialization.data(withJSONObject: Self.masked(payload), options: [.prettyPrinted, .sortedKeys])
            _ = area(label: L("Complete proposal (read only, secrets masked)"), value: String(decoding: details, as: UTF8.self), editable: false)
            if let value = args["attachments"] {
                let bytes = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed])
                attachments = area(label: L("Attachments (JSON)"), value: String(decoding: bytes, as: UTF8.self), editable: editable)
            }
        } else {
            let bytes = try JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted, .sortedKeys])
            generic = area(label: L("Review payload (JSON)"), value: String(decoding: bytes, as: UTF8.self), editable: editable)
        }
        view.frame = NSRect(x: 0, y: 0, width: 540, height: 340)
    }

    private func area(label: String, value: String, editable: Bool) -> NSTextView {
        view.addArrangedSubview(NSTextField(labelWithString: label))
        let editor = NSTextView(frame: NSRect(x: 0, y: 0, width: 510, height: 180))
        editor.isRichText = false
        editor.isEditable = editable
        editor.string = value
        editor.setAccessibilityLabel(label)
        let scroll = NSScrollView(frame: editor.frame)
        scroll.hasVerticalScroller = true
        scroll.documentView = editor
        scroll.widthAnchor.constraint(equalToConstant: 510).isActive = true
        scroll.heightAnchor.constraint(equalToConstant: 180).isActive = true
        view.addArrangedSubview(scroll)
        return editor
    }

    private static func masked(_ value: Any) -> Any {
        if let object = value as? [String: Any] {
            return object.mapValues { masked($0) }
                .merging(object.filter { key, _ in
                    ["secret", "password", "token", "authorization", "api_key"].contains { key.lowercased().contains($0) }
                }.mapValues { _ in "••••••••" as Any }) { _, masked in masked }
        }
        if let array = value as? [Any] { return array.map { masked($0) } }
        return value
    }

    func payload() throws -> [String: Any] {
        if let generic {
            guard let value = try JSONSerialization.jsonObject(with: Data(generic.string.utf8)) as? [String: Any] else { throw CocoaError(.coderReadCorrupt) }
            return value
        }
        var result = original
        if let text { result["text"] = text.string }
        if plugin, var args = original["arguments"] as? [String: Any] {
            for (key, field) in fields { args[key] = field.stringValue }
            for (key, field) in longFields { args[key] = field.string }
            if let attachments {
                let value = try JSONSerialization.jsonObject(with: Data(attachments.string.utf8), options: [.fragmentsAllowed])
                args["attachments"] = value
            }
            result["arguments"] = args
        }
        return result
    }
}
