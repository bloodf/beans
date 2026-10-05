import AppKit
import UniformTypeIdentifiers

/// A bot's generated portrait or an image of the user's own; custom image wins until removed.
final class BotLookViewController: SheetViewController {

    /// Longest side of a stored profile image. Small enough to sync in a moment, large enough
    /// for the biggest avatar any screen draws.
    static let imageSide: CGFloat = 512

    private enum ImageChange {
        case keep
        case remove
        case set(URL, NSImage)
    }

    private let store = AppStore.shared
    private let botID: Bot.ID
    private var imageChange: ImageChange = .keep

    private let preview = AvatarView(diameter: 72)
    private let chooseButton = NSButton()
    private let removeButton = NSButton()
    private let caption = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    init(botID: Bot.ID) {
        self.botID = botID
        super.init(
            title: L("Look"),
            subtitle: L("Your bot's portrait is generated from its ID. Use your own image instead; paired Devices see the same look."),
            width: 400
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var bot: Bot? { store.bot(botID) }

    /// Whether the saved look, with the pending change applied, has an image.
    private var hasImage: Bool {
        switch imageChange {
        case .keep: bot?.avatar != nil
        case .remove: false
        case .set: true
        }
    }

    override func loadView() {
        super.loadView()

        let previewRow = NSView()
        previewRow.translatesAutoresizingMaskIntoConstraints = false
        previewRow.addSubview(preview)
        NSLayoutConstraint.activate([
            preview.centerXAnchor.constraint(equalTo: previewRow.centerXAnchor),
            preview.topAnchor.constraint(equalTo: previewRow.topAnchor),
            preview.bottomAnchor.constraint(equalTo: previewRow.bottomAnchor),
        ])


        chooseButton.title = L("Choose Image…")
        chooseButton.bezelStyle = .rounded
        chooseButton.controlSize = .regular
        chooseButton.target = self
        chooseButton.action = #selector(chooseImage)
        chooseButton.translatesAutoresizingMaskIntoConstraints = false

        removeButton.title = L("Remove Image")
        removeButton.bezelStyle = .rounded
        removeButton.controlSize = .regular
        removeButton.target = self
        removeButton.action = #selector(removeImage)
        removeButton.translatesAutoresizingMaskIntoConstraints = false

        let imageRow = Build.stack([chooseButton, removeButton], orientation: .horizontal, spacing: 8)

        let rows: [NSView] = [previewRow, heading(L("Image")), imageRow, caption]
        for row in rows {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.setCustomSpacing(18, after: previewRow)
        contentStack.setCustomSpacing(6, after: rows[1])
        setButtons(confirm: L("Save"))
        refresh()
    }

    private func heading(_ text: String) -> NSView {
        Build.label(text.uppercased(), font: .systemFont(ofSize: 10, weight: .semibold), color: .tertiaryLabelColor)
    }


    private func refresh() {
        switch imageChange {
        case let .set(_, image):
            preview.content = .image(image)
        case .remove:
            preview.content = .bot(id: botID)
        case .keep:
            if let bot, let image = store.avatarImage(for: bot) {
                preview.content = .image(image)
            } else {
                preview.content = .bot(id: botID)
            }
        }



        removeButton.isHidden = !hasImage
        caption.stringValue = hasImage
            ? L("The image shows in place of the generated portrait. It is resized to %d px and shared encrypted, like an attachment.", Int(Self.imageSide))
            : L("Images are resized to %d px and shared encrypted, like an attachment.", Int(Self.imageSide))
        fitSheetToContent()
    }

    @objc private func chooseImage() {
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = L("Choose an image for this bot.")
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            guard let prepared = Self.prepare(imageAt: url) else {
                let alert = NSAlert()
                alert.messageText = L("That file could not be read as an image.")
                alert.runModal()
                return
            }
            imageChange = .set(prepared.url, prepared.image)
            refresh()
        }
    }

    @objc private func removeImage() {
        imageChange = .remove
        refresh()
    }

    override func confirmTapped() {
        guard bot != nil else {
            dismiss(nil)
            return
        }
        switch imageChange {
        case .keep:
            break
        case .remove:
            store.setBotAvatar(botID, fileURL: nil)
        case let .set(url, _):
            store.setBotAvatar(botID, fileURL: url)
        }
        dismiss(nil)
    }

    /// A square, center-cropped PNG of at most `imageSide` px, written to a temporary file the
    /// CLI copies into its store. The bytes that leave the computer are these, not the original.
    static func prepare(imageAt url: URL) -> (url: URL, image: NSImage)? {
        guard let source = NSImage(contentsOf: url), source.isValid else { return nil }
        guard let representation = source.representations.max(by: { $0.pixelsWide < $1.pixelsWide }) else { return nil }
        let pixelWidth = CGFloat(representation.pixelsWide)
        let pixelHeight = CGFloat(representation.pixelsHigh)
        guard pixelWidth > 0, pixelHeight > 0 else { return nil }

        let side = Int(min(imageSide, min(pixelWidth, pixelHeight)))
        guard
            let bitmap = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8, samplesPerPixel: 4,
                hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
        else { return nil }
        bitmap.size = NSSize(width: side, height: side)

        NSGraphicsContext.saveGraphicsState()
        guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
        NSGraphicsContext.current = context
        context.imageInterpolation = .high
        // Aspect-fill the square from the middle of the picture.
        let scale = CGFloat(side) / min(pixelWidth, pixelHeight)
        let drawn = NSSize(width: pixelWidth * scale, height: pixelHeight * scale)
        let origin = NSPoint(x: (CGFloat(side) - drawn.width) / 2, y: (CGFloat(side) - drawn.height) / 2)
        source.draw(in: NSRect(origin: origin, size: drawn), from: .zero, operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()

        guard let data = bitmap.representation(using: .png, properties: [:]) else { return nil }
        let target = FileManager.default.temporaryDirectory
            .appendingPathComponent("lorca-avatar-\(UUID().uuidString.lowercased().prefix(8)).png")
        do {
            try data.write(to: target, options: .atomic)
        } catch {
            return nil
        }
        let image = NSImage(size: bitmap.size)
        image.addRepresentation(bitmap)
        return (target, image)
    }
}
