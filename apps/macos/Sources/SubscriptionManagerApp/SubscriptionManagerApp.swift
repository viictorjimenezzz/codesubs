import AppKit
import SwiftUI

@main
struct SubscriptionManagerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra {
            QuotaPopoverView(store: delegate.store)
        } label: {
            Image(nsImage: menuBarIcon)
                .accessibilityLabel("Subscription Manager")
        }
        .menuBarExtraStyle(.window)
    }

    /// The white Codex logo as a template image, so macOS draws it white on
    /// a dark menu bar and black on a light one.
    private var menuBarIcon: NSImage {
        if let url = Bundle.main.resourceURL?.appendingPathComponent("Assets/codex-mono-light.png"),
           let image = NSImage(contentsOf: url) {
            image.isTemplate = true
            image.size = NSSize(width: 18, height: 18)
            return image
        }
        return NSImage(systemSymbolName: "gauge.with.dots.needle.67percent", accessibilityDescription: "Subscription Manager") ?? NSImage()
    }
}

@MainActor
private final class AppDelegate: NSObject, NSApplicationDelegate {
    let store = AccountViewModel()

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        Task {
            await store.shutDown()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}
