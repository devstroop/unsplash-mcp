use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use chromiumoxide::Page;
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Modern desktop UA matching the installed Chrome major (153, macOS).
/// Applied per-tab via stealth mode (which also hides `navigator.webdriver`,
/// patches permissions/plugins/WebGL vendor).
const STEALTH_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";

/// Resolves a usable Chrome/Chromium binary.
fn resolve_chrome_executable() -> Option<PathBuf> {
    // 1. Explicit env override.
    if let Ok(p) = std::env::var("CHROME_PATH") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Some(pb);
        }
    }
    // 2. macOS Google Chrome default.
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    for c in candidates {
        let pb = PathBuf::from(c);
        if pb.exists() {
            return Some(pb);
        }
    }
    // 3. PATH lookup.
    for bin in ["google-chrome", "chromium", "chromium-browser", "chrome"] {
        if let Ok(path) = which_chrome(bin) {
            return Some(path);
        }
    }
    None
}

fn which_chrome(bin: &str) -> Result<PathBuf> {
    let out = std::process::Command::new("which").arg(bin).output()?;
    if !out.status.success() {
        return Err(anyhow!("{bin} not on PATH"));
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        return Err(anyhow!("{bin} not on PATH"));
    }
    Ok(PathBuf::from(p))
}

/// How the Anubis/BotStopper wait ended.
#[derive(Debug)]
enum WaitFail {
    /// Terminal "Oh noes!" / BotStopper page — retrying in the same browser
    /// mode is pointless, but a different browser mode may pass.
    Denied,
    /// Challenge still pending after the timeout budget.
    Timeout(String),
}

/// How a clearance attempt ended.
enum ClearFail {
    /// BotStopper denied this browser mode; another mode may still pass.
    Denied,
    /// Anything else (launch/tab/timeout errors) with the real cause.
    Fatal(anyhow::Error),
}

/// Self-managing browser pool. The server owns the entire anti-bot strategy:
///
/// 1. Hardened headless Chrome (new-headless, stealth scripts, no
///    `--enable-automation`, persistent profile) — invisible, tried first.
/// 2. If BotStopper denies headless, a server-managed headed Chrome is
///    launched automatically and the request is retried there.
///
/// No user setup, no env wiring, no restarts: whoever calls the tools just
/// gets clean results or an honest error. Each tool call opens a fresh tab
/// (Page) and closes it afterwards; browsers live for the process lifetime.
pub struct BrowserManager {
    inner: Mutex<BrowserSlots>,
    launch_timeout: Duration,
    anubis_timeout: Duration,
}

#[derive(Default)]
struct BrowserSlots {
    headless: Option<Browser>,
    headed: Option<Browser>,
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BrowserSlots::default()),
            launch_timeout: Duration::from_secs(
                std::env::var("UNSPLASH_CHROME_LAUNCH_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            ),
            anubis_timeout: Duration::from_secs(
                std::env::var("UNSPLASH_ANUBIS_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(45),
            ),
        }
    }

    /// Opens a stealth-hardened tab in the requested browser mode.
    /// The browser stays behind the mutex; `Page` is self-sufficient once
    /// created, so the guard is dropped before returning.
    async fn new_tab(&self, url: &str, headed: bool) -> Result<Page> {
        self.ensure_launched(headed).await?;
        let guard = self.inner.lock().await;
        let slot = if headed {
            &guard.headed
        } else {
            &guard.headless
        };
        let browser = slot
            .as_ref()
            .ok_or_else(|| anyhow!("browser not launched"))?;
        let page = browser.new_page(url).await.context("opening new tab")?;
        drop(guard);

        // Stealth scripts inject on document creation, so reload once to make
        // sure the evaluated document has them (the initial load predates them).
        page.enable_stealth_mode_with_agent(STEALTH_UA)
            .await
            .context("enabling stealth mode")?;
        page.reload().await.context("reloading with stealth")?;
        Ok(page)
    }

    async fn ensure_launched(&self, headed: bool) -> Result<()> {
        {
            let guard = self.inner.lock().await;
            let slot = if headed {
                &guard.headed
            } else {
                &guard.headless
            };
            if slot.is_some() {
                return Ok(());
            }
        }
        let mut guard = self.inner.lock().await;
        // Double-check after acquiring the write lock.
        let slot = if headed {
            &mut guard.headed
        } else {
            &mut guard.headless
        };
        if slot.is_some() {
            return Ok(());
        }
        let browser = self.launch(headed).await?;
        *slot = Some(browser);
        Ok(())
    }

    async fn launch(&self, headed: bool) -> Result<Browser> {
        let mode = if headed { "headed" } else { "headless" };
        match self.try_launch(headed).await {
            Ok(browser) => Ok(browser),
            Err(e) if is_singleton_lock(&e) => {
                // A previous server process died without cleaning up its Chrome
                // child, which still holds the profile lock. Reap processes
                // using OUR mode-specific profile dir and retry once — fully
                // automatic, no user involvement.
                warn!("stale Chrome lock on our profile; reaping orphaned Chrome and retrying");
                if let Some(dir) = self.profile_dir(headed) {
                    reap_profile_processes(&dir);
                }
                self.try_launch(headed)
                    .await
                    .with_context(|| format!("launching {mode} Chrome (after stale-lock cleanup)"))
            }
            Err(e) => Err(e.context(format!("launching {mode} Chrome"))),
        }
    }

    /// Profile dir for a browser mode. Separate subdirs per mode so headless
    /// and headed browsers never fight over one SingletonLock.
    fn profile_dir(&self, headed: bool) -> Option<PathBuf> {
        let base = std::env::var("UNSPLASH_CHROME_PROFILE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .or_else(default_profile_dir)?;
        Some(base.join(if headed { "headed" } else { "headless" }))
    }

    async fn try_launch(&self, headed: bool) -> Result<Browser> {
        let mut builder = BrowserConfig::builder()
            .launch_timeout(self.launch_timeout)
            .no_sandbox()
            .window_size(1366, 900)
            // Drop chromiumoxide's default `--enable-automation` switch — a
            // well-known bot signal — while keeping the other sane defaults.
            .disable_default_args()
            .args(DEFAULT_CHROME_ARGS.iter().copied())
            .arg("--no-default-browser-check")
            .arg("--disable-gpu");

        if headed {
            // Visible window, headful UA/feature set. Used automatically only
            // after headless is denied — no user action involved.
            builder = builder.with_head();
            info!("launching server-managed headed Chrome (fallback after headless denial)");
        } else {
            // New headless exposes a headful-like UA and feature set; the old
            // `--headless` mode is trivially detected ("Oh noes!").
            builder = builder.new_headless_mode();
            // Hides `navigator.webdriver` (disable-blink-features=AutomationControlled).
            builder = builder.hide();
        }

        if let Some(exe) = resolve_chrome_executable() {
            info!(path = %exe.display(), "using Chrome executable");
            builder = builder.chrome_executable(exe);
        } else {
            info!("no explicit Chrome found on well-known paths; letting chromiumoxide resolve it");
        }

        // Persistent profile dir keeps Anubis clearance cookies across restarts.
        // Defaults to ~/.cache/unsplash-mcp/chrome-profile so repeat tool calls
        // don't re-solve the challenge every process launch.
        if let Some(dir) = self.profile_dir(headed) {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                warn!(path = %dir.display(), error = %e, "could not create Chrome profile dir");
            } else {
                // Drop dead lock files from a crashed predecessor when no live
                // process uses this profile; a live holder is handled via the
                // SingletonLock retry in `launch`.
                clear_dead_singleton_files(&dir);
                info!(path = %dir.display(), "using persistent Chrome profile");
                builder = builder.user_data_dir(dir);
            }
        }

        let config = builder.build().map_err(|e| anyhow!("browser config: {e}"))?;
        let (browser, mut handler) = Browser::launch(config).await?;

        // The handler drives the CDP websocket; it must be polled forever.
        tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    break;
                }
            }
        });

        Ok(browser)
    }

    /// One clearance attempt in the given browser mode.
    async fn try_clear(&self, url: &str, headed: bool) -> std::result::Result<Page, ClearFail> {
        let page = self
            .new_tab(url, headed)
            .await
            .map_err(ClearFail::Fatal)?;
        match self.wait_for_anubis(&page).await {
            Ok(()) => Ok(page),
            Err(WaitFail::Denied) => {
                let _ = page.close().await;
                Err(ClearFail::Denied)
            }
            Err(WaitFail::Timeout(d)) => {
                let _ = page.close().await;
                Err(ClearFail::Fatal(anyhow!(
                    "Unsplash bot-check (Anubis) did not clear within {}s ({d})",
                    self.anubis_timeout.as_secs(),
                )))
            }
        }
    }

    /// Opens a tab on Unsplash and waits until Anubis has cleared.
    /// Headless first; on BotStopper denial automatically retries in a
    /// server-managed headed browser. The caller never touches Chrome.
    pub async fn cleared_page(&self, url: &str) -> Result<Page> {
        match self.try_clear(url, false).await {
            Ok(page) => Ok(page),
            Err(ClearFail::Denied) => {
                warn!("headless Chrome denied by BotStopper; retrying in server-managed headed Chrome");
                match self.try_clear(url, true).await {
                    Ok(page) => {
                        info!("headed Chrome passed the bot-check");
                        Ok(page)
                    }
                    Err(ClearFail::Denied) => Err(anyhow!(
                        "Unsplash denied both headless and headed browsers (BotStopper). \
                         This network's egress appears to be flagged; the block is \
                         network-level, not something browser settings can fix. \
                         Retry later or from a different network."
                    )),
                    Err(ClearFail::Fatal(e)) => {
                        Err(e.context("headed-Chrome fallback also failed"))
                    }
                }
            }
            Err(ClearFail::Fatal(e)) => Err(e),
        }
    }

    async fn wait_for_anubis(&self, page: &Page) -> std::result::Result<(), WaitFail> {
        let deadline = Instant::now() + self.anubis_timeout;
        loop {
            let state: Value = page
                .evaluate(
                    r#"(() => ({
                        title: document.title,
                        challenge: !!document.getElementById('anubis_challenge'),
                        body: (document.body ? document.body.innerText.slice(0, 300) : '')
                    }))()"#,
                )
                .await
                .map(|r| r.into_value().unwrap_or(Value::Null))
                .unwrap_or(Value::Null);

            let title = state.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let challenge = state.get("challenge").and_then(|v| v.as_bool()).unwrap_or(true);
            let body = state.get("body").and_then(|v| v.as_str()).unwrap_or("");

            // BotStopper's "Oh noes!" page is terminal — no JS challenge will
            // ever solve it in this browser mode. Report denial so the caller
            // can switch modes instead of burning the full timeout.
            if body.contains("BotStopper") || title == "Oh noes!" {
                return Err(WaitFail::Denied);
            }
            if !challenge && !title.to_lowercase().contains("not a bot") {
                return Ok(());
            }
            if Instant::now() >= deadline {
                // Best-effort diagnostics for the failure page.
                let diag: Value = page
                    .evaluate(
                        r#"(() => ({
                            url: location.href,
                            title: document.title,
                            body: (document.body ? document.body.innerText.slice(0, 500) : '')
                        }))()"#,
                    )
                    .await
                    .map(|r| r.into_value().unwrap_or(Value::Null))
                    .unwrap_or(Value::Null);
                warn!(title, challenge, diag = %diag, "Anubis challenge did not clear in time");
                return Err(WaitFail::Timeout(diag.to_string()));
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
    }

    /// GETs a `/napi/...` JSON endpoint *inside* the cleared page context so
    /// Anubis cookies apply, and returns the parsed JSON.
    pub async fn fetch_napi(&self, napi_path: &str) -> Result<Value> {
        let page = self
            .cleared_page("https://unsplash.com/")
            .await
            .context("preparing cleared tab")?;

        // Escape for embedding in a JS string literal.
        let escaped = napi_path.replace('\\', "\\\\").replace('\'', "\\'");
        let js = format!(
            r#"(async () => {{
                const res = await fetch('{escaped}', {{ headers: {{ 'Accept': 'application/json' }}, credentials: 'same-origin' }});
                const text = await res.text();
                return {{ status: res.status, text: text.slice(0, 500000) }};
            }})()"#
        );

        let result: Value = page
            .evaluate(js.as_str())
            .await
            .context("fetching napi endpoint in page")?
            .into_value()
            .map_err(|e| anyhow!("decoding fetch result: {e}"))?;

        // Close the tab promptly; the browser itself stays alive.
        let _ = page.close().await;

        let status = result.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        if status == 401 || status == 403 {
            return Err(anyhow!(
                "Unsplash rejected the request (HTTP {status}). Anubis clearance may have expired; retry."
            ));
        }
        if !(200..300).contains(&status) {
            return Err(anyhow!(
                "Unsplash napi {napi_path} returned HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&text).map_err(|e| anyhow!("parsing napi JSON: {e}"))
    }
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

fn default_profile_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    if home.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".cache/unsplash-mcp/chrome-profile"))
}

/// True when a launch failure is Chrome refusing a locked profile dir
/// (stale child from a killed predecessor still holds SingletonLock).
fn is_singleton_lock(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}");
    s.contains("SingletonLock") || s.contains("ProcessSingleton")
}

/// Kill processes whose command line references our profile dir (orphaned
/// Chrome children of a dead server process), then SIGKILL survivors.
fn reap_profile_processes(profile: &PathBuf) {
    let marker = profile.to_string_lossy().to_string();
    let _ = std::process::Command::new("pkill")
        .args(["-f", &marker])
        .output();
    std::thread::sleep(Duration::from_secs(2));
    let _ = std::process::Command::new("pkill")
        .args(["-9", "-f", &marker])
        .output();
    clear_dead_singleton_files(profile);
}

/// Remove Singleton lock files only when no live process uses the profile.
/// Chrome recreates them on next launch; deleting a live holder's lock would
/// corrupt it, so the pgrep guard matters.
fn clear_dead_singleton_files(profile: &PathBuf) {
    let marker = profile.to_string_lossy().to_string();
    let live = std::process::Command::new("pgrep")
        .args(["-f", &marker])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if live {
        return;
    }
    for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(profile.join(name));
    }
}

/// Chromium launch flags: puppeteer's sane defaults minus `--enable-automation`
/// (a well-known bot signal that trips BotStopper/Anubis).
const DEFAULT_CHROME_ARGS: &[&str] = &[
    "--disable-background-networking",
    "--enable-features=NetworkService,NetworkServiceInProcess",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-breakpad",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-default-apps",
    "--disable-dev-shm-usage",
    "--disable-features=TranslateUI",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--disable-sync",
    "--force-color-profile=srgb",
    "--metrics-recording-only",
    "--no-first-run",
    "--password-store=basic",
    "--use-mock-keychain",
    "--enable-blink-features=IdleDetection",
    "--lang=en-US",
];

/// Shared handle type used by the MCP tools.
pub type SharedBrowser = Arc<BrowserManager>;
