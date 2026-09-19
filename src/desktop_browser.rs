//! 桌面端（macOS / Windows / Linux）的可控浏览器实现。
//!
//! 用 Tauri 的子 WebviewWindow。导航拦截靠 `on_navigation`：回调返回 false 即
//! **取消**这次导航，自定义 scheme（`xxxapp://oauth?code=...`）也会走到这里，
//! 因此不会被系统拿去唤起真正的那个 App —— 这正是劫持回调所需要的。

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::controlled::{
    BrowserCookie, BrowserEvent, BrowserEventKind, ControlledBrowsers, OpenControlledRequest,
    emit_browser_event,
};
use crate::{Error, Result};

fn window_label(id: u32) -> String {
    format!("astrobox-inappbrowser-{id}")
}

pub(crate) fn open<R: Runtime>(
    app: &AppHandle<R>,
    browsers: &Arc<ControlledBrowsers>,
    request: OpenControlledRequest,
) -> Result<u32> {
    let url = url::Url::parse(&request.url).map_err(|_| Error::InvalidUrl)?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(Error::InvalidUrl),
    }

    let id = browsers.allocate(&request);
    let label = window_label(id);

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(request.title.clone().unwrap_or_else(|| "".to_string()))
        .inner_size(
            request.width.unwrap_or(480) as f64,
            request.height.unwrap_or(720) as f64,
        )
        .incognito(request.ephemeral);

    if let Some(user_agent) = request.user_agent.as_deref() {
        builder = builder.user_agent(user_agent);
    }

    builder = builder.on_navigation({
        let browsers = Arc::clone(browsers);
        let app = app.clone();
        move |url| {
            let url = url.to_string();
            if browsers.should_intercept(id, &url) {
                emit_browser_event(
                    &app,
                    BrowserEvent {
                        id,
                        kind: BrowserEventKind::Intercepted,
                        url: url.clone(),
                    },
                );
                if browsers.should_close_on_intercept(id) {
                    if let Some(window) = app.get_webview_window(&window_label(id)) {
                        let _ = window.destroy();
                    }
                }
                // 返回 false = 取消导航，系统不会去唤起自定义 scheme 对应的 App。
                return false;
            }

            emit_browser_event(
                &app,
                BrowserEvent {
                    id,
                    kind: BrowserEventKind::Navigated,
                    url,
                },
            );
            true
        }
    });

    builder = builder.on_page_load({
        let browsers = Arc::clone(browsers);
        let app = app.clone();
        move |webview, _payload| {
            let url = webview
                .url()
                .map(|url| url.to_string())
                .unwrap_or_default();
            browsers.record_url(id, &url);
            emit_browser_event(
                &app,
                BrowserEvent {
                    id,
                    kind: BrowserEventKind::LoadFinished,
                    url,
                },
            );
        }
    });

    let window = builder.build().map_err(|err| Error::Browser(err.to_string()))?;

    // 用户手动关窗时也要唤醒还在 wait 的插件，否则它会一直挂着。
    window.on_window_event({
        let browsers = Arc::clone(browsers);
        let app = app.clone();
        move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                browsers.mark_closed(id);
                emit_browser_event(
                    &app,
                    BrowserEvent {
                        id,
                        kind: BrowserEventKind::Closed,
                        url: String::new(),
                    },
                );
            }
        }
    });

    Ok(id)
}

pub(crate) fn navigate<R: Runtime>(app: &AppHandle<R>, id: u32, url: String) -> Result<()> {
    let parsed = url::Url::parse(&url).map_err(|_| Error::InvalidUrl)?;
    let window = app
        .get_webview_window(&window_label(id))
        .ok_or(Error::BrowserNotFound(id))?;
    window
        .navigate(parsed)
        .map_err(|err| Error::Browser(err.to_string()))
}

pub(crate) async fn eval<R: Runtime>(app: &AppHandle<R>, id: u32, script: String) -> Result<String> {
    let window = app
        .get_webview_window(&window_label(id))
        .ok_or(Error::BrowserNotFound(id))?;

    let (tx, rx) = oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    window
        .eval_with_callback(script, move |value| {
            if let Some(tx) = tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = tx.send(value);
            }
        })
        .map_err(|err| Error::Browser(err.to_string()))?;

    rx.await
        .map_err(|_| Error::Browser("eval callback was dropped".to_string()))
}

pub(crate) fn get_cookies<R: Runtime>(
    app: &AppHandle<R>,
    id: u32,
    url: String,
) -> Result<Vec<BrowserCookie>> {
    let parsed = url::Url::parse(&url).map_err(|_| Error::InvalidUrl)?;
    let window = app
        .get_webview_window(&window_label(id))
        .ok_or(Error::BrowserNotFound(id))?;

    let cookies = window
        .cookies_for_url(parsed)
        .map_err(|err| Error::Browser(err.to_string()))?;

    Ok(cookies
        .into_iter()
        .map(|cookie| BrowserCookie {
            name: cookie.name().to_string(),
            value: cookie.value().to_string(),
            domain: cookie.domain().unwrap_or_default().to_string(),
            path: cookie.path().unwrap_or_default().to_string(),
        })
        .collect())
}

pub(crate) fn close<R: Runtime>(
    app: &AppHandle<R>,
    browsers: &Arc<ControlledBrowsers>,
    id: u32,
) -> Result<()> {
    let window = app.get_webview_window(&window_label(id));
    // 先标记关闭再销毁：销毁事件是异步来的，等待者不该被多挂一会儿。
    browsers.mark_closed(id);
    if let Some(window) = window {
        let _ = window.destroy();
    }
    Ok(())
}
