//! 可控应用内浏览器。
//!
//! 与本插件原有的 `open`/`close`（iOS 的 SFSafariViewController，供 App 自身登录用）
//! 是**两套并存**的东西：SFSafariViewController 读不到 cookie、也拦不到导航，无法
//! 用来复刻第三方登录；这里这套则是在三端都用可控的 WebView，能拦截导航、注入 JS、
//! 读 cookie。
//!
//! 平台实现：
//!   * 桌面：Tauri 的子 WebviewWindow，导航拦截走 `on_navigation`。
//!   * iOS：WKWebView（`ios/Sources/ControlledBrowser.swift`）。
//!   * Android：Dialog 里的 WebView（`android/.../ControlledBrowser.kt`）。
//!
//! 跨平台共用的部分（id 分配、拦截前缀匹配、等待者唤醒、事件广播）都在这里，
//! 各平台只负责"开窗 / 导航 / 取值 / 关窗"这几件事。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenControlledRequest {
    pub url: String,
    pub title: Option<String>,
    pub user_agent: Option<String>,
    /// 命中任一前缀就取消导航并上报。
    #[serde(default)]
    pub intercept_prefixes: Vec<String>,
    #[serde(default)]
    pub close_on_intercept: bool,
    #[serde(default)]
    pub ephemeral: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserEventKind {
    Navigated,
    Intercepted,
    LoadFinished,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserEvent {
    pub id: u32,
    pub kind: BrowserEventKind,
    pub url: String,
}

/// 单个浏览器实例的跨平台状态。
struct BrowserSlot {
    intercept_prefixes: Vec<String>,
    close_on_intercept: bool,
    /// 已经命中的拦截 URL（可能在插件来 wait 之前就发生了，必须先存住）。
    intercepted: Option<String>,
    closed: bool,
    /// 在等拦截结果的调用方。
    waiters: Vec<oneshot::Sender<Result<String, String>>>,
    last_url: String,
}

impl BrowserSlot {
    fn matches(&self, url: &str) -> bool {
        self.intercept_prefixes
            .iter()
            .any(|prefix| !prefix.is_empty() && url.starts_with(prefix.as_str()))
    }
}

/// 关闭后仍保留的槽位数量上限。
///
/// 关闭不能直接把槽位删掉：`close-on-intercept` 的典型流程是「命中回调 → 自动关窗」，
/// 插件很可能在这之后才来 `wait-for-intercept`，槽位没了就会变成「查无此浏览器」，
/// 而它其实是成功的。所以关闭只做标记、保留已拦截到的 URL，靠这个上限回收。
const MAX_RETAINED_CLOSED: usize = 32;

#[derive(Default)]
pub struct ControlledBrowsers {
    slots: Mutex<HashMap<u32, BrowserSlot>>,
    next_id: AtomicU32,
}

impl ControlledBrowsers {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn allocate(&self, request: &OpenControlledRequest) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut slots = self.lock();

        // 回收多余的已关闭槽位（保留最近的若干个，供迟到的 wait 取结果）。
        let mut closed: Vec<u32> = slots
            .iter()
            .filter(|(_, slot)| slot.closed)
            .map(|(id, _)| *id)
            .collect();
        if closed.len() > MAX_RETAINED_CLOSED {
            closed.sort_unstable();
            for stale in closed
                .iter()
                .take(closed.len() - MAX_RETAINED_CLOSED)
            {
                slots.remove(stale);
            }
        }

        slots.insert(
            id,
            BrowserSlot {
                intercept_prefixes: request.intercept_prefixes.clone(),
                close_on_intercept: request.close_on_intercept,
                intercepted: None,
                closed: false,
                waiters: Vec::new(),
                last_url: request.url.clone(),
            },
        );
        id
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u32, BrowserSlot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// 判断一次导航是否该被拦截，命中则顺带记录并唤醒等待者。
    ///
    /// 桌面端在 `on_navigation` 里调用它，返回 true 表示**取消**这次导航。
    /// 移动端的匹配发生在原生导航回调里（那里必须同步决定，来不及回 Rust 问），
    /// 因此移动端只调用 [`Self::record_intercepted`]。
    pub(crate) fn should_intercept(&self, id: u32, url: &str) -> bool {
        let matched = {
            let mut slots = self.lock();
            let Some(slot) = slots.get_mut(&id) else {
                return false;
            };
            slot.last_url = url.to_string();
            slot.matches(url)
        };
        if matched {
            self.record_intercepted(id, url);
        }
        matched
    }

    /// 记录一次已发生的拦截并唤醒等待者。
    pub fn record_intercepted(&self, id: u32, url: &str) {
        let mut slots = self.lock();
        let Some(slot) = slots.get_mut(&id) else {
            return;
        };
        slot.intercepted = Some(url.to_string());
        slot.last_url = url.to_string();
        for waiter in slot.waiters.drain(..) {
            let _ = waiter.send(Ok(url.to_string()));
        }
    }

    pub(crate) fn should_close_on_intercept(&self, id: u32) -> bool {
        self.lock()
            .get(&id)
            .map(|slot| slot.close_on_intercept)
            .unwrap_or(false)
    }

    pub fn record_url(&self, id: u32, url: &str) {
        if let Some(slot) = self.lock().get_mut(&id) {
            slot.last_url = url.to_string();
        }
    }

    pub fn current_url(&self, id: u32) -> Option<String> {
        self.lock().get(&id).map(|slot| slot.last_url.clone())
    }

    /// 标记已关闭并唤醒等待者。
    ///
    /// 若关闭前已经命中过拦截，等待者拿到的仍是那个 URL——「命中后自动关窗」是
    /// 成功路径，不该因为窗口没了就报错。
    pub fn mark_closed(&self, id: u32) {
        let mut slots = self.lock();
        let Some(slot) = slots.get_mut(&id) else {
            return;
        };
        slot.closed = true;
        let intercepted = slot.intercepted.clone();
        for waiter in slot.waiters.drain(..) {
            let _ = match intercepted.clone() {
                Some(url) => waiter.send(Ok(url)),
                None => waiter.send(Err("browser was closed".to_string())),
            };
        }
    }

    pub fn exists(&self, id: u32) -> bool {
        self.lock().contains_key(&id)
    }

    /// 还开着（未关闭）的浏览器。navigate/eval 这类操作要用它判断。
    pub fn is_open(&self, id: u32) -> bool {
        self.lock()
            .get(&id)
            .map(|slot| !slot.closed)
            .unwrap_or(false)
    }

    /// 登记一个等待者。若拦截已经发生就立刻返回结果。
    pub(crate) fn wait(&self, id: u32) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let (tx, rx) = oneshot::channel();
        let mut slots = self.lock();
        let Some(slot) = slots.get_mut(&id) else {
            return Err(format!("browser {id} not found"));
        };
        // 拦截可能早于 wait 发生（尤其是 close-on-intercept），先看有没有存好的结果。
        if let Some(url) = slot.intercepted.clone() {
            let _ = tx.send(Ok(url));
            return Ok(rx);
        }
        if slot.closed {
            let _ = tx.send(Err("browser was closed".to_string()));
            return Ok(rx);
        }
        slot.waiters.push(tx);
        Ok(rx)
    }
}

/// 把浏览器事件广播给宿主其它部分（插件系统据此投递 browser 事件）。
pub fn emit_browser_event<R: Runtime>(app: &AppHandle<R>, event: BrowserEvent) {
    use tauri::Emitter;
    if let Err(err) = app.emit("astrobox://inappbrowser/event", &event) {
        log_warn(format!("failed to emit in-app browser event: {err}"));
    }
}

fn log_warn(message: String) {
    // 本 crate 不引 log，交给调用方的日志系统；这里只在 debug 下打印。
    #[cfg(debug_assertions)]
    eprintln!("[inappbrowser] {message}");
    #[cfg(not(debug_assertions))]
    let _ = message;
}
