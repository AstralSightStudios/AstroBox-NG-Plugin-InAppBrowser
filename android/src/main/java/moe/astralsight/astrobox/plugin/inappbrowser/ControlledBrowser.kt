package moe.astralsight.astrobox.plugin.inappbrowser

import android.app.Activity
import android.app.Dialog
import android.graphics.Color
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject

/**
 * 可控应用内浏览器（Android）。
 *
 * 用全屏 Dialog 承载一个 WebView，而不是新开 Activity：不需要往宿主 App 的
 * AndroidManifest 里注册组件，生命周期也跟着当前 Activity 走。
 *
 * 拦截在 `shouldOverrideUrlLoading` 里做前缀匹配，命中就返回 true（**吃掉**这次
 * 导航）。自定义 scheme（xxxapp://oauth?code=...）因此不会被系统拿去唤起真正的
 * 那个 App，回调参数留在我们手里。匹配必须在这里做：该回调要求同步返回，
 * 来不及回 Rust 问一趟。
 */
class ControlledBrowserSession(
    private val activity: Activity,
    val id: Int,
    private val interceptPrefixes: List<String>,
    private val closeOnIntercept: Boolean,
    private val ephemeral: Boolean,
    private val userAgent: String?,
    private val title: String?,
    private val onEvent: Channel,
) {
    private var dialog: Dialog? = null
    private var webView: WebView? = null
    private var finished = false

    fun send(kind: String, url: String) {
        val payload = JSObject()
        payload.put("id", id)
        payload.put("kind", kind)
        payload.put("url", url)
        onEvent.send(payload)
    }

    private fun markClosed() {
        if (finished) return
        finished = true
        send("closed", "")
    }

    fun open(url: String) {
        val web = WebView(activity)
        webView = web
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        if (!userAgent.isNullOrEmpty()) {
            web.settings.userAgentString = userAgent
        }

        val cookieManager = CookieManager.getInstance()
        cookieManager.setAcceptCookie(true)
        cookieManager.setAcceptThirdPartyCookies(web, true)
        if (ephemeral) {
            // Android 的 CookieManager 是进程级共享的，没有 iOS 那种 nonPersistent
            // 数据区。这里只能尽力而为：关闭时不落盘，避免污染后续会话。
            web.settings.cacheMode = android.webkit.WebSettings.LOAD_NO_CACHE
        }

        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView?,
                request: WebResourceRequest?,
            ): Boolean {
                val target = request?.url?.toString().orEmpty()
                if (target.isNotEmpty() && interceptPrefixes.any { it.isNotEmpty() && target.startsWith(it) }) {
                    send("intercepted", target)
                    if (closeOnIntercept) {
                        activity.runOnUiThread { close() }
                    }
                    // true = 这次导航由我们接管，WebView 不再继续。
                    return true
                }
                if (target.isNotEmpty()) {
                    send("navigated", target)
                }
                return false
            }

            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                send("load-finished", url.orEmpty())
            }
        }

        val root = LinearLayout(activity)
        root.orientation = LinearLayout.VERTICAL
        root.setBackgroundColor(Color.WHITE)

        val bar = LinearLayout(activity)
        bar.orientation = LinearLayout.HORIZONTAL
        bar.setPadding(24, 24, 24, 24)

        val closeButton = Button(activity)
        closeButton.text = "×"
        closeButton.setOnClickListener { close() }
        bar.addView(closeButton)

        val titleView = TextView(activity)
        titleView.text = title.orEmpty()
        titleView.setPadding(24, 0, 0, 0)
        bar.addView(titleView)

        root.addView(
            bar,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ),
        )
        root.addView(
            web,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            ),
        )

        val created = Dialog(activity, android.R.style.Theme_Material_Light_NoActionBar)
        created.setContentView(
            root,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        // 用户按返回键/点外部关闭时，也要补发 closed，否则等待中的插件会一直挂着。
        created.setOnDismissListener { markClosed() }
        dialog = created
        created.show()

        web.loadUrl(url)
    }

    fun navigate(url: String) {
        activity.runOnUiThread { webView?.loadUrl(url) }
    }

    fun evaluate(script: String, callback: (String) -> Unit) {
        activity.runOnUiThread {
            val web = webView
            if (web == null) {
                callback("")
                return@runOnUiThread
            }
            web.evaluateJavascript(script) { value ->
                // evaluateJavascript 回来的本身就是 JSON 文本，"null" 归一成空串。
                callback(if (value == null || value == "null") "" else value)
            }
        }
    }

    fun cookies(url: String): List<JSObject> {
        val raw = CookieManager.getInstance().getCookie(url) ?: return emptyList()
        val host = try {
            android.net.Uri.parse(url).host.orEmpty()
        } catch (_: Exception) {
            ""
        }
        // getCookie 只给 "a=b; c=d"，拿不到 domain/path，按请求 URL 回填。
        return raw.split(";").mapNotNull { part ->
            val trimmed = part.trim()
            val separator = trimmed.indexOf('=')
            if (separator <= 0) return@mapNotNull null
            val item = JSObject()
            item.put("name", trimmed.substring(0, separator))
            item.put("value", trimmed.substring(separator + 1))
            item.put("domain", host)
            item.put("path", "/")
            item
        }
    }

    fun close() {
        activity.runOnUiThread {
            markClosed()
            webView?.destroy()
            webView = null
            dialog?.let { existing ->
                existing.setOnDismissListener(null)
                if (existing.isShowing) existing.dismiss()
            }
            dialog = null
        }
    }
}
