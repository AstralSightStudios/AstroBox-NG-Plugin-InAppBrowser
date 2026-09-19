use serde::{Serialize, ser::Serializer};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[cfg(any(target_os = "ios", target_os = "android"))]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
    #[error("in-app browser is not supported on this platform")]
    UnsupportedPlatform,
    #[error("invalid url")]
    InvalidUrl,
    #[error("in-app browser {0} not found")]
    BrowserNotFound(u32),
    #[error("in-app browser error: {0}")]
    Browser(String),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}
