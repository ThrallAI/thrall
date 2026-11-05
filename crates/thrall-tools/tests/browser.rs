//! The browser against a tiny local website: open, read, fill a form, click, screenshot.
//! Uses the installed Chrome/Edge (headless, its own profile); skipped if there's none.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use thrall_tools::browser::Browsers;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HOME: &str = r#"<!doctype html><html><head><title>Thrall test shop</title></head><body>
<h1>Welcome to the test shop</h1>
<form action="/search" method="get">
  <label for="q">Search products</label> <input id="q" name="q">
  <button type="submit">Go</button>
</form>
<a href="/about">About us</a>
</body></html>"#;

/// Buttons that open pop-ups, as Hoofwright's "Reset data" does (a pop-up nobody answers froze
/// Thrall's turn for 18 hours).
const POPUPS: &str = r#"<!doctype html><html><head><title>Stable</title></head><body>
<p id="horses">3 horses</p>
<button onclick="if (confirm('Erase all horses?')) document.getElementById('horses').innerText = 'erased'">Reset data</button>
<button onclick="alert('Saved!')">Save</button>
</body></html>"#;

/// A page that keeps something in the browser's storage (`?set`) and shows what it finds:
/// people's browsers must not share it.
const REMEMBER: &str = r#"<!doctype html><html><head><title>Remember</title></head><body>
<p id="found"></p>
<script>
if (location.search.includes('set')) localStorage.setItem('who', 'owner');
document.getElementById('found').innerText = 'stored: ' + (localStorage.getItem('who') || 'nothing');
</script>
</body></html>"#;

/// Serve the home page, an about page, and a search page that echoes the query.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = if path.starts_with("/remember") {
                    REMEMBER.to_owned()
                } else if path.starts_with("/popups") {
                    POPUPS.to_owned()
                } else if path.starts_with("/search") {
                    let q = path.split("q=").nth(1).unwrap_or("").replace('+', " ");
                    format!(
                        "<html><head><title>Results</title></head><body><p>You searched for {q}</p></body></html>"
                    )
                } else if path.starts_with("/about") {
                    "<html><head><title>About</title></head><body><p>A shop for testing Thrall.</p></body></html>".to_owned()
                } else {
                    HOME.to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/")
}

fn has_browser() -> bool {
    ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .any(|k| {
            std::env::var(k).is_ok_and(|base| {
                let base = std::path::PathBuf::from(base);
                base.join(r"Google\Chrome\Application\chrome.exe").is_file()
                    || base
                        .join(r"Microsoft\Edge\Application\msedge.exe")
                        .is_file()
            })
        })
}

#[tokio::test]
async fn the_browser_opens_reads_fills_clicks_and_screenshots() {
    if !has_browser() {
        eprintln!("no Chrome or Edge here; skipping");
        return;
    }
    let site = serve().await;
    let data = std::env::temp_dir().join(format!("thrall-browser-{}", std::process::id()));
    let browsers = Browsers::default();

    let (title, url) = browsers.open(Some(&data), "s1", &site, None).await.unwrap();
    assert_eq!(title, "Thrall test shop");
    assert!(url.starts_with(&site));

    let text = browsers.extract("s1", None, true).await.unwrap();
    assert!(text.contains("Welcome to the test shop"), "{text}");
    assert!(text.contains("About us →"), "links are listed: {text}");

    // A field found by its label; Enter submits the form.
    let (title, url) = browsers
        .fill("s1", None, Some("Search products"), "blue mugs", true)
        .await
        .unwrap();
    assert_eq!(title, "Results");
    assert!(url.contains("q=blue+mugs"), "{url}");
    assert!(
        browsers
            .extract("s1", None, false)
            .await
            .unwrap()
            .contains("You searched for blue mugs")
    );

    let (title, _) = browsers.back("s1").await.unwrap();
    assert_eq!(title, "Thrall test shop");

    // A link found by its text.
    let (title, url) = browsers.click("s1", None, Some("About us")).await.unwrap();
    assert_eq!(title, "About");
    assert!(url.ends_with("/about"), "{url}");

    let png = browsers.screenshot("s1", false).await.unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));

    assert!(
        browsers
            .click("s1", None, Some("No such button"))
            .await
            .is_err()
    );
    browsers.close("s1").await.unwrap();
    assert!(
        browsers.extract("s1", None, false).await.is_err(),
        "the tab is gone"
    );
}

#[tokio::test]
async fn pop_ups_are_answered_no_and_reported_instead_of_freezing_the_page() {
    if !has_browser() {
        return;
    }
    let site = serve().await;
    let data = std::env::temp_dir().join(format!("thrall-browser-popups-{}", std::process::id()));
    let browsers = Browsers::default();
    browsers
        .open(Some(&data), "s1", &format!("{site}popups"), None)
        .await
        .unwrap();

    // A pop-up that asks gets No: the click returns at once and nothing is erased.
    let started = std::time::Instant::now();
    let (title, _) = browsers
        .click("s1", None, Some("Reset data"))
        .await
        .unwrap();
    assert_eq!(title, "Stable");
    assert!(started.elapsed().as_secs() < 15, "{:?}", started.elapsed());
    let notes = browsers.take_dialogs("s1");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("Erase all horses?") && notes[0].contains("answered No"),
        "{notes:?}"
    );
    assert!(
        browsers.take_dialogs("s1").is_empty(),
        "each pop-up is told once"
    );
    let text = browsers.extract("s1", None, false).await.unwrap();
    assert!(text.contains("3 horses"), "nothing erased: {text}");

    // A message is closed, and its text reported.
    browsers.click("s1", None, Some("Save")).await.unwrap();
    let notes = browsers.take_dialogs("s1");
    assert!(notes.len() == 1 && notes[0].contains("Saved!"), "{notes:?}");
    // The page still answers afterwards.
    assert!(browsers.extract("s1", None, false).await.is_ok());
    browsers.close_all().await;
}

#[tokio::test]
async fn a_browser_left_behind_is_cleared_before_starting_again() {
    if !has_browser() {
        return;
    }
    let site = serve().await;
    let data = std::env::temp_dir().join(format!("thrall-browser-left-{}", std::process::id()));
    // An engine that dies without closing its browser: the browser process stays behind.
    let first = Browsers::default();
    first.open(Some(&data), "s1", &site, None).await.unwrap();
    std::mem::forget(first);
    // The next start clears it (Chrome won't share a profile) and works.
    let second = Browsers::default();
    let (title, _) = second.open(Some(&data), "s1", &site, None).await.unwrap();
    assert_eq!(title, "Thrall test shop");
    second.close_all().await;
}

#[tokio::test]
async fn markdown_prints_to_a_paged_pdf_with_numbers() {
    if !has_browser() {
        return;
    }
    use thrall_tools::office::layout::Layout;
    use thrall_tools::office::print;
    let data = std::env::temp_dir().join(format!("thrall-browser-pdf-{}", std::process::id()));
    let mut md =
        String::from("# Quarterly Report\n\n| Item | Cost |\n|---|--:|\n| Rent | 1200 |\n\n");
    for i in 0..60 {
        md.push_str(&format!(
            "Paragraph {i} of the report, long enough to fill the page.\n\n"
        ));
    }
    let browsers = Browsers::default();
    let layout = Layout::default();
    let html = print::document(&md, "Quarterly Report", &layout);
    let setup = print::setup(&layout, "Quarterly Report");
    let (pdf, pages, scale) = browsers
        .print_pdf(Some(&data), &html, &setup, None)
        .await
        .unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert_eq!(scale, 1.0);
    assert!(pages >= 2, "a long document spans pages");
    // Asked to fit in fewer pages, everything shrinks evenly.
    let (_, fitted, shrunk) = browsers
        .print_pdf(Some(&data), &html, &setup, Some(pages - 1))
        .await
        .unwrap();
    assert!(fitted < pages && shrunk < 1.0, "{fitted} pages at {shrunk}");
    // Without a header and footer there are no page numbers.
    let plain = Layout {
        header_footer: false,
        ..Layout::default()
    };
    let (bare, _, _) = browsers
        .print_pdf(Some(&data), &html, &print::setup(&plain, "Q"), None)
        .await
        .unwrap();
    browsers.close_all().await;
    let bare_text = lopdf::Document::load_mem(&bare)
        .unwrap()
        .extract_text(&[1])
        .unwrap_or_default();
    assert!(!bare_text.contains("Page 1 of"), "{bare_text}");
    let doc = lopdf::Document::load_mem(&pdf).unwrap();
    let out = data.join("report.pdf");
    std::fs::write(&out, &pdf).unwrap();
    let text = thrall_tools::documents::extract(&out).unwrap().text;
    assert!(text.contains("Rent"), "{text}");
    let first = doc.extract_text(&[1]).unwrap_or_default();
    assert!(
        first.contains("Page") && first.contains("of"),
        "page numbers: {first}"
    );
}

#[tokio::test]
async fn the_make_pdf_tool_lays_out_a_markdown_file() {
    use std::sync::Arc;
    use thrall_store::{NewSession, SqliteStore, Store};
    use thrall_tools::tools::MakePdf;
    use thrall_tools::{Cancel, Tool, ToolContext};
    if !has_browser() {
        return;
    }
    let workspace = std::env::temp_dir().join(format!("thrall-make-pdf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("notes.md"),
        "# Trip Plan\n\n| Day | Where |\n|---|---|\n| 1 | Hobart |\n\n- pack\n- go\n",
    )
    .unwrap();
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u", "U", 1).await.unwrap();
    store
        .create_session(NewSession {
            id: "s".into(),
            user_id: "u".into(),
            workspace: None,
            at: 1,
        })
        .await
        .unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = ToolContext {
        workspace: workspace.clone(),
        store: Arc::new(store),
        user_id: "u".into(),
        session_id: "s".into(),
        http: reqwest::Client::new(),
        output: tx,
        reads: Default::default(),
        cancel: Cancel::never(),
        data_dir: Some(workspace.join(".data")),
        mcp: None,
        memory: None,
        browser: Default::default(),
        disabled_skills: Vec::new(),
        pc: None,
    };
    let args = serde_json::json!({"path": "docs/notes.pdf", "source": "notes.md"});
    assert_eq!(MakePdf.precheck(&ctx, &args), None);
    let bad = serde_json::json!({"path": "notes.txt", "content": "x"});
    assert!(MakePdf.precheck(&ctx, &bad).is_some(), "only .pdf");
    let out = MakePdf.run(&ctx, args).await;
    ctx.browser.close_all().await;
    assert!(out.ok, "{}", out.content);
    assert!(out.content.contains("1 pages"), "{}", out.content);
    let text = thrall_tools::documents::extract(&workspace.join("docs/notes.pdf"))
        .unwrap()
        .text;
    assert!(
        text.contains("Trip Plan") && text.contains("Hobart"),
        "{text}"
    );
}

#[tokio::test]
async fn pictures_print_one_per_page_turned_to_fit_with_a_preview() {
    if !has_browser() {
        return;
    }
    use thrall_tools::office::layout::Layout;
    use thrall_tools::office::{embed, pictures, print};
    let data = std::env::temp_dir().join(format!("thrall-browser-pics-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    // Two upright pictures and one wide one, each its own colour (the same picture twice is
    // stored once).
    for (name, w, h, red) in [
        ("a.png", 60, 80, 200),
        ("b.png", 120, 60, 120),
        ("c.png", 60, 80, 40),
    ] {
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [red, 40, 40, 255]).collect();
        std::fs::write(
            data.join(name),
            thrall_tools::saved::png(w, h, &rgba).unwrap(),
        )
        .unwrap();
    }
    let list: Vec<pictures::Picture> = ["a.png", "b.png", "c.png"]
        .iter()
        .map(|p| pictures::Picture {
            path: (*p).to_owned(),
            caption: None,
        })
        .collect();
    let how = pictures::PictureLayout {
        fill: false,
        turn_to_fit: true,
        cover: None,
        blank_backs: false,
    };
    let layout = Layout {
        margin_mm: 10.0,
        header_footer: false,
        ..Layout::default()
    };
    let page = pictures::pictures_page(&list, &how, &layout, "Book", |p| data.join(p));
    let sources = embed::Sources {
        workspace: &data,
        also: Vec::new(),
    };
    let done = embed::embed(&sources, &page).unwrap();
    assert_eq!(done.pictures, 3);
    let mut setup = print::setup(&layout, "Book");
    setup.css_page_size = true;
    setup.margin_top_in = 0.0;
    setup.margin_bottom_in = 0.0;
    setup.margin_side_in = 0.0;
    let browsers = Browsers::default();
    let (pdf, pages, _) = browsers
        .print_pdf(Some(&data), &done.html, &setup, None)
        .await
        .unwrap();
    let preview = browsers
        .print_preview(Some(&data), &done.html, 11.7, 11.7, 11.7 * 8.0, 0.5)
        .await
        .unwrap();
    browsers.close_all().await;
    assert_eq!(pages, 3, "one picture per page, no spill-over pages");
    assert!(preview.starts_with(&[0xFF, 0xD8]), "the preview is a JPEG");
    // As tall as the three pages, not the eight it was allowed (at 96 px an inch, half size).
    let tall = f64::from(u32::try_from(imagesize::blob_size(&preview).unwrap().height).unwrap());
    assert!(
        tall < 11.7 * 4.0 * 96.0 * 0.5,
        "the preview is {tall} px tall"
    );
    let doc = lopdf::Document::load_mem(&pdf).unwrap();
    let sizes: Vec<(f32, f32)> = doc
        .get_pages()
        .values()
        .map(|id| {
            let page = doc.get_dictionary(*id).unwrap();
            let b = page.get(b"MediaBox").unwrap().as_array().unwrap();
            let n = |o: &lopdf::Object| o.as_float().unwrap_or(0.0);
            (n(&b[2]) - n(&b[0]), n(&b[3]) - n(&b[1]))
        })
        .collect();
    assert!(sizes[0].1 > sizes[0].0, "page 1 upright: {sizes:?}");
    assert!(
        sizes[1].0 > sizes[1].1,
        "page 2 turned for the wide picture: {sizes:?}"
    );
    let images = doc
        .objects
        .values()
        .filter(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict
                    .get(b"Subtype")
                    .is_ok_and(|t| t.as_name().is_ok_and(|n| n == b"Image"))
            })
        })
        .count();
    assert!(images >= 3, "every picture is in the PDF ({images} images)");
}

#[tokio::test]
async fn each_person_has_their_own_browser() {
    if !has_browser() {
        return;
    }
    let site = serve().await;
    let base = std::env::temp_dir().join(format!("thrall-browser-people-{}", std::process::id()));
    let (owner, sam) = (base.join("owner"), base.join("sam"));
    let browsers = Browsers::default();
    browsers
        .open(
            Some(&owner),
            "s_owner",
            &format!("{site}remember?set"),
            None,
        )
        .await
        .unwrap();
    let text = browsers.extract("s_owner", None, false).await.unwrap();
    assert!(text.contains("stored: owner"), "{text}");
    // Another person's browser has its own profile: nothing of the owner's is there.
    browsers
        .open(Some(&sam), "s_sam", &format!("{site}remember"), None)
        .await
        .unwrap();
    let text = browsers.extract("s_sam", None, false).await.unwrap();
    assert!(
        text.contains("stored: nothing"),
        "Sam saw the owner's storage: {text}"
    );
    assert!(owner.join("browser-profile").is_dir() && sam.join("browser-profile").is_dir());
    // Closing one person's tab leaves the other's browser running.
    browsers.close("s_owner").await.unwrap();
    assert!(browsers.extract("s_sam", None, false).await.is_ok());
    browsers.close_all().await;
}
