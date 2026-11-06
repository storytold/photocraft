import QuartzCore
import SwiftUI

/// PhotoCraft for iPad: SwiftUI owns the app and its scene; the canvas view hosts the Rust app.
/// Native panels (toolbars, layers, document pickers) grow around `CanvasView` from here.
@main
struct PhotoCraftApp: App {
    var body: some Scene {
        WindowGroup {
            CanvasView()
                .ignoresSafeArea(edges: .bottom)
                .background(Color.black)
                .statusBarHidden()
        }
    }
}

/// The Rust-drawn canvas as a SwiftUI view.
struct CanvasView: UIViewRepresentable {
    func makeUIView(context: Context) -> CanvasUIView { CanvasUIView(frame: .zero) }
    func updateUIView(_ view: CanvasUIView, context: Context) {}
}

/// A Metal-backed view that Rust draws into. It forwards its size, the display's refresh and
/// every touch (with coalesced Pencil samples and their force); Rust decides when to draw.
final class CanvasUIView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    private var link: CADisplayLink?
    private var started = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = .black
    }

    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil {
            let link = CADisplayLink(target: self, selector: #selector(step))
            link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
            link.add(to: .main, forMode: .common)
            self.link = link
        } else {
            // The link retains this view: break the cycle, and drop the surface before the layer.
            link?.invalidate()
            link = nil
            if started {
                pc_stop()
                started = false
            }
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, let metal = layer as? CAMetalLayer else { return }
        let scale = traitCollection.displayScale
        contentScaleFactor = scale
        let width = UInt32(max(0, (bounds.width * scale).rounded()))
        let height = UInt32(max(0, (bounds.height * scale).rounded()))
        guard width > 0, height > 0 else { return }
        metal.drawableSize = CGSize(width: Int(width), height: Int(height))
        if started {
            pc_resize(width, height, Float(scale))
        } else {
            started = pc_start(Unmanaged.passUnretained(metal).toOpaque(), width, height, Float(scale))
            if started { replayDemoTaps() }
        }
    }

    @objc private func step() {
        _ = pc_frame()
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 0) }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 1) }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 2) }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 3) }

    /// Development aid: `PHOTOCRAFT_DEMO_TAPS="x,y;x,y"` (points) taps there, a second apart,
    /// through the same path as real touches. The simulator has no way to tap from a script.
    private func replayDemoTaps() {
        guard let spec = ProcessInfo.processInfo.environment["PHOTOCRAFT_DEMO_TAPS"] else { return }
        let taps = spec.split(separator: ";").compactMap { pair -> (Float, Float)? in
            let xy = pair.split(separator: ",").compactMap { Float($0) }
            return xy.count == 2 ? (xy[0], xy[1]) : nil
        }
        for (index, (x, y)) in taps.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + .seconds(2 + index)) {
                let id = UInt64(1000 + index)
                pc_touch(id, 0, x, y, 0)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(100)) { pc_touch(id, 2, x, y, 0) }
            }
        }
    }

    private func send(_ touches: Set<UITouch>, _ event: UIEvent?, phase: UInt32) {
        for touch in touches {
            let id = UInt64(UInt(bitPattern: Unmanaged.passUnretained(touch).toOpaque()))
            // A Pencil reports up to 240 samples a second; the display shows 120. Keep them all.
            let samples = phase == 1 ? (event?.coalescedTouches(for: touch) ?? [touch]) : [touch]
            for sample in samples {
                let point = sample.preciseLocation(in: self)
                let force = sample.maximumPossibleForce > 0 ? sample.force / sample.maximumPossibleForce : 0
                pc_touch(id, phase, Float(point.x), Float(point.y), Float(force))
            }
        }
    }
}
