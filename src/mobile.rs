//! iOS / Android 的可控浏览器桥接。
//!
//! 原生侧负责开窗、导航与**拦截判定**（导航回调必须同步返回是否放行，来不及回
//! Rust 问一趟），Rust 侧负责跨平台的状态与等待唤醒。原生通过 `Channel` 把导航
//! 事件回传，这里落到共享登记表里。

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::controlled::{
    emit_browser_event, BrowserCookie, BrowserEvent, BrowserEventKind, ControlledBrowsers,
    OpenControlledRequest,
};
use crate::models::*;
use crate::Error;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_inappbrowser);

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<Inappbrowser<R>> {
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_inappbrowser)?;
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin("moe.astralsight.astrobox.plugin.inappbrowser", "InappbrowserPlugin")?;

    Ok(Inappbrowser {
        handle,
        app: app.clone(),
        browsers: crate::browsers(),
    })
}

/// 原生回传的事件。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeBrowserEvent {
    id: u32,
    /// navigated / intercepted / load-finished / closed
    kind: String,
    #[serde(default)]
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenControlledArgs {
    id: u32,
    url: String,
    title: Option<String>,
    user_agent: Option<String>,
    intercept_prefixes: Vec<String>,
    close_on_intercept: bool,
    ephemeral: bool,
    on_event: Channel<NativeBrowserEvent>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserIdArgs {
    id: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigateArgs {
    id: u32,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalArgs {
    id: u32,
    script: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CookiesArgs {
    id: u32,
    url: String,
}

#[derive(Deserialize)]
struct EvalResult {
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
struct CookiesResult {
    #[serde(default)]
    cookies: Vec<BrowserCookie>,
}

/// Access to the in-app browser APIs.
pub struct Inappbrowser<R: Runtime> {
    handle: PluginHandle<R>,
    // PluginHandle 不提供取 AppHandle 的方法，事件广播要用，这里自己留一份。
    app: AppHandle<R>,
    browsers: Arc<ControlledBrowsers>,
}

impl<R: Runtime> Inappbrowser<R> {
    /// 旧接口：iOS 的 SFSafariViewController，供 App 自身 OAuth 登录用。
    /// 它读不到 cookie、也拦不到导航，插件要劫持回调请用 `open_controlled`。
    pub fn open(&self, req: OpenRequest) -> crate::Result<()> {
        self.handle.run_mobile_plugin("open", req).map_err(Into::into)
    }

    pub fn close(&self) -> crate::Result<()> {
        self.handle.run_mobile_plugin("close", ()).map_err(Into::into)
    }

    pub async fn open_controlled(&self, request: OpenControlledRequest) -> crate::Result<u32> {
        let id = self.browsers.allocate(&request);
        let app = self.app.clone();
        let browsers = Arc::clone(&self.browsers);

        let on_event = Channel::new(move |event| {
            let event: NativeBrowserEvent = match event.deserialize() {
                Ok(event) => event,
                Err(_) => return Ok(()),
            };
            let kind = match event.kind.as_str() {
                "navigated" => BrowserEventKind::Navigated,
                "intercepted" => BrowserEventKind::Intercepted,
                "load-finished" => BrowserEventKind::LoadFinished,
                "closed" => BrowserEventKind::Closed,
                _ => return Ok(()),
            };

            match kind {
                // 拦截的匹配发生在原生侧，这里只记录结果并唤醒等待者。
                BrowserEventKind::Intercepted => {
                    browsers.record_intercepted(event.id, &event.url)
                }
                BrowserEventKind::Closed => browsers.mark_closed(event.id),
                _ => browsers.record_url(event.id, &event.url),
            }

            emit_browser_event(
                &app,
                BrowserEvent {
                    id: event.id,
                    kind,
                    url: event.url,
                },
            );
            Ok(())
        });

        self.handle
            .run_mobile_plugin_async::<()>(
                "openControlled",
                OpenControlledArgs {
                    id,
                    url: request.url,
                    title: request.title,
                    user_agent: request.user_agent,
                    intercept_prefixes: request.intercept_prefixes,
                    close_on_intercept: request.close_on_intercept,
                    ephemeral: request.ephemeral,
                    on_event,
                },
            )
            .await
            .map_err(|err| {
                // 原生开窗失败，登记表里的槽位不能留着。
                self.browsers.mark_closed(id);
                Error::from(err)
            })?;

        Ok(id)
    }

    pub async fn wait_for_intercept(
        &self,
        id: u32,
        timeout_ms: Option<u64>,
    ) -> crate::Result<String> {
        crate::wait_for_intercept_shared(&self.browsers, id, timeout_ms).await
    }

    pub async fn navigate(&self, id: u32, url: String) -> crate::Result<()> {
        self.handle
            .run_mobile_plugin_async("navigateControlled", NavigateArgs { id, url })
            .await
            .map_err(Into::into)
    }

    pub fn current_url(&self, id: u32) -> crate::Result<String> {
        self.browsers
            .current_url(id)
            .ok_or(Error::BrowserNotFound(id))
    }

    pub async fn eval(&self, id: u32, script: String) -> crate::Result<String> {
        let result: EvalResult = self
            .handle
            .run_mobile_plugin_async("evalControlled", EvalArgs { id, script })
            .await?;
        Ok(result.value)
    }

    pub async fn get_cookies(&self, id: u32, url: String) -> crate::Result<Vec<BrowserCookie>> {
        let result: CookiesResult = self
            .handle
            .run_mobile_plugin_async("getCookies", CookiesArgs { id, url })
            .await?;
        Ok(result.cookies)
    }

    pub async fn close_controlled(&self, id: u32) -> crate::Result<()> {
        self.browsers.mark_closed(id);
        self.handle
            .run_mobile_plugin_async("closeControlled", BrowserIdArgs { id })
            .await
            .map_err(Into::into)
    }
}
