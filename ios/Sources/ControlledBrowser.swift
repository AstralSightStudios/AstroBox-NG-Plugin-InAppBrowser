import Foundation
import Tauri
import UIKit
import WebKit

// 可控应用内浏览器（iOS）。
//
// 与同插件的 SFSafariViewController（`open`/`close`，供 App 自身登录用）是两套：
// SFSafariViewController 跑在独立进程，既读不到 cookie 也拦不到导航，没法用来复刻
// 第三方登录。这里用 WKWebView，在 `decidePolicyFor` 里做前缀匹配，命中即 `.cancel`
// —— 自定义 scheme（xxxapp://oauth?code=...）因此不会被系统拿去唤起真正的那个 App，
// 回调参数留在我们手里。
//
// 前缀匹配必须放在原生侧：导航决策要求同步返回放行/取消，来不及回 Rust 问一趟。

struct OpenControlledArgs: Decodable {
    let id: Int
    let url: String
    let title: String?
    let userAgent: String?
    let interceptPrefixes: [String]
    let closeOnIntercept: Bool
    let ephemeral: Bool
    let onEvent: Channel
}

struct BrowserIdArgs: Decodable {
    let id: Int
}

struct NavigateArgs: Decodable {
    let id: Int
    let url: String
}

struct EvalArgs: Decodable {
    let id: Int
    let script: String
}

struct CookiesArgs: Decodable {
    let id: Int
    let url: String
}

@MainActor
final class ControlledBrowserSession: NSObject, WKNavigationDelegate {
    let id: Int
    let interceptPrefixes: [String]
    let closeOnIntercept: Bool
    let onEvent: Channel
    let webView: WKWebView
    weak var controller: UIViewController?
    private var finished = false

    init(
        id: Int,
        interceptPrefixes: [String],
        closeOnIntercept: Bool,
        ephemeral: Bool,
        userAgent: String?,
        onEvent: Channel
    ) {
        self.id = id
        self.interceptPrefixes = interceptPrefixes
        self.closeOnIntercept = closeOnIntercept
        self.onEvent = onEvent

        let config = WKWebViewConfiguration()
        if ephemeral {
            // 非持久数据区：cookie / storage 不落盘，也不与用户正常浏览数据混在一起。
            config.websiteDataStore = .nonPersistent()
        }
        self.webView = WKWebView(frame: .zero, configuration: config)
        if let userAgent, !userAgent.isEmpty {
            self.webView.customUserAgent = userAgent
        }
        super.init()
        self.webView.navigationDelegate = self
    }

    func send(kind: String, url: String) {
        onEvent.send(["id": id, "kind": kind, "url": url] as JsonObject)
    }

    func markClosed() {
        guard !finished else { return }
        finished = true
        send(kind: "closed", url: "")
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        let url = navigationAction.request.url?.absoluteString ?? ""

        if !url.isEmpty, interceptPrefixes.contains(where: { !$0.isEmpty && url.hasPrefix($0) }) {
            send(kind: "intercepted", url: url)
            decisionHandler(.cancel)
            if closeOnIntercept {
                ControlledBrowserRegistry.shared.dismiss(id: id)
            }
            return
        }

        if !url.isEmpty {
            send(kind: "navigated", url: url)
        }
        decisionHandler(.allow)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        send(kind: "load-finished", url: webView.url?.absoluteString ?? "")
    }
}

// 承载 WebView 的控制器，带一个关闭按钮。
@MainActor
final class ControlledBrowserController: UIViewController {
    private let session: ControlledBrowserSession
    private let titleText: String?

    init(session: ControlledBrowserSession, title: String?) {
        self.session = session
        self.titleText = title
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        let bar = UINavigationBar()
        bar.translatesAutoresizingMaskIntoConstraints = false
        let item = UINavigationItem(title: titleText ?? "")
        item.leftBarButtonItem = UIBarButtonItem(
            barButtonSystemItem: .close,
            target: self,
            action: #selector(closeTapped)
        )
        bar.items = [item]
        view.addSubview(bar)

        let webView = session.webView
        webView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(webView)

        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            bar.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: view.trailingAnchor),

            webView.topAnchor.constraint(equalTo: bar.bottomAnchor),
            webView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            webView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }

    @objc private func closeTapped() {
        ControlledBrowserRegistry.shared.dismiss(id: session.id)
    }

    // 用户下拉关闭时也要补发 closed，否则等在 wait-for-intercept 的插件会一直挂着。
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || isMovingFromParent {
            ControlledBrowserRegistry.shared.forget(id: session.id)
        }
    }
}

/// 会话登记表。插件类本身由 Tauri 负责实例化，这里单独放一个 MainActor 单例，
/// 免得把状态塞进插件类、又要和既有的 SFSafariViewController 路径混在一起。
@MainActor
final class ControlledBrowserRegistry {
    static let shared = ControlledBrowserRegistry()

    private var sessions: [Int: ControlledBrowserSession] = [:]

    private init() {}

    static func topViewController() -> UIViewController? {
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

    func session(_ id: Int) -> ControlledBrowserSession? {
        sessions[id]
    }

    func open(_ args: OpenControlledArgs) -> String? {
        guard let url = URL(string: args.url) else { return "invalid_url" }
        guard let presenter = ControlledBrowserRegistry.topViewController() else {
            return "no_presenter"
        }

        let session = ControlledBrowserSession(
            id: args.id,
            interceptPrefixes: args.interceptPrefixes,
            closeOnIntercept: args.closeOnIntercept,
            ephemeral: args.ephemeral,
            userAgent: args.userAgent,
            onEvent: args.onEvent
        )
        let controller = ControlledBrowserController(session: session, title: args.title)
        controller.modalPresentationStyle = .automatic
        session.controller = controller
        sessions[args.id] = session

        presenter.present(controller, animated: true)
        session.webView.load(URLRequest(url: url))
        return nil
    }

    func dismiss(id: Int) {
        guard let session = sessions[id] else { return }
        session.markClosed()
        session.controller?.dismiss(animated: true)
        sessions.removeValue(forKey: id)
    }

    /// 控制器已被系统关掉（用户下拉），只需补发事件并清理。
    func forget(id: Int) {
        guard let session = sessions[id] else { return }
        session.markClosed()
        sessions.removeValue(forKey: id)
    }
}
