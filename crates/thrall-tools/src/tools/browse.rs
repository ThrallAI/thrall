//! `browser`: use a real browser — open pages, click, fill in forms, read, take screenshots.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::RESULT_MAX_CHARS;
use crate::saved;
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

/// Drive the browser.
pub struct BrowserTool;

#[derive(Deserialize)]
struct Args {
    action: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    submit: bool,
    #[serde(default)]
    links: bool,
    #[serde(default)]
    full_page: bool,
    #[serde(default)]
    visible: Option<bool>,
}

const ACTIONS: &[&str] = &[
    "open",
    "click",
    "fill",
    "extract",
    "screenshot",
    "back",
    "close",
];

fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_owned()
}

impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }
    fn description(&self) -> &'static str {
        "A real browser (Chrome/Edge) for pages that need JavaScript, clicking or forms; use \
         web_fetch for simply reading a page. action: open (url; visible: true shows the window), \
         click (selector, or the element's visible text), fill (a field by selector, or by its \
         label/placeholder in text; value; submit: true presses Enter), extract (the page's text, \
         or one element's by selector; links: true lists links), screenshot (shown to the user; \
         full_page optional), back, close. Each session has its own tab; it keeps logins between \
         uses. Never type passwords or payment details. Never open sign-in pages here (OAuth \
         links, Google or account logins: sites refuse this browser): give the person the link \
         to open in their own browser. If a tool can't do what's asked, say so; don't work \
         around it by logging in to the website."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ACTIONS },
                "url": { "type": "string", "description": "For open" },
                "selector": { "type": "string", "description": "CSS selector, for click, fill or extract" },
                "text": { "type": "string", "description": "For click: the element's visible text. For fill: the field's label or placeholder" },
                "value": { "type": "string", "description": "For fill: what to type" },
                "submit": { "type": "boolean", "description": "For fill: press Enter after typing" },
                "links": { "type": "boolean", "description": "For extract: also list the links" },
                "full_page": { "type": "boolean", "description": "For screenshot: the whole page, not just what's visible" },
                "visible": { "type": "boolean", "description": "For open: show the browser window (default: hidden)" }
            },
            "required": ["action"]
        })
    }
    fn title(&self, args: &Value) -> String {
        match arg_str(args, "action") {
            "open" => format!("Opening {}", arg_str(args, "url")),
            "click" => {
                let what = args
                    .get("text")
                    .or_else(|| args.get("selector"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                format!("Clicking {what}")
            }
            "fill" => {
                let what = args
                    .get("text")
                    .or_else(|| args.get("selector"))
                    .and_then(Value::as_str)
                    .unwrap_or("a field");
                format!("Filling in {what}")
            }
            "extract" => "Reading the page".to_owned(),
            "screenshot" => "Taking a screenshot".to_owned(),
            "back" => "Going back".to_owned(),
            _ => "Closing the browser tab".to_owned(),
        }
    }
    fn precheck(&self, _ctx: &ToolContext, args: &Value) -> Option<String> {
        let action = arg_str(args, "action");
        if !ACTIONS.contains(&action) {
            return Some(format!("action must be one of: {}", ACTIONS.join(", ")));
        }
        if action == "open"
            && let Err(e) = crate::browser::web_url(arg_str(args, "url"))
        {
            return Some(e);
        }
        if action == "fill" && args.get("value").and_then(Value::as_str).is_none() {
            return Some("fill needs value (what to type)".to_owned());
        }
        None
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        // Clicking and typing act on websites (they can submit, buy, post): those ask. Opening,
        // reading and screenshots only look.
        match arg_str(args, "action") {
            "click" => Effect::Run {
                summary: format!(
                    "Click \"{}\" in the browser",
                    args.get("text")
                        .or_else(|| args.get("selector"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                ),
                risk: Risk::Low,
                grant: Some("browser".to_owned()),
            },
            "fill" => Effect::Run {
                summary: format!(
                    "Type \"{}\" into {}{}",
                    arg_str(args, "value"),
                    args.get("text")
                        .or_else(|| args.get("selector"))
                        .and_then(Value::as_str)
                        .unwrap_or("a field"),
                    if args.get("submit").and_then(Value::as_bool) == Some(true) {
                        " and submit"
                    } else {
                        ""
                    }
                ),
                risk: Risk::Low,
                grant: Some("browser".to_owned()),
            },
            _ => Effect::Read,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: Args = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let browsers = &ctx.browser;
            let session = ctx.session_id.as_str();
            let at = |(title, url): (String, String)| {
                let title = if title.is_empty() {
                    "(no title)".to_owned()
                } else {
                    title
                };
                (format!("Now at: {title}\n{url}"), host(&url))
            };
            let mut out = match a.action.as_str() {
                "open" => {
                    let url = a.url.clone().unwrap_or_default();
                    match browsers
                        .open(ctx.data_dir.as_deref(), session, &url, a.visible)
                        .await
                    {
                        Ok(place) => {
                            let (content, where_) = at(place);
                            // A first look at the page, so the next step can be chosen.
                            let preview = browsers
                                .extract(session, None, false)
                                .await
                                .map(|t| t.chars().take(3000).collect::<String>())
                                .unwrap_or_default();
                            ToolOutput::ok(
                                format!(
                                    "{content}\n\nThe page begins:\n{preview}\n\n(Use extract for all of it.)"
                                ),
                                format!("Opened {where_}"),
                            )
                        }
                        Err(e) => ToolOutput::error(e),
                    }
                }
                "click" => match browsers
                    .click(session, a.selector.as_deref(), a.text.as_deref())
                    .await
                {
                    Ok(place) => {
                        let (content, where_) = at(place);
                        ToolOutput::ok(format!("Clicked. {content}"), format!("Clicked · {where_}"))
                    }
                    Err(e) => ToolOutput::error(e),
                },
                "fill" => {
                    let value = a.value.clone().unwrap_or_default();
                    match browsers
                        .fill(
                            session,
                            a.selector.as_deref(),
                            a.text.as_deref(),
                            &value,
                            a.submit,
                        )
                        .await
                    {
                        Ok(place) => {
                            let (content, where_) = at(place);
                            let done = if a.submit {
                                "Typed it and submitted"
                            } else {
                                "Typed it"
                            };
                            ToolOutput::ok(
                                format!("{done}. {content}"),
                                format!("{done} · {where_}"),
                            )
                        }
                        Err(e) => ToolOutput::error(e),
                    }
                }
                "extract" => match browsers
                    .extract(session, a.selector.as_deref(), a.links)
                    .await
                {
                    Ok(text) => {
                        let chars = text.chars().count();
                        ToolOutput::ok(cap(text, RESULT_MAX_CHARS), format!("{chars} characters"))
                    }
                    Err(e) => ToolOutput::error(e),
                },
                "screenshot" => match browsers.screenshot(session, a.full_page).await {
                    Ok(png) => match saved::save(ctx, "screenshot.png", &png) {
                        Ok((path, file)) => {
                            let mut out = ToolOutput::ok(
                                format!(
                                    "Took a screenshot, shown to the user and saved at {}. Use view_image on it to see what's on the page.",
                                    path.display()
                                ),
                                "Screenshot".to_owned(),
                            );
                            out.files = vec![file];
                            out
                        }
                        Err(e) => ToolOutput::error(e),
                    },
                    Err(e) => ToolOutput::error(e),
                },
                "back" => match browsers.back(session).await {
                    Ok(place) => {
                        let (content, where_) = at(place);
                        ToolOutput::ok(content, format!("Back · {where_}"))
                    }
                    Err(e) => ToolOutput::error(e),
                },
                _ => match browsers.close(session).await {
                    Ok(()) => ToolOutput::ok("Closed this session's browser tab.", "Closed"),
                    Err(e) => ToolOutput::error(e),
                },
            };
            // Pop-ups the page showed were answered for Thrall (asks always get No): say so.
            let dialogs = browsers.take_dialogs(session);
            if !dialogs.is_empty() {
                out.content.push_str("\n\n");
                out.content.push_str(&dialogs.join("\n"));
                out.summary.push_str(" · answered a pop-up");
            }
            out
        })
    }
}
