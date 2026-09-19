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

    @MainActor
    func resolveValue(_ value: String) {
        invoke.resolve(["value": value] as JsonObject)
    }

    @MainActor
    func resolveCookies(_ cookies: [JsonObject]) {
        invoke.resolve(["cookies": cookies] as JsonObject)
    }
}

// OpenControlledArgs 持有非 Sendable 的 Channel（Tauri iOS API 的 class，无
// Sendable conformance），整个结构体无法隐式满足 Sendable。与 InvokeResponder
// 同理包一层 @unchecked Sendable：只在 Task { @MainActor in } 里被消费一次，
// Channel 也只会由 @MainActor 的 ControlledBrowserSession 使用，无真实竞争。
private struct OpenControlledRequest: @unchecked Sendable {
    let args: OpenControlledArgs
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

    // MARK: - 可控浏览器（WKWebView）
    //
    // 上面的 open/close 是 SFSafariViewController，拦不到导航也读不到 cookie，
    // 只够 App 自己登录用。插件要复刻第三方登录走下面这组，实现见 ControlledBrowser.swift。

    @objc public func openControlled(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(OpenControlledArgs.self)
        let responder = InvokeResponder(invoke)
        let request = OpenControlledRequest(args: args)
        Task { @MainActor in
            if let failure = ControlledBrowserRegistry.shared.open(request.args) {
                responder.reject(failure)
            } else {
                responder.resolve()
            }
        }
    }

    @objc public func navigateControlled(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(NavigateArgs.self)
        let responder = InvokeResponder(invoke)
        Task { @MainActor in
            guard let session = ControlledBrowserRegistry.shared.session(args.id) else {
                responder.reject("browser_not_found")
                return
            }
            guard let url = URL(string: args.url) else {
                responder.reject("invalid_url")
                return
            }
            session.webView.load(URLRequest(url: url))
            responder.resolve()
        }
    }

    @objc public func evalControlled(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(EvalArgs.self)
        let responder = InvokeResponder(invoke)
        Task { @MainActor in
            guard let session = ControlledBrowserRegistry.shared.session(args.id) else {
                responder.reject("browser_not_found")
                return
            }
            session.webView.evaluateJavaScript(args.script) { value, error in
                Task { @MainActor in
                    if let error {
                        responder.reject(error.localizedDescription)
                        return
                    }
                    // 统一回 JSON 文本，与桌面端 eval_with_callback 的语义对齐。
                    responder.resolveValue(InappbrowserPlugin.serializeJs(value))
                }
            }
        }
    }

    @objc public func getCookies(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(CookiesArgs.self)
        let responder = InvokeResponder(invoke)
        Task { @MainActor in
            guard let session = ControlledBrowserRegistry.shared.session(args.id) else {
                responder.reject("browser_not_found")
                return
            }
            let host = URL(string: args.url)?.host
            session.webView.configuration.websiteDataStore.httpCookieStore.getAllCookies {
                cookies in
                Task { @MainActor in
                    let filtered = cookies.filter { cookie in
                        guard let host else { return true }
                        // domain 常带前导点，按后缀匹配。
                        let domain =
                            cookie.domain.hasPrefix(".")
                            ? String(cookie.domain.dropFirst()) : cookie.domain
                        return host == domain || host.hasSuffix("." + domain)
                    }
                    .map { cookie in
                        [
                            "name": cookie.name,
                            "value": cookie.value,
                            "domain": cookie.domain,
                            "path": cookie.path,
                        ] as JsonObject
                    }
                    responder.resolveCookies(filtered)
                }
            }
        }
    }

    @objc public func closeControlled(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(BrowserIdArgs.self)
        let responder = InvokeResponder(invoke)
        Task { @MainActor in
            ControlledBrowserRegistry.shared.dismiss(id: args.id)
            responder.resolve()
        }
    }

    /// 把 evaluateJavaScript 的返回值序列化成 JSON 文本。
    @MainActor
    static func serializeJs(_ value: Any?) -> String {
        guard let value, !(value is NSNull) else { return "" }
        // 顶层标量不是合法 JSON 根，包一层数组再把括号去掉。
        if JSONSerialization.isValidJSONObject([value]),
            let data = try? JSONSerialization.data(withJSONObject: [value]),
            let text = String(data: data, encoding: .utf8)
        {
            return String(text.dropFirst().dropLast())
        }
        return "\(value)"
    }
}

@_cdecl("init_plugin_inappbrowser")
public func initPluginInappbrowser() -> Plugin {
    return InappbrowserPlugin()
}
