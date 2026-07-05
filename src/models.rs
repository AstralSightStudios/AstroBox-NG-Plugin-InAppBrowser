use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    /// 要在应用内浏览器（SFSafariViewController）中打开的 http/https URL
    pub url: String,
}
