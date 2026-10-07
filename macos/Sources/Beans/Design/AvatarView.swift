import AppKit

import JavaScriptCore

enum Glyph {
    static func symbol(_ name: String, pointSize: CGFloat, weight: NSFont.Weight = .semibold, color: NSColor)
        -> NSImage?
    {
        let configuration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        return NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration)
    }
}

/// A bot's generated portrait, drawn by `AvatarRenderLayer` from shared numeric geometry in its
/// own shape and background, or the bot's own image clipped to a circle. Also renders "you" and
/// devices.
final class AvatarView: NSView {
    enum Content: Hashable {
        /// A generated portrait: the bot's saved look (nil is the seeded default) in `state`.
        case bot(id: String, look: BotLook? = nil, state: BotAvatarState = .idle)
        /// A custom profile image, drawn aspect-filled inside the circle.
        case image(NSImage)
        case you
        case system
    }

    var content: Content = .system {
        didSet { if content != oldValue { showContent() } }
    }

    private let portrait = AvatarRenderLayer()

    private func showContent() {
        if case let .bot(id, look, state) = content {
            portrait.isHidden = false
            portrait.show(id: id, look: look, state: state)
        } else {
            portrait.clear()
            portrait.isHidden = true
        }
        needsDisplay = true
    }

    /// Set, a click on the avatar calls this and the pointer becomes a hand over it.
    var onClick: (() -> Void)? {
        didSet {
            toolTip = onClick == nil ? nil : L("Change look")
            window?.invalidateCursorRects(for: self)
        }
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        if onClick != nil { addCursorRect(bounds, cursor: .pointingHand) }
    }

    override func mouseDown(with event: NSEvent) {
        guard onClick != nil else { return super.mouseDown(with: event) }
    }

    override func mouseUp(with event: NSEvent) {
        guard let onClick else { return super.mouseUp(with: event) }
        if bounds.contains(convert(event.locationInWindow, from: nil)) { onClick() }
    }

    /// Green dot at the bottom right while the bot has a turn running.
    var isWorking = false {
        didSet {
            if isWorking != oldValue {
                needsDisplay = true
                syncPresence()
            }
        }
    }

    private let presence = PresenceLayer()

    private var box: NSRect {
        NSRect(x: 0, y: 0, width: diameter, height: diameter)
            .offsetBy(dx: (bounds.width - diameter) / 2, dy: (bounds.height - diameter) / 2)
    }

    private func syncPresence() {
        let box = box
        presence.sync(isWorking: isWorking, in: box, on: layer, flipped: isFlipped, view: self)
        portrait.cutouts = isWorking
            ? [AvatarView.presenceRect(in: box).insetBy(dx: -2, dy: -2).offsetBy(dx: -box.minX, dy: -box.minY)] : []
    }

    override func layout() {
        super.layout()
        portrait.frame = box
        syncPresence()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        portrait.setScale(window?.backingScaleFactor ?? 2)
        portrait.wake()
        presence.refreshMotion()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        portrait.setScale(window?.backingScaleFactor ?? 2)
    }

    override func viewDidHide() {
        super.viewDidHide()
        portrait.wake()
        presence.refreshMotion()
    }

    override func viewDidUnhide() {
        super.viewDidUnhide()
        portrait.wake()
        presence.refreshMotion()
    }

    var diameter: CGFloat {
        didSet {
            if diameter != oldValue {
                invalidateIntrinsicContentSize()
                needsDisplay = true
            }
        }
    }

    init(diameter: CGFloat = Theme.Metric.avatarSize) {
        self.diameter = diameter
        super.init(frame: NSRect(x: 0, y: 0, width: diameter, height: diameter))
        wantsLayer = true
        translatesAutoresizingMaskIntoConstraints = false
        portrait.host = self
        portrait.isHidden = true
        layer?.addSublayer(portrait)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize {
        NSSize(width: diameter, height: diameter)
    }

    override var allowsVibrancy: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        let box = box
        AvatarView.render(content, in: box)
        if isWorking { AvatarView.clearPresenceRing(in: box) }
    }

    /// Where the working dot sits: over the bottom-right edge of the avatar.
    static func presenceRect(in box: NSRect) -> NSRect {
        let size = max(7, (box.width * 0.28).rounded())
        return NSRect(x: box.maxX - size + 1, y: box.minY - 1, width: size, height: size)
    }

    /// Clears a ring under the dot so it reads as sitting on top whatever the background is.
    static func clearPresenceRing(in box: NSRect) {
        guard let context = NSGraphicsContext.current else { return }
        context.compositingOperation = .clear
        NSColor.black.setFill()
        NSBezierPath(ovalIn: presenceRect(in: box).insetBy(dx: -2, dy: -2)).fill()
        context.compositingOperation = .sourceOver
    }

    /// Draws one avatar into the current context. Shared with `AvatarClusterView`,
    /// which packs several of these into a single view. A generated portrait is the view's
    /// `AvatarRenderLayer`, not a drawing.
    static func render(_ content: Content, in box: NSRect) {
        let path = NSBezierPath(ovalIn: box)

        switch content {
        case .bot:
            break
        case let .image(image):
            NSGraphicsContext.saveGraphicsState()
            path.addClip()
            NSColor.quaternaryLabelColor.setFill()
            path.fill()
            let size = image.size
            guard size.width > 0, size.height > 0 else {
                NSGraphicsContext.restoreGraphicsState()
                return
            }
            // Aspect-fill: scale so the shorter side spans the circle, centered.
            let scale = max(box.width / size.width, box.height / size.height)
            let drawn = NSSize(width: size.width * scale, height: size.height * scale)
            let origin = NSPoint(x: box.midX - drawn.width / 2, y: box.midY - drawn.height / 2)
            image.draw(in: NSRect(origin: origin, size: drawn), from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: [.interpolation: NSImageInterpolation.high])
            NSGraphicsContext.restoreGraphicsState()

        case .you:
            NSColor.tertiaryLabelColor.setFill()
            path.fill()
            renderSymbol("person.fill", in: box, color: .white, scale: 0.5)

        case .system:
            NSColor.quaternaryLabelColor.setFill()
            path.fill()
            renderSymbol("gearshape.fill", in: box, color: .white, scale: 0.48)
        }
    }

    private static func renderSymbol(_ name: String, in box: NSRect, color: NSColor, scale: CGFloat) {
        guard
            let image = Glyph.symbol(name, pointSize: box.width * scale, weight: .semibold, color: color)
        else { return }
        let size = image.size
        let origin = NSPoint(
            x: box.midX - size.width / 2,
            y: box.midY - size.height / 2
        )
        image.draw(in: NSRect(origin: origin, size: size))
    }

    static func content(for author: Message.Author, store: AppStore, in chatID: Chat.ID? = nil) -> Content {
        switch author {
        case .you: .you
        case .system: .system
        case let .bot(id):
            store.bot(id).map { AvatarView.content(for: $0, store: store, in: chatID) } ?? .system
        }
    }

    /// The bot's image when it has one and this computer has the bytes (the store fetches them and
    /// redraws otherwise), else its generated portrait with its saved look. `chatID` picks what the
    /// bot is doing there; without one it is the strongest state across its chats.
    @MainActor
    static func content(for bot: Bot, store: AppStore? = nil, in chatID: Chat.ID? = nil) -> Content {
        let store = store ?? AppStore.shared
        if let image = store.avatarImage(for: bot) { return .image(image) }
        let state = chatID.map { store.avatarState(of: bot.id, in: $0) } ?? store.avatarState(of: bot.id)
        return .bot(id: bot.id, look: bot.look, state: state)
    }
}

/// Group avatars packed into a fixed square. A row of overlapping circles grows
/// with the participant count and drags the title along with it; a constant slot
/// keeps every sidebar row's text on the same baseline column.
final class AvatarClusterView: NSView {
    private var contents: [AvatarView.Content] = []
    private let slot: CGFloat
    private let ring: CGFloat = 1

    var isWorking = false {
        didSet {
            if isWorking != oldValue {
                needsDisplay = true
                syncPresence()
            }
        }
    }

    private let presence = PresenceLayer()
    /// One portrait layer per slot, back to front; generated slots animate, others stay hidden.
    private let portraits = (0..<4).map { _ in AvatarRenderLayer() }

    private func syncPresence() {
        guard let box = boxes().last else { return }
        presence.sync(isWorking: isWorking, in: box, on: layer, flipped: isFlipped, view: self)
        syncPortraits()
    }

    /// Places each slot's layer and cuts the rings drawn views clear: under the front avatars and
    /// the working dot.
    private func syncPortraits() {
        let boxes = boxes()
        for (index, portrait) in portraits.enumerated() {
            guard index < contents.count, case let .bot(id, look, state) = contents[index] else {
                portrait.clear()
                portrait.isHidden = true
                continue
            }
            let box = boxes[index]
            portrait.frame = box
            portrait.isHidden = false
            portrait.show(id: id, look: look, state: state)
            var cutouts = boxes.dropFirst(index + 1).prefix(contents.count - index - 1)
                .map { $0.insetBy(dx: -ring, dy: -ring).offsetBy(dx: -box.minX, dy: -box.minY) }
            if isWorking, index == contents.count - 1 {
                cutouts.append(AvatarView.presenceRect(in: box).insetBy(dx: -2, dy: -2).offsetBy(dx: -box.minX, dy: -box.minY))
            }
            portrait.cutouts = cutouts
        }
    }

    override func layout() {
        super.layout()
        syncPresence()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        for portrait in portraits {
            portrait.setScale(window?.backingScaleFactor ?? 2)
            portrait.wake()
        }
        presence.refreshMotion()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        for portrait in portraits { portrait.setScale(window?.backingScaleFactor ?? 2) }
    }

    override func viewDidHide() {
        super.viewDidHide()
        for portrait in portraits { portrait.wake() }
        presence.refreshMotion()
    }

    override func viewDidUnhide() {
        super.viewDidUnhide()
        for portrait in portraits { portrait.wake() }
        presence.refreshMotion()
    }

    init(slot: CGFloat) {
        self.slot = slot
        super.init(frame: NSRect(x: 0, y: 0, width: slot, height: slot))
        wantsLayer = true
        translatesAutoresizingMaskIntoConstraints = false
        for portrait in portraits {
            portrait.host = self
            portrait.isHidden = true
            layer?.addSublayer(portrait)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { NSSize(width: slot, height: slot) }

    override var allowsVibrancy: Bool { false }

    func configure(with bots: [Bot]) {
        configure(with: bots.prefix(4).map { AvatarView.content(for: $0) })
    }

    func configure(with contents: [AvatarView.Content]) {
        let next = Array(contents.prefix(4))
        guard next != self.contents else { return }
        self.contents = next
        needsDisplay = true
        syncPresence()
    }

    /// Cells of a 2×2 grid that overlap slightly, ordered back to front so the
    /// right column sits over the left. Fewer than four bots drop cells but keep
    /// the grid's geometry, so a chat's avatar never shifts the title column.
    private func boxes() -> [NSRect] {
        let origin = NSPoint(x: (bounds.width - slot) / 2, y: (bounds.height - slot) / 2)
        func box(_ x: CGFloat, _ y: CGFloat, _ size: CGFloat) -> NSRect {
            NSRect(x: origin.x + x, y: origin.y + y, width: size, height: size)
        }

        guard contents.count > 1 else {
            let size = slot - 2
            return [box(1, 1, size)]
        }

        let size = (slot * 0.6).rounded()
        let free = slot - size
        let left: CGFloat = 0
        let right = free
        let top = free
        let bottom: CGFloat = 0

        switch contents.count {
        case 2:
            return [box(left, top, size), box(right, bottom, size)]
        case 3:
            return [box(left, top, size), box(right, top, size), box(free / 2, bottom, size)]
        default:
            return [
                box(left, top, size), box(right, top, size),
                box(left, bottom, size), box(right, bottom, size),
            ]
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        let boxes = boxes()
        for (index, box) in zip(contents.indices, boxes) {
            // Clear a ring under each front circle so the one behind reads as separate
            // whatever the row is sitting on — sidebar material or selection fill.
            if index > 0, let context = NSGraphicsContext.current {
                context.compositingOperation = .clear
                NSColor.black.setFill()
                NSBezierPath(ovalIn: box.insetBy(dx: -ring, dy: -ring)).fill()
                context.compositingOperation = .sourceOver
            }
            AvatarView.render(contents[index], in: box)
        }
        if isWorking, let last = boxes.last { AvatarView.clearPresenceRing(in: last) }
    }
}

/// The green working dot: a layer so it can breathe (scale 1 → 0.8 over 2.4 s) while the
/// avatar underneath stays a plain drawing.
@MainActor
final class PresenceLayer: CALayer {
    private weak var hostView: NSView?
    override init() {
        super.init()
        backgroundColor = NSColor.systemGreen.cgColor
        isHidden = true
        NotificationCenter.default.addObserver(self, selector: #selector(refreshMotion), name: AvatarClock.environmentDidChange, object: nil)
    }

    override init(layer: Any) {
        super.init(layer: layer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func sync(isWorking: Bool, in box: NSRect, on host: CALayer?, flipped: Bool, view: NSView) {
        hostView = view
        guard let host else { return }
        if superlayer !== host { host.addSublayer(self) }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        var rect = AvatarView.presenceRect(in: box)
        if flipped { rect.origin.y = box.maxY - rect.maxY + box.minY }
        bounds = CGRect(origin: .zero, size: rect.size)
        position = CGPoint(x: rect.midX, y: rect.midY)
        cornerRadius = rect.width / 2
        backgroundColor = NSColor.systemGreen.cgColor
        isHidden = !isWorking
        CATransaction.commit()
        refreshMotion()
    }

    @objc func refreshMotion() {
        let visible = hostView.map(AvatarClock.isVisible) ?? false
        if !isHidden && visible && !AvatarClock.shared.reducesMotion {
            if animation(forKey: "breathe") == nil {
                let pulse = CABasicAnimation(keyPath: "transform.scale")
                pulse.fromValue = 1
                pulse.toValue = 0.8
                pulse.duration = 1.2
                pulse.autoreverses = true
                pulse.repeatCount = .infinity
                pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                add(pulse, forKey: "breathe")
            }
        } else {
            removeAnimation(forKey: "breathe")
        }
    }
}

/// Overlapping mini avatars used where there is room to spread out horizontally.
final class AvatarStackView: NSView {
    private var avatars: [AvatarView] = []
    private let diameter: CGFloat
    private let overlap: CGFloat

    init(diameter: CGFloat = 30, overlap: CGFloat = 9) {
        self.diameter = diameter
        self.overlap = overlap
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(with bots: [Bot]) {
        let visible = Array(bots.prefix(3))
        while avatars.count < visible.count {
            let avatar = AvatarView(diameter: diameter)
            avatar.translatesAutoresizingMaskIntoConstraints = true
            avatar.wantsLayer = true
            avatar.layer?.cornerRadius = diameter / 2
            avatars.append(avatar)
            addSubview(avatar)
        }
        while avatars.count > visible.count {
            avatars.removeLast().removeFromSuperview()
        }

        // Later avatars sit on top and to the right; a ring keeps them separated.
        for (index, bot) in visible.enumerated() {
            let avatar = avatars[index]
            avatar.content = AvatarView.content(for: bot)
            avatar.layer?.borderWidth = visible.count > 1 ? 1.5 : 0
            avatar.layer?.borderColor = NSColor.windowBackgroundColor.cgColor
        }

        invalidateIntrinsicContentSize()
        needsLayout = true
    }

    override var intrinsicContentSize: NSSize {
        let count = CGFloat(max(avatars.count, 1))
        return NSSize(width: diameter + (count - 1) * (diameter - overlap), height: diameter)
    }

    override func layout() {
        super.layout()
        let step = diameter - overlap
        for (index, avatar) in avatars.enumerated() {
            avatar.frame = NSRect(
                x: CGFloat(index) * step,
                y: (bounds.height - diameter) / 2,
                width: diameter,
                height: diameter
            )
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        for avatar in avatars {
            avatar.layer?.borderColor = NSColor.windowBackgroundColor.cgColor
        }
    }
}
