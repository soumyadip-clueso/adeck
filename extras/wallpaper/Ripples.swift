// Ripples wallpaper: the ripple tank (assets/ripples.js) on the desktop, under the icons, on
// every screen. The cursor stirs it, a click drops a stone. It pauses while a full-screen app
// covers it, and a menu bar item pauses or quits it.

import AppKit
import WebKit

final class Wallpaper: NSObject, NSApplicationDelegate {
    private var screens: [(window: NSWindow, view: WKWebView)] = []
    private var status: NSStatusItem!
    private var paused = false
    private var lastMove = Date.distantPast

    private var page: URL {
        Bundle.main.url(forResource: "index", withExtension: "html", subdirectory: "ripples")!
    }

    func applicationDidFinishLaunching(_ note: Notification) {
        build()
        NotificationCenter.default.addObserver(self, selector: #selector(rebuild), name: NSApplication.didChangeScreenParametersNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(rebuild), name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(sleeping), name: NSWorkspace.screensDidSleepNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(waking), name: NSWorkspace.screensDidWakeNotification, object: nil)

        // The cursor, from anywhere: mouse events need no accessibility permission.
        NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved, .leftMouseDown, .leftMouseDragged]) { [weak self] e in
            self?.cursor(click: e.type == .leftMouseDown)
        }

        // RIPPLES_SNAPSHOT=/path.png: photograph the first screen's wallpaper after a few seconds.
        if let out = ProcessInfo.processInfo.environment["RIPPLES_SNAPSHOT"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 4) { [weak self] in
                self?.screens.first?.view.takeSnapshot(with: nil) { img, _ in
                    if let img, let tiff = img.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff) {
                        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: out))
                    }
                }
            }
        }

        status = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        status.button?.image = NSImage(systemSymbolName: "water.waves", accessibilityDescription: "Ripples")
        menu()
    }

    private func menu() {
        let m = NSMenu()
        m.addItem(withTitle: paused ? "Resume" : "Pause", action: #selector(togglePause), keyEquivalent: "p").target = self
        m.addItem(withTitle: "Drop a stone", action: #selector(stone), keyEquivalent: "").target = self
        m.addItem(.separator())
        m.addItem(withTitle: "Quit Ripples", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        status.menu = m
    }

    private func build() {
        for screen in NSScreen.screens {
            let w = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
            w.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
            w.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenNone]
            w.ignoresMouseEvents = true
            w.isOpaque = true
            w.hasShadow = false
            w.backgroundColor = .black
            w.isReleasedWhenClosed = false
            let cfg = WKWebViewConfiguration()
            cfg.suppressesIncrementalRendering = true
            let v = WKWebView(frame: NSRect(origin: .zero, size: screen.frame.size), configuration: cfg)
            v.setValue(false, forKey: "drawsBackground")
            v.loadFileURL(page, allowingReadAccessTo: page.deletingLastPathComponent())
            w.contentView = v
            w.setFrame(screen.frame, display: true)
            w.orderFront(nil)
            NotificationCenter.default.addObserver(self, selector: #selector(occlusion(_:)), name: NSWindow.didChangeOcclusionStateNotification, object: w)
            screens.append((w, v))
        }
    }

    @objc private func rebuild() {
        let frames = NSScreen.screens.map(\.frame)
        if frames == screens.map({ $0.window.frame }) { return }
        screens.forEach { NotificationCenter.default.removeObserver(self, name: NSWindow.didChangeOcclusionStateNotification, object: $0.window); $0.window.close() }
        screens = []
        build()
    }

    private func js(_ s: String, _ view: WKWebView? = nil) {
        for v in view.map({ [$0] }) ?? screens.map(\.view) {
            v.evaluateJavaScript("window.ripples && ripples.\(s)", completionHandler: nil)
        }
    }

    private func cursor(click: Bool) {
        if paused { return }
        if !click && Date().timeIntervalSince(lastMove) < 0.03 { return }
        lastMove = Date()
        let p = NSEvent.mouseLocation
        guard let s = screens.first(where: { $0.window.frame.contains(p) }) else { return }
        let f = s.window.frame
        js(String(format: "poke(%.4f, %.4f, %@)", (p.x - f.minX) / f.width, 1 - (p.y - f.minY) / f.height, click ? "true" : "false"), s.view)
    }

    // Hidden behind a full-screen app, or the screens asleep: stop drawing.
    @objc private func occlusion(_ n: Notification) {
        guard let w = n.object as? NSWindow, let s = screens.first(where: { $0.window === w }) else { return }
        js("pause(\(paused || !w.occlusionState.contains(.visible)))", s.view)
    }
    @objc private func sleeping() { js("pause(true)") }
    @objc private func waking() { js("pause(\(paused))") }

    @objc private func togglePause() {
        paused.toggle()
        js("pause(\(paused))")
        menu()
    }

    @objc private func stone() {
        js("drop(\(Double.random(in: 0.2...0.8)), \(Double.random(in: 0.2...0.8)), 1.8, 3)")
    }
}

let app = NSApplication.shared
let delegate = Wallpaper()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
