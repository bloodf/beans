import AppKit
import JavaScriptCore

/// A generated portrait as Core Animation shape layers: background, droplet taper, body, petals,
/// and two eyes, rebuilt from numeric frames. Morphs between states start from what is on
/// screen, so an interrupted change continues from where it was.
@MainActor
final class AvatarRenderLayer: CALayer {
    private let backgroundShape = CAShapeLayer()
    private let extraShape = CAShapeLayer()
    private let coreShape = CAShapeLayer()
    private let petalShape = CAShapeLayer()
    private let eyeShapes = [CAShapeLayer(), CAShapeLayer()]
    private let cutoutMask = CAShapeLayer()

    /// The view whose visibility decides whether this layer animates.
    weak var host: NSView?

    private var target: AvatarGeometryBridge.Endpoint?
    /// What the layer showed last: the target once settled, an interpolation mid-morph.
    private var displayed: JSValueBox?
    private var morph: (from: JSValueBox, start: CFTimeInterval)?

    /// Ovals, in this layer's coordinates, left transparent: the ring under the working dot and
    /// under a cluster's front avatars.
    var cutouts: [CGRect] = [] {
        didSet { if cutouts != oldValue { updateMask() } }
    }

    override init() {
        super.init()
        for shape in [backgroundShape, extraShape, coreShape, petalShape] + eyeShapes {
            shape.actions = Self.noActions
            addSublayer(shape)
        }
        actions = Self.noActions
        cutoutMask.fillRule = .evenOdd
        cutoutMask.actions = Self.noActions
        AvatarClock.shared.register(self)
    }

    override init(layer: Any) {
        super.init(layer: layer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private static let noActions: [String: CAAction] = [
        "path": NSNull(), "fillColor": NSNull(), "opacity": NSNull(), "bounds": NSNull(), "position": NSNull(),
        "hidden": NSNull(), "contents": NSNull(), "mask": NSNull(),
    ]

    /// Shows `id` with `look` in `state`, morphing from what is on screen when motion allows.
    func show(id: String, look: BotLook?, state: BotAvatarState) {
        let next = AvatarGeometryBridge.endpoint(id: id, look: look, state: state)
        guard next.key != target?.key else { return }
        contents = nil
        for shape in [backgroundShape, extraShape, coreShape, petalShape] + eyeShapes { shape.isHidden = false }
        if target != nil, next.motion, isVisible, !AvatarClock.shared.reducesMotion, let displayed {
            morph = (displayed, CACurrentMediaTime())
        } else {
            morph = nil
        }
        target = next
        if morph == nil { render(at: CACurrentMediaTime()) }
        wake()
    }

    /// A photo, "you", or a device instead of a generated portrait: a still image already drawn
    /// at the layer's size, no geometry, no clock.
    func show(image: NSImage) {
        clear()
        for shape in [backgroundShape, extraShape, coreShape, petalShape] + eyeShapes { shape.isHidden = true }
        contents = image
        contentsGravity = .resize
    }

    /// Back to nothing: a recycled cell keeps no geometry or clock.
    func clear() {
        target = nil
        displayed = nil
        morph = nil
        AvatarClock.shared.remove(self)
    }

    /// Shape layers added by hand do not follow the window's backing scale; the host passes it.
    func setScale(_ scale: CGFloat) {
        guard scale > 0, scale != contentsScale else { return }
        for item in [self, cutoutMask, backgroundShape, extraShape, coreShape, petalShape] + eyeShapes {
            item.contentsScale = scale
        }
    }

    override func layoutSublayers() {
        super.layoutSublayers()
        for shape in [backgroundShape, extraShape, coreShape, petalShape] + eyeShapes {
            shape.frame = bounds
        }
        updateMask()
        render(at: CACurrentMediaTime())
    }

    private var isVisible: Bool {
        host.map(AvatarClock.isVisible) ?? false
    }

    /// Whether ambient motion runs: the look allows it, the system does not reduce motion, and
    /// the portrait is on screen.
    private var amp: Double {
        guard let target, target.motion, !AvatarClock.shared.reducesMotion, isVisible else { return 0 }
        return 1
    }

    /// Joins the shared clock when there is something to animate, else draws the static frame.
    func wake() {
        if target != nil, !AvatarClock.shared.reducesMotion, morph != nil || amp > 0, isVisible {
            AvatarClock.shared.add(self)
        } else {
            morph = nil
            AvatarClock.shared.remove(self)
            render(at: CACurrentMediaTime())
        }
    }

    /// One clock tick; false once nothing moves, which drops the layer from the clock.
    func tick(_ time: CFTimeInterval) -> Bool {
        guard target != nil, isVisible else {
            morph = nil
            render(at: time)
            return false
        }
        render(at: time)
        return morph != nil || amp > 0
    }

    private func render(at time: CFTimeInterval) {
        guard let target, bounds.width > 0 else { return }
        let amp = amp
        let frame: AvatarFrameData
        let painted: JSValue
        if let morph {
            let progress = AvatarGeometryBridge.morphProgress(elapsed: time - morph.start, toward: target.geometry)
            if progress.done || amp == 0 {
                self.morph = nil
                painted = target.geometry
                frame = AvatarGeometryBridge.frame(of: target, time: time, amp: amp)
            } else {
                let current = AvatarGeometryBridge.interpolate(from: morph.from.value, to: target.geometry, t: progress.t)
                painted = current
                frame = AvatarGeometryBridge.frame(of: current, time: time, amp: amp)
            }
        } else {
            painted = target.geometry
            frame = AvatarGeometryBridge.frame(of: target, time: time, amp: amp)
        }
        apply(frame)
        displayed = JSValueBox(painted)
    }

    /// The 0–100 y-down viewBox onto this layer's y-up bounds.
    private var viewBox: CGAffineTransform {
        let scale = min(bounds.width, bounds.height) / 100
        return CGAffineTransform(
            a: scale, b: 0, c: 0, d: -scale,
            tx: (bounds.width - scale * 100) / 2, ty: bounds.height - (bounds.height - scale * 100) / 2)
    }

    private func apply(_ frame: AvatarFrameData) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let viewBox = viewBox
        let body = frame.body.concatenating(viewBox)
        backgroundShape.path = frame.background.cgPath(transform: viewBox)
        backgroundShape.fillColor = AvatarColors.color(frame.backgroundFill)
        backgroundShape.opacity = Float(frame.backgroundOpacity)
        let head = AvatarColors.color(frame.head)
        extraShape.path = frame.extra.cgPath(transform: body)
        coreShape.path = frame.core.cgPath(transform: body)
        let petals = CGMutablePath()
        petals.addPath(frame.petalPath(), transform: viewBox)
        petalShape.path = petals
        for shape in [extraShape, coreShape, petalShape] { shape.fillColor = head }
        let eye = AvatarColors.color(frame.eye)
        for (index, shape) in eyeShapes.enumerated() {
            shape.path = frame.eyes[index].cgPath(transform: frame.eyeMatrices[index].concatenating(viewBox))
            shape.fillColor = eye
        }
        CATransaction.commit()
    }

    private func updateMask() {
        guard !cutouts.isEmpty else {
            mask = nil
            return
        }
        let path = CGMutablePath()
        path.addRect(bounds)
        for oval in cutouts { path.addEllipse(in: oval) }
        cutoutMask.frame = bounds
        cutoutMask.path = path
        mask = cutoutMask
    }
}

/// A `JSValue` kept by a Swift value without exposing JavaScriptCore to callers.
struct JSValueBox {
    let value: JSValue
    init(_ value: JSValue) { self.value = value }
}

/// The one clock every animating portrait shares. It runs only while some visible portrait
/// moves, and stops when the last one settles, leaves the screen, or motion is reduced.
@MainActor
final class AvatarClock {
    static let shared = AvatarClock()

    /// Every live portrait layer, so Reduce Motion and window visibility changes reach them all.
    private let all = NSHashTable<AvatarRenderLayer>.weakObjects()
    private let active = NSHashTable<AvatarRenderLayer>.weakObjects()
    private var timer: Timer?
    private(set) var reducesMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion

    /// Applies the system setting (or a test's) to every portrait at once.
    func setReducesMotion(_ value: Bool) {
        reducesMotion = value
        wakeAll()
        NotificationCenter.default.post(name: Self.environmentDidChange, object: nil)
    }
    // ponytail: a 30 Hz main-run-loop timer, not a display link; switch to NSView.displayLink if
    // motion needs to track ProMotion refresh.
    static let interval: TimeInterval = 1.0 / 30

    static let environmentDidChange = Notification.Name("AvatarMotionEnvironmentDidChange")

    static func isVisible(_ view: NSView) -> Bool {
        guard let window = view.window else { return false }
        return window.isVisible && !window.isMiniaturized && window.occlusionState.contains(.visible)
            && !view.isHiddenOrHasHiddenAncestor && !view.visibleRect.isEmpty
    }

    private init() {
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated {
                AvatarClock.shared.setReducesMotion(NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
            }
        }
        for name in [NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated {
                    AvatarClock.shared.wakeAll()
                    NotificationCenter.default.post(name: Self.environmentDidChange, object: nil)
                }
            }
        }
    }

    /// How many portraits the clock is driving; zero means the timer is stopped.
    var activeCount: Int { active.allObjects.count }
    var isRunning: Bool { timer != nil }

    func register(_ layer: AvatarRenderLayer) { all.add(layer) }

    func add(_ layer: AvatarRenderLayer) {
        active.add(layer)
        guard timer == nil else { return }
        let timer = Timer(timeInterval: Self.interval, repeats: true) { _ in
            MainActor.assumeIsolated { AvatarClock.shared.tick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func remove(_ layer: AvatarRenderLayer) {
        active.remove(layer)
        if active.allObjects.isEmpty { stop() }
    }

    func wakeAll() {
        for layer in all.allObjects { layer.wake() }
    }

    private func tick() {
        let time = CACurrentMediaTime()
        for layer in active.allObjects where !layer.tick(time) {
            active.remove(layer)
        }
        if active.allObjects.isEmpty { stop() }
    }

    private func stop() {
        timer?.invalidate()
        timer = nil
    }
}
