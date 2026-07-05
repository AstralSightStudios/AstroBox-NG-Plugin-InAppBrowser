use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use models::*;

#[cfg(target_os = "ios")]
mod mobile;
#[cfg(not(target_os = "ios"))]
mod stub;

mod error;
mod models;

pub use error::{Error, Result};

#[cfg(target_os = "ios")]
use mobile::Inappbrowser;
#[cfg(not(target_os = "ios"))]
use stub::Inappbrowser;

pub trait InappbrowserExt<R: Runtime> {
    fn inappbrowser(&self) -> &Inappbrowser<R>;
}

impl<R: Runtime, T: Manager<R>> crate::InappbrowserExt<R> for T {
    fn inappbrowser(&self) -> &Inappbrowser<R> {
        self.state::<Inappbrowser<R>>().inner()
    }
}

/// Initializes the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("inappbrowser")
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            let inappbrowser = mobile::init(app, api)?;
            #[cfg(not(target_os = "ios"))]
            let inappbrowser = stub::init(app, api)?;
            app.manage(inappbrowser);
            Ok(())
        })
        .build()
}
