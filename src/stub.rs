//! 桌面端实现（非 iOS / Android）。
//!
//! 文件名沿用历史叫法：原先桌面端是全空实现，现在原有的 `open`/`close`
//! （SFSafariViewController 语义）在桌面仍不支持，但**可控浏览器**在桌面是完整可用的。

use std::sync::Arc;

use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::controlled::{BrowserCookie, ControlledBrowsers, OpenControlledRequest};
use crate::desktop_browser;
use crate::models::*;
use crate::Error;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Inappbrowser<R>> {
    Ok(Inappbrowser {
        app: app.clone(),
        browsers: crate::browsers(),
    })
}

pub struct Inappbrowser<R: Runtime> {
    app: AppHandle<R>,
    browsers: Arc<ControlledBrowsers>,
}

impl<R: Runtime> Inappbrowser<R> {
    /// 兼容旧接口：桌面端没有 SFSafariViewController 语义的"系统内浏览器"。
    pub fn open(&self, _req: OpenRequest) -> crate::Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    pub fn close(&self) -> crate::Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    pub async fn open_controlled(&self, request: OpenControlledRequest) -> crate::Result<u32> {
        desktop_browser::open(&self.app, &self.browsers, request)
    }

    pub async fn wait_for_intercept(
        &self,
        id: u32,
        timeout_ms: Option<u64>,
    ) -> crate::Result<String> {
        crate::wait_for_intercept_shared(&self.browsers, id, timeout_ms).await
    }

    pub async fn navigate(&self, id: u32, url: String) -> crate::Result<()> {
        desktop_browser::navigate(&self.app, id, url)
    }

    pub fn current_url(&self, id: u32) -> crate::Result<String> {
        self.browsers
            .current_url(id)
            .ok_or(Error::BrowserNotFound(id))
    }

    pub async fn eval(&self, id: u32, script: String) -> crate::Result<String> {
        desktop_browser::eval(&self.app, id, script).await
    }

    pub async fn get_cookies(&self, id: u32, url: String) -> crate::Result<Vec<BrowserCookie>> {
        desktop_browser::get_cookies(&self.app, id, url)
    }

    pub async fn close_controlled(&self, id: u32) -> crate::Result<()> {
        desktop_browser::close(&self.app, &self.browsers, id)
    }
}
