package moe.astralsight.astrobox.plugin.inappbrowser

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.ConcurrentHashMap

@InvokeArg
class OpenControlledArgs {
    var id: Int = 0
    lateinit var url: String
    var title: String? = null
    var userAgent: String? = null
    var interceptPrefixes: List<String> = emptyList()
    var closeOnIntercept: Boolean = false
    var ephemeral: Boolean = false
    lateinit var onEvent: Channel
}

@InvokeArg
class BrowserIdArgs {
    var id: Int = 0
}

@InvokeArg
class NavigateArgs {
    var id: Int = 0
    lateinit var url: String
}

@InvokeArg
class EvalArgs {
    var id: Int = 0
    lateinit var script: String
}

@InvokeArg
class CookiesArgs {
    var id: Int = 0
    lateinit var url: String
}

/**
 * Android 上只提供**可控浏览器**。
 *
 * iOS 那边还有一组 open/close 走 SFSafariViewController（App 自身登录用），
 * Android 没有对应物，宿主自己用系统浏览器即可，因此这里不实现。
 */
@TauriPlugin
class InappbrowserPlugin(private val activity: Activity) : Plugin(activity) {
    private val sessions = ConcurrentHashMap<Int, ControlledBrowserSession>()

    @Command
    fun openControlled(invoke: Invoke) {
        val args = invoke.parseArgs(OpenControlledArgs::class.java)
        val session = ControlledBrowserSession(
            activity = activity,
            id = args.id,
            interceptPrefixes = args.interceptPrefixes,
            closeOnIntercept = args.closeOnIntercept,
            ephemeral = args.ephemeral,
            userAgent = args.userAgent,
            title = args.title,
            onEvent = args.onEvent,
        )
        sessions[args.id] = session
        activity.runOnUiThread {
            try {
                session.open(args.url)
                invoke.resolve()
            } catch (err: Exception) {
                sessions.remove(args.id)
                invoke.reject(err.message ?: "failed to open in-app browser")
            }
        }
    }

    @Command
    fun navigateControlled(invoke: Invoke) {
        val args = invoke.parseArgs(NavigateArgs::class.java)
        val session = sessions[args.id]
        if (session == null) {
            invoke.reject("browser_not_found")
            return
        }
        session.navigate(args.url)
        invoke.resolve()
    }

    @Command
    fun evalControlled(invoke: Invoke) {
        val args = invoke.parseArgs(EvalArgs::class.java)
        val session = sessions[args.id]
        if (session == null) {
            invoke.reject("browser_not_found")
            return
        }
        session.evaluate(args.script) { value ->
            val result = JSObject()
            result.put("value", value)
            invoke.resolve(result)
        }
    }

    @Command
    fun getCookies(invoke: Invoke) {
        val args = invoke.parseArgs(CookiesArgs::class.java)
        val session = sessions[args.id]
        if (session == null) {
            invoke.reject("browser_not_found")
            return
        }
        val result = JSObject()
        result.put("cookies", session.cookies(args.url))
        invoke.resolve(result)
    }

    @Command
    fun closeControlled(invoke: Invoke) {
        val args = invoke.parseArgs(BrowserIdArgs::class.java)
        sessions.remove(args.id)?.close()
        invoke.resolve()
    }
}
