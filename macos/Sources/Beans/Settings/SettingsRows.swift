import AppKit

/// Version and check status share a text column; the native action stays on the first line.
final class UpdateStatusRow: NSView {
    private let value = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    var onAction: (() -> Void)?

    init(key keyText: String, actionTitle: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let key = Build.label(keyText, font: .systemFont(ofSize: 12.5), lines: 0)
        let button = NSButton(title: actionTitle, target: self, action: #selector(tapped))
        button.isBordered = false
        button.font = .systemFont(ofSize: 12, weight: .medium)
        button.contentTintColor = .controlAccentColor
        button.translatesAutoresizingMaskIntoConstraints = false
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        value.setContentCompressionResistancePriority(.required, for: .vertical)
        addSubview(key)
        addSubview(value)
        addSubview(button)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 52),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            key.trailingAnchor.constraint(lessThanOrEqualTo: button.leadingAnchor, constant: -10),
            button.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            button.centerYAnchor.constraint(equalTo: key.centerYAnchor),
            value.leadingAnchor.constraint(equalTo: key.leadingAnchor),
            value.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            value.topAnchor.constraint(equalTo: key.bottomAnchor, constant: 4),
            value.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func setValue(_ text: String) { value.stringValue = text }
    @objc private func tapped() { onAction?() }
}

/// Key on the left, any control on the right, inside a section card.
final class AccessoryRow: NSView {
    init(key keyText: String, accessory: NSView) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let key = Build.label(keyText, font: .systemFont(ofSize: 12.5), lines: 0)
        key.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        key.setContentCompressionResistancePriority(.required, for: .vertical)
        accessory.translatesAutoresizingMaskIntoConstraints = false
        accessory.setAccessibilityLabel(keyText)
        addSubview(key)
        addSubview(accessory)
        if let popUp = accessory as? SettingsPopUpButton {
            popUp.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            popUp.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: 0.55).isActive = true
        }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 36),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            key.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 8),
            key.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),
            accessory.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            accessory.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 6),
            accessory.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -6),
            accessory.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            accessory.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}
