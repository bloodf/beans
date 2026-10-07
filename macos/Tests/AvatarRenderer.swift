// Run with macos/Tests/run-avatar-renderer.sh; drives the JavaScriptCore geometry bridge, the
// shape-layer renderer, and the shared clock without launching the app.
import AppKit
import JavaScriptCore

@main
struct AvatarRendererChecks {
    @MainActor
    static func main() {
        _ = NSApplication.shared
        // Occlusion state and window visibility come only to a launched app.
        NSApp.setActivationPolicy(.accessory)
        NSApp.finishLaunching()
        var failures: [String] = []
        func check(_ condition: Bool, _ message: String, line: Int = #line) {
            if !condition { failures.append("line \(line): \(message)") }
        }
        AvatarGeometryBridge.scriptURL = URL(fileURLWithPath: CommandLine.arguments[1])

        // Endpoints: cached by bot, canonical look, and state; seed stays the bot ID.
        var look = BotLook()
        look.base = BotAppearance(shape: .cloud, background: .squircle)
        look.states[.error] = BotAppearance(shape: .triangle, expression: .sad, motion: false)
        let idle = AvatarGeometryBridge.endpoint(id: "lead", look: look, state: .idle)
        check(AvatarGeometryBridge.endpoint(id: "lead", look: look, state: .idle).geometry === idle.geometry, "endpoint cached")
        let error = AvatarGeometryBridge.endpoint(id: "lead", look: look, state: .error)
        check(error.key != idle.key && idle.motion && !error.motion, "state override and motion inheritance")
        check(AvatarGeometryBridge.endpoint(id: "other", look: look, state: .idle).key != idle.key, "seed is the bot ID")

        // Frames: the contract's counts, finite numbers, and amp 0 frozen in time.
        let still = AvatarGeometryBridge.frame(of: idle.geometry, time: 0, amp: 0)
        check(still.core.segments.count == 24 && still.extra.segments.count == 4, "core 24, extra 4")
        check(still.background.segments.count == 8 && still.backgroundOpacity == 1, "squircle background drawn")
        check(still.petals.count == 9 && still.eyes.count == 2 && still.eyeMatrices.count == 2, "petals 9, eyes 2")
        for t: CFTimeInterval in [0.3, 2.9, 61] {
            let later = AvatarGeometryBridge.frame(of: idle.geometry, time: t, amp: 0)
            check(later.body == still.body && later.eyeMatrices == still.eyeMatrices, "amp 0 is static at \(t)s")
        }
        let moving = (0..<40).map { AvatarGeometryBridge.frame(of: idle.geometry, time: Double($0) * 0.1, amp: 1).body }
        check(Set(moving.map { "\($0.ty),\($0.a)" }).count > 3, "amp 1 moves")
        let none = AvatarGeometryBridge.frame(of: AvatarGeometryBridge.endpoint(id: "lead", look: nil, state: .idle).geometry, time: 0, amp: 0)
        check(none.backgroundOpacity == 0, "seeded default has no backdrop")

        // Morph: the endpoint at t = 1 exactly, and a retarget from the displayed geometry.
        let end = AvatarGeometryBridge.frame(of: AvatarGeometryBridge.interpolate(from: idle.geometry, to: error.geometry, t: 1), time: 0, amp: 0)
        let target = AvatarGeometryBridge.frame(of: error.geometry, time: 0, amp: 0)
        check(end.core.segments == target.core.segments && end.head == target.head, "t 1 is the target")
        let progress = AvatarGeometryBridge.morphProgress(elapsed: 10, toward: error.geometry)
        check(progress.done && progress.t == 1, "morph finishes")

        // Paths in AppKit coordinates: inside the layer, eyes inside the body box, y flipped.
        let host = NSView(frame: NSRect(x: 0, y: 0, width: 64, height: 64))
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        host.wantsLayer = true
        let layer = AvatarRenderLayer()
        layer.host = host
        layer.frame = host.bounds
        host.layer?.addSublayer(layer)
        layer.show(id: "lead", look: look, state: .idle)
        layer.layoutIfNeeded()
        let shapes = layer.sublayers?.compactMap { $0 as? CAShapeLayer } ?? []
        check(shapes.count == 6, "six shape layers")
        let body = shapes[2].path?.boundingBoxOfPath ?? .null
        check(!body.isNull && layer.bounds.insetBy(dx: -2, dy: -2).contains(body), "body inside bounds: \(body)")
        for eye in shapes[4...5] {
            let box = eye.path?.boundingBoxOfPath ?? .null
            check(!box.isNull && body.contains(box), "eye inside body: \(box)")
            check(box.midY > body.midY, "eyes sit in the upper half after the y flip")
        }
        check(layer.contents == nil, "generated portrait draws no image")

        // Clock: offscreen and photo portraits never run it; Reduce Motion is honored by amp.
        check(!AvatarClock.shared.isRunning, "window not on screen: clock stopped")
        layer.show(image: NSImage(size: NSSize(width: 4, height: 4)))
        check(AvatarClock.shared.activeCount == 0 && shapes.allSatisfy(\.isHidden), "photo hides shapes, no clock")
        layer.show(id: "lead", look: look, state: .error)
        check(shapes.allSatisfy { !$0.isHidden } && layer.contents == nil, "back to generated")
        layer.clear()
        check(AvatarClock.shared.activeCount == 0 && !AvatarClock.shared.isRunning, "clear leaves no clock")

        // Live: a visible window runs the shared clock, moves frames, retargets mid-morph, and
        // stops for Reduce Motion, motion off, hiding, and closing.
        func pump(_ seconds: TimeInterval) {
            let deadline = Date().addingTimeInterval(seconds)
            while Date() < deadline {
                while let event = NSApp.nextEvent(matching: .any, until: .distantPast, inMode: .default, dequeue: true) {
                    NSApp.sendEvent(event)
                }
                RunLoop.main.run(until: min(deadline, Date().addingTimeInterval(0.02)))
            }
        }
        func bodyPath() -> CGPath? { (layer.sublayers?[2] as? CAShapeLayer)?.path }
        let live = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 96, height: 96), styleMask: [.titled], backing: .buffered, defer: false)
        let liveHost = NSView(frame: NSRect(x: 0, y: 0, width: 96, height: 96))
        liveHost.wantsLayer = true
        live.contentView = liveHost
        layer.host = liveHost
        layer.frame = liveHost.bounds
        liveHost.layer?.addSublayer(layer)
        live.orderFrontRegardless()
        pump(1)
        let onScreen = live.occlusionState.contains(.visible)
        if onScreen {
            AvatarClock.shared.setReducesMotion(false)
            layer.show(id: "lead", look: look, state: .idle)
            pump(0.1)
            check(AvatarClock.shared.isRunning && AvatarClock.shared.activeCount == 1, "visible portrait runs the clock")
            let first = bodyPath()
            pump(0.4)
            check(bodyPath() != first, "ambient frames change")

            // Two retargets before any paint must both start from the last painted geometry.
            var middleLook = look
            middleLook.states[.working] = BotAppearance(shape: .boxy)
            let painted = bodyPath()
            layer.show(id: "lead", look: middleLook, state: .working)
            layer.show(id: "lead", look: middleLook, state: .thinking)
            _ = layer.tick(CACurrentMediaTime())
            check(painted != nil && bodyPath() != nil && abs(painted!.boundingBoxOfPath.width - bodyPath()!.boundingBoxOfPath.width) < 1, "unpainted endpoint is not a retarget origin")
            pump(0.6)

            AvatarClock.shared.setReducesMotion(true)
            pump(0.1)
            check(!AvatarClock.shared.isRunning, "Reduce Motion stops the clock")
            let still = bodyPath()
            pump(0.3)
            check(bodyPath() == still, "Reduce Motion is static")
            layer.show(id: "lead", look: look, state: .idle)
            check(!AvatarClock.shared.isRunning, "Reduce Motion: state changes cut, no morph")
            AvatarClock.shared.setReducesMotion(false)
            pump(0.1)
            check(AvatarClock.shared.isRunning, "motion resumes")

            layer.show(id: "lead", look: look, state: .error)
            check(!AvatarClock.shared.isRunning, "motion-off target snaps without starting a morph")
            pump(0.6)
            check(!AvatarClock.shared.isRunning, "motion off in the look: static")

            layer.show(id: "lead", look: look, state: .idle)
            pump(0.1)
            liveHost.isHidden = true
            layer.wake()
            pump(0.1)
            check(!AvatarClock.shared.isRunning, "hidden view stops the clock")
            liveHost.isHidden = false
            layer.wake()
            pump(0.1)
            check(AvatarClock.shared.isRunning, "shown again resumes")
            live.orderOut(nil)
            pump(0.2)
            check(!AvatarClock.shared.isRunning, "window off screen stops the clock")
            layer.clear()
        } else {
            print("avatar renderer: live checks skipped; visible=\(live.isVisible), occlusion=\(live.occlusionState.rawValue), screen=\(live.screen != nil)")
        }
        live.close()

        if failures.isEmpty {
            print("avatar renderer: ok")
        } else {
            failures.forEach { print("FAIL \($0)") }
            exit(1)
        }
    }
}
