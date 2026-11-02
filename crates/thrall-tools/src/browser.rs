//! A real browser for Thrall: the installed Chrome (or Edge), driven over the DevTools protocol.
//! One browser process for the engine, started on first use with its own profile in the data
//! folder (never the user's everyday profile); each session gets its own tab.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, DialogType, EventJavascriptDialogOpening, HandleJavaScriptDialogParams,
};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;

use crate::constants::{
    BROWSER_ACTION_SECS, BROWSER_DIALOG_MAX_CHARS, BROWSER_LAUNCH_SECS, BROWSER_OPEN_SECS,
    BROWSER_POLL_MS, BROWSER_PRINT_SECS, BROWSER_PROFILE_DIR, BROWSER_SETTLE_MS,
    BROWSER_TIMEOUT_SECS, BROWSER_WINDOW_HEIGHT, BROWSER_WINDOW_WIDTH, CSS_PX_PER_IN,
    LINUX_BROWSERS, MAC_BROWSERS, PDF_MIN_SCALE, PDF_PREVIEW_QUALITY, PDF_SCALE_STEP,
};

struct Running {
    browser: Browser,
    handler: tokio::task::JoinHandle<()>,
    visible: bool,
    pages: HashMap<String, Page>,
}

/// Each person's browser (by its profile folder, so people never share web sign-ins), started
/// when first needed, and each session's tab.
#[derive(Default)]
pub struct Browsers {
    running: tokio::sync::Mutex<HashMap<PathBuf, Running>>,
    /// Pop-ups each session's tab showed (and that were answered) since the last look.
    dialogs: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

/// Run one browser action with a hard time limit: a page that stops answering (a pop-up nobody
/// answered, a script stuck in a loop) fails the step instead of holding the turn.
async fn limited<T>(
    secs: u64,
    action: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(secs), action)
        .await
        .unwrap_or_else(|_| Err(format!("the page stopped responding after {secs} seconds")))
}

/// What to tell the model about a pop-up that was answered for it.
fn dialog_note(kind: &DialogType, message: &str) -> String {
    let message: String = message.chars().take(BROWSER_DIALOG_MAX_CHARS).collect();
    match kind {
        DialogType::Alert => format!("The page showed a message: \"{message}\" (closed it)."),
        DialogType::Confirm => format!(
            "The page asked: \"{message}\" and was answered No. Pop-ups that ask are always \
             answered No; if it needs a Yes, ask the person to do it."
        ),
        DialogType::Prompt => format!("The page asked for input: \"{message}\" and was cancelled."),
        DialogType::Beforeunload => {
            "The page warned about leaving it (unsaved changes?), so it stayed on the page."
                .to_owned()
        }
    }
}

/// Answer the tab's pop-ups as they open (an alert is closed, anything that asks gets No), and
/// note each one for the session. Without this a pop-up freezes the page and every action waits.
async fn answer_dialogs(
    page: &Page,
    session: &str,
    dialogs: Arc<Mutex<HashMap<String, Vec<String>>>>,
) {
    let Ok(mut opened) = page.event_listener::<EventJavascriptDialogOpening>().await else {
        return;
    };
    let page = page.clone();
    let session = session.to_owned();
    tokio::spawn(async move {
        while let Some(dialog) = opened.next().await {
            let accept = matches!(dialog.r#type, DialogType::Alert);
            let _ = page
                .execute(HandleJavaScriptDialogParams::new(accept))
                .await;
            if let Ok(mut all) = dialogs.lock() {
                all.entry(session.clone())
                    .or_default()
                    .push(dialog_note(&dialog.r#type, &dialog.message));
            }
        }
    });
}

/// A Chromium browser to drive: Chrome, else Edge, else Chromium or Brave, where each system
/// installs them (Linux: on the PATH, the way distributions install them).
fn executable() -> Option<PathBuf> {
    let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    if cfg!(windows) {
        let candidates = [
            env("ProgramFiles").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
            env("ProgramFiles(x86)").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
            env("LOCALAPPDATA").map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
            env("ProgramFiles(x86)").map(|p| p.join(r"Microsoft\Edge\Application\msedge.exe")),
            env("ProgramFiles").map(|p| p.join(r"Microsoft\Edge\Application\msedge.exe")),
        ];
        return candidates.into_iter().flatten().find(|p| p.is_file());
    }
    if cfg!(target_os = "macos") {
        // Installed for everyone in /Applications, or just for this person in ~/Applications.
        let home = env("HOME");
        return MAC_BROWSERS
            .iter()
            .flat_map(|app| {
                [
                    Some(PathBuf::from("/Applications").join(app)),
                    home.as_ref().map(|h| h.join("Applications").join(app)),
                ]
            })
            .flatten()
            .find(|p| p.is_file());
    }
    let path = std::env::var_os("PATH")?;
    LINUX_BROWSERS.iter().find_map(|name| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

/// A URL Thrall may open: http(s) only (a bare name gets https://).
pub fn web_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("give a web address".to_owned());
    }
    let lower = raw.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(raw.to_owned());
    }
    if lower.contains("://") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return Err("only http and https pages can be opened".to_owned());
    }
    Ok(format!("https://{raw}"))
}

/// Paper and margins for printing a page to PDF, in inches, and the header and footer
/// templates (none: no running header or footer).
#[derive(Debug, Clone)]
pub struct PrintSetup {
    /// Paper width.
    pub width_in: f64,
    /// Paper height.
    pub height_in: f64,
    /// Top margin (holds the header).
    pub margin_top_in: f64,
    /// Bottom margin (holds the footer).
    pub margin_bottom_in: f64,
    /// Left and right margins.
    pub margin_side_in: f64,
    /// Header and footer HTML templates.
    pub header: Option<(String, String)>,
    /// The page's own CSS (`@page`) sets the paper size, which may change from page to page.
    pub css_page_size: bool,
}
impl Browsers {
    /// Start the browser for `data_dir`'s profile (or restart it to show or hide its window);
    /// the profile folder, which names it.
    async fn ensure(
        &self,
        data_dir: Option<&Path>,
        visible: Option<bool>,
    ) -> Result<PathBuf, String> {
        let profile = data_dir.map_or_else(
            || std::env::temp_dir().join(BROWSER_PROFILE_DIR),
            |d| d.join(BROWSER_PROFILE_DIR),
        );
        let mut running = self.running.lock().await;
        if let Some(r) = running.get(&profile) {
            let alive = !r.handler.is_finished();
            if alive && visible.is_none_or(|v| v == r.visible) {
                return Ok(profile);
            }
        }
        if let Some(mut old) = running.remove(&profile) {
            let _ = old.browser.close().await;
            old.handler.abort();
        }
        let exe = executable().ok_or_else(|| {
            "no Chrome, Edge, Chromium or Brave found; install one (Google Chrome or Chromium) to \
             use the browser"
                .to_owned()
        })?;
        // A browser left from an engine that didn't stop cleanly still holds the profile, and
        // Chrome won't start a second one on it.
        close_leftovers(&profile);
        let visible = visible.unwrap_or(false);
        let mut config = BrowserConfig::builder()
            .chrome_executable(exe)
            .user_data_dir(profile.clone())
            .window_size(BROWSER_WINDOW_WIDTH, BROWSER_WINDOW_HEIGHT)
            .viewport(None)
            .request_timeout(Duration::from_secs(BROWSER_TIMEOUT_SECS))
            .launch_timeout(Duration::from_secs(BROWSER_LAUNCH_SECS));
        config = if visible {
            config.with_head()
        } else {
            config.new_headless_mode()
        };
        let config = config.build()?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| format!("couldn't start the browser: {e}"))?;
        let handler = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        running.insert(
            profile.clone(),
            Running {
                browser,
                handler,
                visible,
                pages: HashMap::new(),
            },
        );
        Ok(profile)
    }

    /// The session's tab in `profile`'s browser, opened if needed.
    async fn page(&self, profile: &Path, session: &str) -> Result<Page, String> {
        let mut running = self.running.lock().await;
        let r = running
            .get_mut(profile)
            .ok_or_else(|| "no page is open; open one first".to_owned())?;
        if let Some(p) = r.pages.get(session) {
            return Ok(p.clone());
        }
        let page = r
            .browser
            .new_page("about:blank")
            .await
            .map_err(|e| format!("couldn't open a tab: {e}"))?;
        answer_dialogs(&page, session, self.dialogs.clone()).await;
        r.pages.insert(session.to_owned(), page.clone());
        Ok(page)
    }

    /// The pop-ups the session's tab showed since the last call, as notes for the model.
    pub fn take_dialogs(&self, session: &str) -> Vec<String> {
        self.dialogs
            .lock()
            .ok()
            .and_then(|mut all| all.remove(session))
            .unwrap_or_default()
    }

    /// The session's tab if it has one.
    async fn existing(&self, session: &str) -> Result<Page, String> {
        let running = self.running.lock().await;
        running
            .values()
            .find_map(|r| r.pages.get(session).cloned())
            .ok_or_else(|| "no page is open in this session; open one first".to_owned())
    }

    /// Open `url` in the session's tab; its title and address after loading.
    pub async fn open(
        &self,
        data_dir: Option<&Path>,
        session: &str,
        url: &str,
        visible: Option<bool>,
    ) -> Result<(String, String), String> {
        let url = web_url(url)?;
        limited(BROWSER_OPEN_SECS, async {
            let profile = self.ensure(data_dir, visible).await?;
            let page = self.page(&profile, session).await?;
            page.goto(url.as_str())
                .await
                .map_err(|e| format!("couldn't open {url}: {e}"))?;
            settle().await;
            Ok(where_now(&page).await)
        })
        .await
    }

    /// Run JavaScript in the session's tab and read the result as a string.
    async fn eval_string(page: &Page, script: &str) -> Result<String, String> {
        page.evaluate(script)
            .await
            .map_err(|e| format!("the page didn't answer: {e}"))?
            .into_value::<String>()
            .map_err(|e| format!("unexpected answer from the page: {e}"))
    }

    /// The page's text (or one element's), and optionally its links.
    pub async fn extract(
        &self,
        session: &str,
        selector: Option<&str>,
        links: bool,
    ) -> Result<String, String> {
        limited(
            BROWSER_ACTION_SECS,
            self.extract_now(session, selector, links),
        )
        .await
    }

    async fn extract_now(
        &self,
        session: &str,
        selector: Option<&str>,
        links: bool,
    ) -> Result<String, String> {
        let page = self.existing(session).await?;
        let target = selector.map_or_else(
            || "document.body".to_owned(),
            |s| format!("document.querySelector({})", js_string(s)),
        );
        let script = format!(
            "(() => {{ const el = {target}; if (!el) return '\u{0}missing'; \
             let out = el.innerText || ''; \
             if ({links}) {{ const seen = new Set(); const items = []; \
               for (const a of el.querySelectorAll('a[href]')) {{ \
                 const t = (a.innerText || a.getAttribute('aria-label') || '').trim().replace(/\\s+/g, ' '); \
                 if (!t || seen.has(a.href)) continue; seen.add(a.href); items.push('- ' + t.slice(0, 80) + ' → ' + a.href); \
                 if (items.length >= 150) break; }} \
               out += '\\n\\nLinks:\\n' + items.join('\\n'); }} \
             return out; }})()"
        );
        let text = Self::eval_string(&page, &script).await?;
        if text == "\u{0}missing" {
            return Err(format!(
                "nothing on the page matches {}",
                selector.unwrap_or("")
            ));
        }
        Ok(text)
    }

    /// Click an element found by CSS selector or by its visible text.
    pub async fn click(
        &self,
        session: &str,
        selector: Option<&str>,
        text: Option<&str>,
    ) -> Result<(String, String), String> {
        limited(BROWSER_ACTION_SECS, async {
            let page = self.existing(session).await?;
            let css = mark_target(&page, selector, text, Target::Clickable).await?;
            let element = page
                .find_element(css)
                .await
                .map_err(|e| format!("couldn't find it: {e}"))?;
            let _ = element.scroll_into_view().await;
            element
                .click()
                .await
                .map_err(|e| format!("couldn't click it: {e}"))?;
            settle().await;
            Ok(where_now(&page).await)
        })
        .await
    }

    /// Type into a field found by selector or by its label, placeholder or name.
    pub async fn fill(
        &self,
        session: &str,
        selector: Option<&str>,
        field: Option<&str>,
        value: &str,
        submit: bool,
    ) -> Result<(String, String), String> {
        limited(
            BROWSER_ACTION_SECS,
            self.fill_now(session, selector, field, value, submit),
        )
        .await
    }

    async fn fill_now(
        &self,
        session: &str,
        selector: Option<&str>,
        field: Option<&str>,
        value: &str,
        submit: bool,
    ) -> Result<(String, String), String> {
        let page = self.existing(session).await?;
        let css = mark_target(&page, selector, field, Target::Field).await?;
        let element = page
            .find_element(css.clone())
            .await
            .map_err(|e| format!("couldn't find the field: {e}"))?;
        let _ = element.scroll_into_view().await;
        element
            .click()
            .await
            .map_err(|e| format!("couldn't select the field: {e}"))?;
        // Replace what's there.
        let _ = page
            .evaluate(format!(
                "(() => {{ const el = document.querySelector({}); if (el && 'value' in el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles: true}})); }} }})()",
                js_string(&css)
            ))
            .await;
        element
            .type_str(value)
            .await
            .map_err(|e| format!("couldn't type: {e}"))?;
        if submit {
            element
                .press_key("Enter")
                .await
                .map_err(|e| format!("couldn't press Enter: {e}"))?;
            settle().await;
        }
        Ok(where_now(&page).await)
    }

    /// Go back a page.
    pub async fn back(&self, session: &str) -> Result<(String, String), String> {
        limited(BROWSER_ACTION_SECS, async {
            let page = self.existing(session).await?;
            page.evaluate("history.back()")
                .await
                .map_err(|e| format!("couldn't go back: {e}"))?;
            settle().await;
            Ok(where_now(&page).await)
        })
        .await
    }

    /// A PNG of the visible page (or the whole page).
    pub async fn screenshot(&self, session: &str, full_page: bool) -> Result<Vec<u8>, String> {
        limited(BROWSER_ACTION_SECS, async {
            let page = self.existing(session).await?;
            page.screenshot(
                ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .full_page(full_page)
                    .build(),
            )
            .await
            .map_err(|e| format!("couldn't take a screenshot: {e}"))
        })
        .await
    }

    /// Print an HTML page to PDF in a fresh tab, then close the tab. With `max_pages`, the whole
    /// page is shrunk evenly (down to `PDF_MIN_SCALE`) until it fits. Returns the PDF, its page
    /// count and the scale used.
    pub async fn print_pdf(
        &self,
        data_dir: Option<&Path>,
        html: &str,
        setup: &PrintSetup,
        max_pages: Option<usize>,
    ) -> Result<(Vec<u8>, usize, f64), String> {
        use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
        let profile = self.ensure(data_dir, None).await?;
        let page = {
            let running = self.running.lock().await;
            let r = running
                .get(&profile)
                .ok_or_else(|| "the browser isn't running".to_owned())?;
            r.browser
                .new_page("about:blank")
                .await
                .map_err(|e| format!("couldn't open a tab: {e}"))?
        };
        let result = limited(BROWSER_PRINT_SECS, async {
            page.set_content(html)
                .await
                .map_err(|e| format!("couldn't lay out the document: {e}"))?;
            // Embedded fonts must be ready before printing, or the PDF uses a fallback.
            let _ = page.evaluate("document.fonts.ready.then(() => true)").await;
            let mut scale = 1.0_f64;
            loop {
                let mut params = PrintToPdfParams::builder()
                    .print_background(true)
                    .paper_width(setup.width_in)
                    .paper_height(setup.height_in)
                    .margin_top(setup.margin_top_in)
                    .margin_bottom(setup.margin_bottom_in)
                    .margin_left(setup.margin_side_in)
                    .margin_right(setup.margin_side_in)
                    .scale(scale)
                    .prefer_css_page_size(setup.css_page_size)
                    .display_header_footer(setup.header.is_some());
                if let Some((header, footer)) = &setup.header {
                    params = params
                        .header_template(header.clone())
                        .footer_template(footer.clone());
                }
                let pdf = page
                    .pdf(params.build())
                    .await
                    .map_err(|e| format!("couldn't print the PDF: {e}"))?;
                let pages = lopdf::Document::load_mem(&pdf)
                    .map(|d| d.get_pages().len())
                    .unwrap_or(0);
                let next = scale - PDF_SCALE_STEP;
                match max_pages {
                    Some(limit) if pages > limit && next >= PDF_MIN_SCALE - 1e-9 => scale = next,
                    _ => return Ok((pdf, pages, scale)),
                }
            }
        })
        .await;
        let _ = page.close().await;
        result
    }
    /// A picture of `html` laid out for print, pages one under the other, `width_in` wide and at
    /// most `max_height_in` tall, scaled by `scale`: so a PDF can be checked by eye.
    pub async fn print_preview(
        &self,
        data_dir: Option<&Path>,
        html: &str,
        width_in: f64,
        max_height_in: f64,
        scale: f64,
    ) -> Result<Vec<u8>, String> {
        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
        use chromiumoxide::cdp::browser_protocol::page::Viewport;
        use chromiumoxide::page::MediaTypeParams;
        let profile = self.ensure(data_dir, None).await?;
        let page = {
            let running = self.running.lock().await;
            let r = running
                .get(&profile)
                .ok_or_else(|| "the browser isn't running".to_owned())?;
            r.browser
                .new_page("about:blank")
                .await
                .map_err(|e| format!("couldn't open a tab: {e}"))?
        };
        let width = (width_in * CSS_PX_PER_IN).round();
        let result = limited(BROWSER_PRINT_SECS, async {
            page.execute(SetDeviceMetricsOverrideParams::new(
                width as i64,
                (max_height_in * CSS_PX_PER_IN) as i64,
                1.0,
                false,
            ))
            .await
            .map_err(|e| format!("couldn't size the preview: {e}"))?;
            page.emulate_media_type(MediaTypeParams::Print)
                .await
                .map_err(|e| format!("couldn't lay out the preview: {e}"))?;
            page.set_content(html)
                .await
                .map_err(|e| format!("couldn't lay out the preview: {e}"))?;
            let _ = page.evaluate("document.fonts.ready.then(() => true)").await;
            let tall = page
                .evaluate("document.documentElement.scrollHeight")
                .await
                .ok()
                .and_then(|v| v.into_value::<f64>().ok())
                .unwrap_or(max_height_in * CSS_PX_PER_IN);
            page.screenshot(
                ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Jpeg)
                    .quality(PDF_PREVIEW_QUALITY)
                    .capture_beyond_viewport(true)
                    .clip(Viewport {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height: tall.min(max_height_in * CSS_PX_PER_IN).max(1.0),
                        scale,
                    })
                    .build(),
            )
            .await
            .map_err(|e| format!("couldn't take the preview: {e}"))
        })
        .await;
        let _ = page.close().await;
        result
    }
    /// Stop every browser (the engine is stopping).
    pub async fn close_all(&self) {
        let all: Vec<Running> = self.running.lock().await.drain().map(|(_, r)| r).collect();
        for mut old in all {
            let _ = old.browser.close().await;
            let _ = old.browser.wait().await;
            old.handler.abort();
        }
    }

    /// Close the session's tab; its browser stops when no tabs are left in it.
    pub async fn close(&self, session: &str) -> Result<(), String> {
        let mut running = self.running.lock().await;
        let Some(profile) = running
            .iter()
            .find(|(_, r)| r.pages.contains_key(session))
            .map(|(p, _)| p.clone())
        else {
            return Ok(());
        };
        let Some(r) = running.get_mut(&profile) else {
            return Ok(());
        };
        if let Some(page) = r.pages.remove(session) {
            let _ = page.close().await;
        }
        if r.pages.is_empty()
            && let Some(mut old) = running.remove(&profile)
        {
            let _ = old.browser.close().await;
            old.handler.abort();
        }
        Ok(())
    }
}

/// End browser processes using Thrall's profile (left by an engine that crashed or was killed).
fn close_leftovers(profile: &Path) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let marker = profile.to_string_lossy().to_lowercase();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    for process in sys.processes().values() {
        let ours = process
            .cmd()
            .iter()
            .any(|arg| arg.to_string_lossy().to_lowercase().contains(&marker));
        if ours {
            process.kill();
        }
    }
}

/// Wait for a page to settle after something that may load (navigation, a click).
async fn settle() {
    tokio::time::sleep(Duration::from_millis(BROWSER_SETTLE_MS)).await;
}

/// The page's title and address now, once it has finished loading (polled: a page restored from
/// the back/forward cache fires no navigation event to wait for).
async fn where_now(page: &Page) -> (String, String) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(BROWSER_TIMEOUT_SECS);
    while tokio::time::Instant::now() < deadline {
        let ready = page
            .evaluate("document.readyState")
            .await
            .ok()
            .and_then(|r| r.into_value::<String>().ok());
        if ready.as_deref() == Some("complete") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(BROWSER_POLL_MS)).await;
    }
    let title = page.get_title().await.ok().flatten().unwrap_or_default();
    let url = page.url().await.ok().flatten().unwrap_or_default();
    (title, url)
}
/// A JavaScript string literal.
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

#[derive(Clone, Copy)]
enum Target {
    Clickable,
    Field,
}

/// Find the element (by selector, or by visible text / label) in the page, tag it, and return a
/// selector for it.
async fn mark_target(
    page: &Page,
    selector: Option<&str>,
    text: Option<&str>,
    kind: Target,
) -> Result<String, String> {
    if let Some(s) = selector.filter(|s| !s.trim().is_empty()) {
        return Ok(s.to_owned());
    }
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Err("say which element: a CSS selector, or its visible text".to_owned());
    };
    let finder = match kind {
        Target::Clickable => {
            "const pool = [...document.querySelectorAll('a,button,input[type=submit],input[type=button],[role=button],[role=link],[role=tab],[role=menuitem],summary,label')]; \
             const textOf = el => (el.innerText || el.value || el.getAttribute('aria-label') || el.title || '').trim().replace(/\\s+/g, ' ').toLowerCase();"
        }
        Target::Field => {
            "const pool = [...document.querySelectorAll('input:not([type=hidden]),textarea,select,[contenteditable=true]')]; \
             const textOf = el => { const ids = el.id ? [...document.querySelectorAll(`label[for=\"${CSS.escape(el.id)}\"]`)].map(l => l.innerText) : []; \
               const wrap = el.closest('label'); if (wrap) ids.push(wrap.innerText); \
               return [el.getAttribute('aria-label'), el.placeholder, el.name, el.id, ...ids].filter(Boolean).join(' | ').trim().replace(/\\s+/g, ' ').toLowerCase(); };"
        }
    };
    let script = format!(
        "(() => {{ {finder} \
           const want = {}.toLowerCase(); \
           const visible = el => {{ const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; }}; \
           const exact = pool.filter(el => visible(el) && textOf(el) === want); \
           const partial = pool.filter(el => visible(el) && textOf(el).includes(want)); \
           const el = exact[0] || partial[0]; \
           if (!el) return ''; \
           document.querySelectorAll('[data-thrall-target]').forEach(e => e.removeAttribute('data-thrall-target')); \
           el.setAttribute('data-thrall-target', '1'); return 'ok'; }})()",
        js_string(text)
    );
    let found = Browsers::eval_string(page, &script).await?;
    if found != "ok" {
        return Err(format!("nothing on the page shows \"{text}\""));
    }
    Ok("[data-thrall-target=\"1\"]".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_open() {
        assert_eq!(web_url("example.com").unwrap(), "https://example.com");
        assert_eq!(web_url("http://x.test/a").unwrap(), "http://x.test/a");
        assert!(web_url("file:///C:/secret.txt").is_err());
        assert!(web_url("javascript:alert(1)").is_err());
        assert!(web_url("  ").is_err());
    }

    #[tokio::test]
    async fn an_action_that_never_answers_fails_at_the_limit() {
        let stuck = limited(1, std::future::pending::<Result<(), String>>()).await;
        assert_eq!(
            stuck.unwrap_err(),
            "the page stopped responding after 1 seconds"
        );
        assert_eq!(limited(1, async { Ok(5) }).await, Ok(5));
    }

    #[test]
    fn pop_up_notes_say_what_was_answered() {
        assert!(dialog_note(&DialogType::Confirm, "Delete?").contains("answered No"));
        assert!(dialog_note(&DialogType::Alert, "Hi").contains("closed it"));
        let long = "x".repeat(BROWSER_DIALOG_MAX_CHARS * 2);
        assert!(dialog_note(&DialogType::Prompt, &long).len() < BROWSER_DIALOG_MAX_CHARS + 100);
    }

    #[test]
    fn js_strings_are_quoted() {
        assert_eq!(js_string("a\"b"), "\"a\\\"b\"");
    }
}
