use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;
use crate::Error;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Inappbrowser<R>> {
    Ok(Inappbrowser(app.clone()))
}

/// 非 iOS 平台的空实现，所有调用返回 UnsupportedPlatform
pub struct Inappbrowser<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

impl<R: Runtime> Inappbrowser<R> {
    pub fn open(&self, _req: OpenRequest) -> crate::Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    pub fn close(&self) -> crate::Result<()> {
        Err(Error::UnsupportedPlatform)
    }
}
