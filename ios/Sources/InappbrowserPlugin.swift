import Foundation
import SafariServices
import Tauri
import UIKit

struct OpenArgs: Decodable, Sendable {
    let url: String
}

// Invoke 不是 Sendable，包一层后只在 MainActor 上回调，避免 Swift 6 并发告警。
private final class InvokeResponder: @unchecked Sendable {
    private let invoke: Invoke

    init(_ invoke: Invoke) {
        self.invoke = invoke
    }

    @MainActor
    func resolve() {
        invoke.resolve()
    }

    @MainActor
    func reject(_ message: String) {
        invoke.reject(message)
    }
}

class InappbrowserPlugin: Plugin {
    // 取当前最顶层的 ViewController，用于 present/dismiss SFSafariViewController。
    @MainActor
    private static func topViewController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes
        let windowScene =
            (scenes.first { $0.activationState == .foregroundActive } as? UIWindowScene)
            ?? (scenes.first as? UIWindowScene)
        let keyWindow =
            windowScene?.windows.first(where: { $0.isKeyWindow })
            ?? windowScene?.windows.first
        var top = keyWindow?.rootViewController
        while let presented = top?.presentedViewController {
            top = presented
        }
        return top
    }

    // 在应用内 Safari 打开登录页（满足 App Store「不得跳系统浏览器登录」的要求）。
    // OAuth 完成后 Casdoor 会重定向并唤起 astrobox:// deep link，由既有 handler 完成登录；
    // 前端在登录成功事件里再调用 close 把本控制器关掉。
    @objc public func open(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(OpenArgs.self)
        let responder = InvokeResponder(invoke)
        let urlString = args.url
        Task { @MainActor in
            guard let url = URL(string: urlString),
                let scheme = url.scheme?.lowercased(),
                scheme == "http" || scheme == "https"
            else {
                responder.reject("invalid_url")
                return
            }
            let controller = SFSafariViewController(url: url)
            controller.dismissButtonStyle = .done
            controller.modalPresentationStyle = .automatic
            guard let presenter = InappbrowserPlugin.topViewController() else {
                responder.reject("no_presenter")
                return
            }
            presenter.present(controller, animated: true)
            responder.resolve()
        }
    }

    @objc public func close(_ invoke: Invoke) throws {
        let responder = InvokeResponder(invoke)
        Task { @MainActor in
            // 顶层若是我们呈现的 SFSafariViewController 就关掉；找不到也幂等成功
            if let top = InappbrowserPlugin.topViewController(), top is SFSafariViewController {
                top.dismiss(animated: true)
            }
            responder.resolve()
        }
    }
}

@_cdecl("init_plugin_inappbrowser")
func initPluginInappbrowser() -> Plugin {
    return InappbrowserPlugin()
}
