use serde::de::DeserializeOwned;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::models::*;

tauri::ios_plugin_binding!(init_plugin_inappbrowser);

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<Inappbrowser<R>> {
    let handle = api.register_ios_plugin(init_plugin_inappbrowser)?;
    Ok(Inappbrowser(handle))
}

/// Access to the in-app browser (SFSafariViewController) APIs.
pub struct Inappbrowser<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> Inappbrowser<R> {
    pub fn open(&self, req: OpenRequest) -> crate::Result<()> {
        self.0.run_mobile_plugin("open", req).map_err(Into::into)
    }

    pub fn close(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("close", ()).map_err(Into::into)
    }
}
