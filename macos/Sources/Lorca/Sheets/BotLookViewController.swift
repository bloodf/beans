import AppKit
import UniformTypeIdentifiers

/// A bot's generated portrait, per activity state, and an image of the user's own. Edits stay in
/// a draft until Save; the image, when there is one, wins until it is removed.
final class BotLookViewController: SheetViewController {

    /// Longest side of a stored profile image. Small enough to sync in a moment, large enough
    /// for the biggest avatar any screen draws.
    static let imageSide: CGFloat = 512
    /// How long the preview shows each state while it cycles through them.
    static let cycleInterval: TimeInterval = 1.6

    private enum ImageChange {
        case keep
        case remove
        case set(URL, NSImage)
    }

    private let store = AppStore.shared
    private let botID: Bot.ID
    private let save: @MainActor (Bot.ID, BotLookChange, BotPhotoChange) async throws -> Void
    private var imageChange: ImageChange = .keep
    private var draft: BotLookDraft
    private var isSaving = false

    override var canDismiss: Bool { !isSaving }

    private let preview = AvatarView(diameter: 96)
    private let photo = AvatarView(diameter: 40)
    private let photoNote = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let cycleButton = NSButton(checkboxWithTitle: L("Preview activity"), target: nil, action: nil)
    private let stateLabel = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private var cycleTimer: Timer?
    private var cycleIndex = 0

    private let target = SettingsPopUpButton()
    private let shape = SettingsPopUpButton()
    private let expression = SettingsPopUpButton()
    private let background = SettingsPopUpButton()
    private let tone = SettingsPopUpButton()
    private let motion = SettingsPopUpButton()
    private let customHue = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    private let hue = NSSlider(value: 0, minValue: 0, maxValue: 359, target: nil, action: nil)
    private let colorWells: [(key: WritableKeyPath<BotAppearance.Palette, String?>, well: NSColorWell, clear: NSButton)] = [
        (\.head, NSColorWell(), NSButton()), (\.eye, NSColorWell(), NSButton()), (\.bg, NSColorWell(), NSButton()),
    ]
    private let contrast = Build.label("", font: Theme.Font.caption, color: .systemOrange, lines: 0)
    private let resetStateButton = NSButton()
    private let status = Build.label("", font: Theme.Font.caption, color: .systemRed, lines: 0)

    private let chooseButton = NSButton()
    private let removeButton = NSButton()
    private let caption = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    init(
        botID: Bot.ID,
        save: @escaping @MainActor (Bot.ID, BotLookChange, BotPhotoChange) async throws -> Void = {
            try await AppStore.shared.saveBotLook($0, look: $1, photo: $2)
        }
    ) {
        self.botID = botID
        self.save = save
        draft = BotLookDraft(saved: AppStore.shared.bot(botID)?.look)
        super.init(
            title: L("Look"),
            subtitle: L("Shape, face, colors, and motion for each thing your bot does. Paired Devices see the same look."),
            width: 520
        )
    }

    deinit { cycleTimer?.invalidate() }

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

    private var pendingImage: NSImage? {
        switch imageChange {
        case let .set(_, image): image
        case .remove: nil
        case .keep: bot.flatMap { store.avatarImage(for: $0) }
        }
    }

    // MARK: - Layout

    override func loadView() {
        super.loadView()
        for name in [AvatarClock.environmentDidChange, NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(refreshCyclePolicy), name: name, object: nil)
        }

        let previewRow = NSView()
        previewRow.translatesAutoresizingMaskIntoConstraints = false
        let photoStack = Build.stack([photo, photoNote], orientation: .horizontal, spacing: 8)
        let previewSide = Build.stack([cycleButton, stateLabel, photoStack], spacing: 6)
        previewRow.addSubview(preview)
        previewRow.addSubview(previewSide)
        NSLayoutConstraint.activate([
            preview.leadingAnchor.constraint(equalTo: previewRow.leadingAnchor),
            preview.topAnchor.constraint(equalTo: previewRow.topAnchor),
            preview.bottomAnchor.constraint(equalTo: previewRow.bottomAnchor),
            previewSide.leadingAnchor.constraint(equalTo: preview.trailingAnchor, constant: 18),
            previewSide.trailingAnchor.constraint(lessThanOrEqualTo: previewRow.trailingAnchor),
            previewSide.centerYAnchor.constraint(equalTo: preview.centerYAnchor),
            photoNote.widthAnchor.constraint(lessThanOrEqualToConstant: 300),
        ])
        preview.setAccessibilityElement(true)
        preview.setAccessibilityRole(.image)
        cycleButton.target = self
        cycleButton.action = #selector(toggleCycle)

        target.addItem(withTitle: L("Every state"))
        for state in BotAvatarState.allCases { target.addItem(withTitle: Self.title(of: state)) }
        fill(shape, BotAppearance.Shape.allCases.map { Self.title(of: $0) })
        fill(expression, BotAppearance.Expression.allCases.map { Self.title(of: $0) })
        fill(background, BotAppearance.Background.allCases.map { Self.title(of: $0) })
        fill(tone, BotAppearance.Tone.allCases.map { Self.title(of: $0) })
        fill(motion, [L("On"), L("Off")])
        for control in [target, shape, expression, background, tone, motion] {
            control.target = self
            control.action = #selector(controlChanged(_:))
        }

        customHue.target = self
        customHue.action = #selector(controlChanged(_:))
        customHue.setAccessibilityLabel(L("Custom hue"))
        hue.target = self
        hue.action = #selector(controlChanged(_:))
        hue.isContinuous = true
        hue.setAccessibilityLabel(L("Hue"))
        hue.widthAnchor.constraint(equalToConstant: 180).isActive = true

        let colorNames = [L("Body color"), L("Eye color"), L("Background color")]
        var colorRows: [NSView] = []
        for (index, entry) in colorWells.enumerated() {
            entry.well.target = self
            entry.well.action = #selector(colorChanged(_:))
            entry.well.setAccessibilityLabel(colorNames[index])
            entry.well.widthAnchor.constraint(equalToConstant: 44).isActive = true
            entry.clear.title = L("Automatic")
            entry.clear.bezelStyle = .rounded
            entry.clear.controlSize = .small
            entry.clear.tag = index
            entry.clear.target = self
            entry.clear.action = #selector(clearColor(_:))
            entry.clear.setAccessibilityLabel(L("%@: automatic", colorNames[index]))
            colorRows.append(AccessoryRow(key: colorNames[index], accessory: Build.stack([entry.clear, entry.well], orientation: .horizontal, spacing: 8)))
        }

        resetStateButton.title = L("Use Every State")
        resetStateButton.bezelStyle = .rounded
        resetStateButton.target = self
        resetStateButton.action = #selector(resetState)
        let shuffleButton = NSButton(title: L("Shuffle"), target: self, action: #selector(shuffle))
        shuffleButton.bezelStyle = .rounded
        let resetButton = NSButton(title: L("Restore Default"), target: self, action: #selector(resetAll))
        resetButton.bezelStyle = .rounded
        let editRow = Build.stack([shuffleButton, resetStateButton, resetButton], orientation: .horizontal, spacing: 8)

        chooseButton.title = L("Choose Image…")
        chooseButton.bezelStyle = .rounded
        chooseButton.target = self
        chooseButton.action = #selector(chooseImage)
        removeButton.title = L("Remove Image")
        removeButton.bezelStyle = .rounded
        removeButton.target = self
        removeButton.action = #selector(removeImage)
        let imageRow = Build.stack([chooseButton, removeButton], orientation: .horizontal, spacing: 8)

        let hueRow = AccessoryRow(key: L("Hue"), accessory: Build.stack([customHue, hue], orientation: .horizontal, spacing: 8))
        let rows: [NSView] = [
            previewRow,
            AccessoryRow(key: L("Edit"), accessory: target),
            AccessoryRow(key: L("Shape"), accessory: shape),
            AccessoryRow(key: L("Expression"), accessory: expression),
            AccessoryRow(key: L("Background"), accessory: background),
            AccessoryRow(key: L("Tone"), accessory: tone),
            hueRow,
        ] + colorRows + [
            AccessoryRow(key: L("Motion"), accessory: motion),
            contrast, editRow, heading(L("Image")), imageRow, caption, status,
        ]
        for row in rows {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.spacing = 4
        contentStack.setCustomSpacing(18, after: previewRow)
        contentStack.setCustomSpacing(12, after: editRow)
        setButtons(confirm: L("Save"))
        refresh()
    }

    override func viewWillDisappear() {
        super.viewWillDisappear()
        stopCycle()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        refreshCyclePolicy()
    }

    @objc private func refreshCyclePolicy() {
        let allowed = AvatarClock.isVisible(view) && !AvatarClock.shared.reducesMotion
        cycleButton.isEnabled = allowed
        if !allowed {
            stopCycle()
            refreshPreview()
        }
    }

    private func heading(_ text: String) -> NSView {
        Build.label(text.uppercased(), font: .systemFont(ofSize: 10, weight: .semibold), color: .tertiaryLabelColor)
    }

    /// The first item is "nothing set": the seeded value in the base, the base's value in a state.
    private func fill(_ popUp: SettingsPopUpButton, _ titles: [String]) {
        popUp.addItem(withTitle: "")
        popUp.addItems(withTitles: titles)
    }

    // MARK: - State

    private func refresh() {
        let editing = draft.editing
        let own = draft.own
        let inherit = editing == nil ? L("Automatic") : L("Same as every state")
        for popUp in [shape, expression, background, tone] { popUp.item(at: 0)?.title = inherit }
        target.selectItem(at: editing.map { BotAvatarState.allCases.firstIndex(of: $0)! + 1 } ?? 0)
        select(shape, own.shape, in: BotAppearance.Shape.allCases)
        select(expression, own.expression, in: BotAppearance.Expression.allCases)
        select(background, own.background, in: BotAppearance.Background.allCases)
        select(tone, own.tone, in: BotAppearance.Tone.allCases)

        // Motion has no seeded value: the base is on unless turned off; a state may inherit.
        motion.item(at: 0)?.isHidden = editing == nil
        motion.item(at: 0)?.title = L("Same as every state")
        if let value = own.motion {
            motion.selectItem(at: value ? 1 : 2)
        } else {
            motion.selectItem(at: editing == nil ? 1 : 0)
        }

        customHue.state = own.hue == nil ? .off : .on
        hue.isEnabled = own.hue != nil
        hue.doubleValue = draft.shown.hue ?? resolvedHue
        for entry in colorWells {
            let value = own.palette[keyPath: entry.key]
            entry.clear.isEnabled = value != nil
            entry.well.color = color(value ?? resolvedColor(entry.key))
        }
        resetStateButton.isHidden = editing == nil
        resetStateButton.isEnabled = editing.map { draft.overrides($0) } ?? false

        refreshPreview()
        refreshContrast()

        let image = pendingImage
        photo.isHidden = image == nil
        photoNote.isHidden = !hasImage
        if let image { photo.content = .image(image) }
        photoNote.stringValue = L("Your image shows in place of this portrait until you remove it.")
        removeButton.isHidden = !hasImage
        caption.stringValue = L("Images are resized to %d px and shared encrypted, like an attachment.", Int(Self.imageSide))
        confirmButton.isEnabled = !isSaving
        cancelButton?.isEnabled = !isSaving
        fitSheetToContent()
    }

    private func select<T: Equatable>(_ popUp: SettingsPopUpButton, _ value: T?, in all: [T]) {
        popUp.selectItem(at: value.flatMap { all.firstIndex(of: $0) }.map { $0 + 1 } ?? 0)
    }

    /// The state the preview shows: the cycle's, else the one being edited, else idle.
    private var previewState: BotAvatarState {
        cycleTimer != nil ? BotAvatarState.allCases[cycleIndex] : draft.editing ?? .idle
    }

    private func refreshPreview() {
        let state = previewState
        preview.content = .bot(id: botID, look: draft.result, state: state)
        stateLabel.stringValue = Self.title(of: state)
        preview.setAccessibilityLabel(L("Portrait preview: %@", Self.title(of: state)))
    }

    private var resolved: (appearance: [String: Any], eyeOnHead: Double, headOnBg: Double) {
        AvatarGeometryBridge.resolved(id: botID, look: draft.result, state: draft.editing ?? .idle)
    }

    private var resolvedHue: Double { resolved.appearance["hue"] as? Double ?? 0 }

    private func resolvedColor(_ key: WritableKeyPath<BotAppearance.Palette, String?>) -> String? {
        guard let palette = resolved.appearance["palette"] as? [String: String] else { return nil }
        let name = key == \.head ? "head" : key == \.eye ? "eye" : "bg"
        return palette[name]
    }

    /// Warns about hard-to-see choices without changing them.
    private func refreshContrast() {
        let (appearance, eyeOnHead, headOnBg) = resolved
        var warnings: [String] = []
        if eyeOnHead < 3 { warnings.append(L("The eyes are hard to see on the body (contrast %@:1).", String(format: "%.1f", eyeOnHead))) }
        if (appearance["background"] as? String ?? "none") != "none", headOnBg < 1.5 {
            warnings.append(L("The body is hard to see on the background (contrast %@:1).", String(format: "%.1f", headOnBg)))
        }
        contrast.stringValue = warnings.joined(separator: "\n")
        contrast.isHidden = warnings.isEmpty
    }

    private func color(_ hex: String?) -> NSColor {
        hex.map { NSColor(cgColor: AvatarColors.color($0)) ?? .gray } ?? .gray
    }

    private static func hex(_ color: NSColor) -> String? {
        guard let rgb = color.usingColorSpace(.sRGB) else { return nil }
        let value = [rgb.redComponent, rgb.greenComponent, rgb.blueComponent].map { Int(($0 * 255).rounded()).clamped(to: 0...255) }
        return String(format: "#%02X%02X%02X", value[0], value[1], value[2])
    }

    // MARK: - Actions

    @objc private func controlChanged(_ sender: NSControl) {
        if sender === target {
            draft.editing = target.indexOfSelectedItem == 0 ? nil : BotAvatarState.allCases[target.indexOfSelectedItem - 1]
            refresh()
            return
        }
        func choice<T>(_ popUp: SettingsPopUpButton, _ all: [T]) -> T? {
            popUp.indexOfSelectedItem > 0 ? all[popUp.indexOfSelectedItem - 1] : nil
        }
        let isBase = draft.editing == nil
        draft.update { appearance in
            if sender === shape {
                appearance.shape = choice(shape, BotAppearance.Shape.allCases)
            } else if sender === expression {
                appearance.expression = choice(expression, BotAppearance.Expression.allCases)
            } else if sender === background {
                appearance.background = choice(background, BotAppearance.Background.allCases)
            } else if sender === tone {
                appearance.tone = choice(tone, BotAppearance.Tone.allCases)
            } else if sender === motion {
                // The base stores only "off"; on is the default.
                let index = motion.indexOfSelectedItem
                appearance.motion = index == 2 ? false : (index == 1 && !isBase ? true : nil)
            } else if sender === customHue {
                appearance.hue = customHue.state == .on ? BotAppearance.normalizedHue(hue.doubleValue) : nil
            } else if sender === hue {
                appearance.hue = BotAppearance.normalizedHue(hue.doubleValue.rounded())
            }
        }
        refresh()
    }

    @objc private func colorChanged(_ sender: NSColorWell) {
        guard let entry = colorWells.first(where: { $0.well === sender }), let hex = Self.hex(sender.color) else { return }
        draft.update { $0.palette[keyPath: entry.key] = hex }
        refresh()
    }

    @objc private func clearColor(_ sender: NSButton) {
        let key = colorWells[sender.tag].key
        draft.update { $0.palette[keyPath: key] = nil }
        refresh()
    }

    @objc private func shuffle() {
        var generator = SystemRandomNumberGenerator()
        draft.shuffle(using: &generator)
        refresh()
    }

    @objc private func resetState() {
        draft.resetState()
        refresh()
    }

    @objc private func resetAll() {
        draft.resetAll()
        refresh()
    }

    @objc private func toggleCycle() {
        if cycleButton.state == .on, AvatarClock.isVisible(view), !AvatarClock.shared.reducesMotion {
            cycleIndex = BotAvatarState.allCases.firstIndex(of: draft.editing ?? .idle) ?? 0
            let timer = Timer(timeInterval: Self.cycleInterval, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    guard AvatarClock.isVisible(self.view), !AvatarClock.shared.reducesMotion else {
                        self.stopCycle()
                        self.refreshPreview()
                        return
                    }
                    self.cycleIndex = (self.cycleIndex + 1) % BotAvatarState.allCases.count
                    self.refreshPreview()
                }
            }
            RunLoop.main.add(timer, forMode: .common)
            cycleTimer = timer
        } else {
            stopCycle()
        }
        refreshPreview()
    }

    private func stopCycle() {
        cycleTimer?.invalidate()
        cycleTimer = nil
        cycleButton.state = .off
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

    /// Saves the look and the image in one request and waits for it; a failure keeps the sheet
    /// and the whole draft, image included, open with the reason.
    override func confirmTapped() {
        guard !isSaving else { return }
        guard bot != nil else {
            dismiss(nil)
            return
        }
        let look = draft.result
        if let look, let error = AvatarGeometryBridge.validationError(look) {
            status.stringValue = error
            fitSheetToContent()
            return
        }
        let photo: BotPhotoChange
        switch imageChange {
        case .keep: photo = .keep
        case .remove: photo = .remove
        case let .set(url, _): photo = .set(url)
        }
        let change = BotLookChange(result: look, hasChanges: draft.hasChanges)
        guard BotLookChange.updateParams(botID, look: change, photo: photo) != nil else {
            dismiss(nil)
            return
        }
        isSaving = true
        status.stringValue = ""
        refresh()
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await save(botID, change, photo)
                dismiss(nil)
            } catch {
                isSaving = false
                status.stringValue = L("The look was not saved: %@", error.localizedDescription)
                refresh()
            }
        }
    }

    // MARK: - Names

    static func title(of state: BotAvatarState) -> String {
        switch state {
        case .idle: L("Idle", context: "avatar state")
        case .thinking: L("Thinking", context: "avatar state")
        case .responding: L("Responding", context: "avatar state")
        case .working: L("Working", context: "avatar state")
        case .waiting: L("Waiting", context: "avatar state")
        case .retry: L("Retrying", context: "avatar state")
        case .error: L("Error", context: "avatar state")
        }
    }

    static func title(of shape: BotAppearance.Shape) -> String {
        switch shape {
        case .round: L("Round")
        case .organic: L("Organic")
        case .boxy: L("Boxy")
        case .capsule: L("Capsule")
        case .nub: L("Nub")
        case .cloud: L("Cloud")
        case .droplet: L("Droplet")
        case .hexagon: L("Hexagon")
        case .sun: L("Sun")
        case .triangle: L("Triangle")
        }
    }

    static func title(of expression: BotAppearance.Expression) -> String {
        switch expression {
        case .idle: L("Calm", context: "avatar expression")
        case .happy: L("Happy", context: "avatar expression")
        case .sad: L("Sad", context: "avatar expression")
        case .mad: L("Mad", context: "avatar expression")
        case .surprised: L("Surprised", context: "avatar expression")
        case .wink: L("Wink", context: "avatar expression")
        case .sleepy: L("Sleepy", context: "avatar expression")
        case .smug: L("Smug", context: "avatar expression")
        case .unsure: L("Unsure", context: "avatar expression")
        case .scared: L("Scared", context: "avatar expression")
        case .love: L("In love", context: "avatar expression")
        case .shy: L("Shy", context: "avatar expression")
        case .sick: L("Sick", context: "avatar expression")
        case .thinking: L("Thinking", context: "avatar expression")
        }
    }

    static func title(of background: BotAppearance.Background) -> String {
        switch background {
        case .none: L("None", context: "avatar background")
        case .square: L("Square")
        case .circle: L("Circle")
        case .squircle: L("Rounded square")
        }
    }

    static func title(of tone: BotAppearance.Tone) -> String {
        switch tone {
        case .pastel: L("Pastel")
        case .pale: L("Pale")
        case .mid: L("Medium", context: "avatar tone")
        case .deep: L("Deep")
        case .bright: L("Bright")
        case .ink: L("Ink")
        }
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

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self { min(max(self, range.lowerBound), range.upperBound) }
}
