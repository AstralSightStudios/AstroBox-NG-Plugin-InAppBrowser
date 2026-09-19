use std::sync::Arc;

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use models::*;

#[cfg(any(target_os = "ios", target_os = "android"))]
mod mobile;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod stub;

pub mod controlled;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod desktop_browser;

mod error;
mod models;

pub use controlled::{
    BrowserCookie, BrowserEvent, BrowserEventKind, ControlledBrowsers, OpenControlledRequest,
};
pub use error::{Error, Result};

#[cfg(any(target_os = "ios", target_os = "android"))]
use mobile::Inappbrowser;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use stub::Inappbrowser;

pub trait InappbrowserExt<R: Runtime> {
    fn inappbrowser(&self) -> &Inappbrowser<R>;
}

impl<R: Runtime, T: Manager<R>> crate::InappbrowserExt<R> for T {
    fn inappbrowser(&self) -> &Inappbrowser<R> {
        self.state::<Inappbrowser<R>>().inner()
    }
}

/// 跨平台共享的浏览器登记表。各平台实现只管开窗/关窗，拦截判定与等待都在这里。
pub(crate) fn browsers() -> Arc<ControlledBrowsers> {
    use std::sync::OnceLock;
    static BROWSERS: OnceLock<Arc<ControlledBrowsers>> = OnceLock::new();
    Arc::clone(BROWSERS.get_or_init(|| Arc::new(ControlledBrowsers::new())))
}

/// 等待拦截命中，跨平台共用。
pub(crate) async fn wait_for_intercept_shared(
    browsers: &Arc<ControlledBrowsers>,
    id: u32,
    timeout_ms: Option<u64>,
) -> Result<String> {
    if !browsers.exists(id) {
        return Err(Error::BrowserNotFound(id));
    }
    let rx = browsers.wait(id).map_err(Error::Browser)?;

    let received = match timeout_ms {
        Some(ms) => match tokio::time::timeout(std::time::Duration::from_millis(ms), rx).await {
            Ok(received) => received,
            Err(_) => return Err(Error::Browser("timed out".to_string())),
        },
        None => rx.await,
    };

    match received {
        Ok(Ok(url)) => Ok(url),
        Ok(Err(err)) => Err(Error::Browser(err)),
        Err(_) => Err(Error::Browser("browser was dropped".to_string())),
    }
}

/// Initializes the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("inappbrowser")
        .setup(|app, api| {
            #[cfg(any(target_os = "ios", target_os = "android"))]
            let inappbrowser = mobile::init(app, api)?;
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            let inappbrowser = stub::init(app, api)?;
            app.manage(inappbrowser);
            Ok(())
        })
        .build()
}
