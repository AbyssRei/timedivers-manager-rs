pub mod steamdb;

mod browser;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, anyhow};

use crate::manifest::{ManifestMap, merge_manifests, save_manifests};

pub use browser::{BrowserInfo, detect_browsers};

/// Events streamed from the scraper worker to the log window.
#[derive(Debug)]
pub enum ScraperEvent {
    Log(String),
    /// The browser is connected; the user should log in to SteamDB now.
    ReadyForLogin,
    Finished(anyhow::Result<PathBuf>),
}

/// Handle to a running scrape: receive `events`, send `()` on Continue.
pub struct ScraperSession {
    pub events: async_channel::Receiver<ScraperEvent>,
    pub cont: async_channel::Sender<()>,
}

fn send_log(events: &async_channel::Sender<ScraperEvent>, msg: impl Into<String>) {
    // Every log entry is one line, like the `print` calls of the original.
    let mut line: String = msg.into();
    line.push('\n');
    let _ = events.try_send(ScraperEvent::Log(line));
}

/// Finds a free TCP port by binding an ephemeral listener (as the original).
fn find_free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_for_cdp(port: u16, retries: u32) -> bool {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .expect("build http client");
    for _ in 0..retries {
        if client.get(&url).send().await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Polls for a CSS selector to appear (stand-in for Playwright's
/// `wait_for_selector` with a 60 second timeout). `false` = timed out.
async fn wait_for_selector(page: &chromiumoxide::Page, selector: &str, seconds: u64) -> bool {
    for _ in 0..seconds {
        if page.find_element(selector).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    page.find_element(selector).await.is_ok()
}

/// Starts the scrape in a dedicated thread with its own tokio runtime and
/// returns channels for events and the Continue signal.
pub fn start_scraper(
    app_id: String,
    depots: Vec<String>,
    browser: BrowserInfo,
    base_dir: PathBuf,
) -> anyhow::Result<ScraperSession> {
    let (events_tx, events_rx) = async_channel::unbounded();
    let (cont_tx, cont_rx) = async_channel::bounded::<()>(1);

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let result = runtime.block_on(run(
            app_id,
            depots,
            browser,
            base_dir,
            events_tx.clone(),
            cont_rx,
        ));
        let _ = events_tx.try_send(ScraperEvent::Finished(result));
    });

    Ok(ScraperSession {
        events: events_rx,
        cont: cont_tx,
    })
}

async fn run(
    app_id: String,
    depots: Vec<String>,
    browser: BrowserInfo,
    base_dir: PathBuf,
    events: async_channel::Sender<ScraperEvent>,
    cont: async_channel::Receiver<()>,
) -> anyhow::Result<PathBuf> {
    use futures::StreamExt as _;

    let browser_data_dir = base_dir.join("browser_data");
    let port = find_free_port()?;
    send_log(
        &events,
        format!("Launched {}, waiting for CDP...", browser.name),
    );
    let mut child =
        browser::launch_browser(&browser, &browser_data_dir, port).context("launch browser")?;

    if !wait_for_cdp(port, 60).await {
        let _ = child.kill();
        return Err(anyhow!("CDP did not become available on port {port}"));
    }

    let connect_result = chromiumoxide::Browser::connect(format!("http://127.0.0.1:{port}")).await;
    let (browser_conn, mut handler) = match connect_result {
        Ok(pair) => pair,
        Err(err) => {
            // Fallback: relaunch the same profile under chromiumoxide control,
            // like the original's launch_persistent_context fallback.
            send_log(
                &events,
                format!("CDP connection failed ({err}), falling back to direct launch..."),
            );
            let _ = child.kill();
            let config = chromiumoxide::BrowserConfig::builder()
                .chrome_executable(&browser.exe)
                .user_data_dir(browser_data_dir.join(&browser.slug))
                .with_head()
                .args(browser::stealth_flags())
                .build()
                .map_err(|e| anyhow!("browser config: {e}"))?;
            chromiumoxide::Browser::launch(config)
                .await
                .context("fallback launch")?
        }
    };
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });

    // Chrome registers its initial tab asynchronously; poll briefly before
    // falling back to creating one, otherwise a stray about:blank tab is
    // left behind next to the login tab.
    let mut page = None;
    for _ in 0..20 {
        if let Some(p) = browser_conn.pages().await?.into_iter().next() {
            page = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let page = match page {
        Some(page) => page,
        None => browser_conn
            .new_page("https://steamdb.info/login/")
            .await
            .context("open login page")?,
    };
    send_log(&events, format!("Connected to {} via CDP", browser.name));
    let _ = events.try_send(ScraperEvent::ReadyForLogin);

    send_log(
        &events,
        format!(
            "{} is ready. Please log in to SteamDB, then click Continue.",
            browser.name
        ),
    );
    let _ = cont.recv().await;
    send_log(&events, "Starting scraping...");

    let mut manifests: ManifestMap = Default::default();
    for depot in &depots {
        send_log(&events, format!("Scraping depot {depot}..."));
        page.goto(format!("https://steamdb.info/depot/{depot}/manifests/"))
            .await
            .with_context(|| format!("open depot {depot} manifests page"))?;
        if !wait_for_selector(&page, "table.table tbody tr", 60).await {
            send_log(
                &events,
                format!("Table not found for depot {depot}. Make sure the page loaded correctly."),
            );
        }
        let html = page.content().await.context("read page content")?;
        let parsed = steamdb::parse_manifests_table(&html, depot);
        for (branch_name, dates) in &parsed.branches {
            let dates_map = manifests.entry(branch_name.clone()).or_default();
            for (date, manifest_id) in dates {
                dates_map
                    .entry(date.clone())
                    .or_default()
                    .insert(depot.clone(), manifest_id.clone());
            }
        }
        // Depot sizes from the same table: Steam reports the download size,
        // while the on-disk content is decompressed and larger.
        if !parsed.sizes.is_empty() {
            let sizes = crate::manifest::load_manifest_sizes(&base_dir, &app_id);
            let mut merged = sizes;
            merged.extend(parsed.sizes.clone());
            let _ = crate::manifest::save_manifest_sizes(&base_dir, &app_id, &merged);
        }
        send_log(&events, "Waiting 5 seconds...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    send_log(&events, "Scraping patch titles...");
    page.goto(format!("https://steamdb.info/app/{app_id}/patchnotes/"))
        .await
        .context("open patch notes page")?;
    let mut patch_titles = Vec::new();
    if wait_for_selector(&page, "#js-builds tr", 60).await {
        let html = page.content().await.context("read patch notes content")?;
        patch_titles = steamdb::parse_patch_titles(&html);
    } else {
        send_log(
            &events,
            "Patch notes table not found. Make sure the page loaded correctly.",
        );
    }

    steamdb::apply_patch_titles(&mut manifests, &patch_titles);
    // Release the browser connection, then stop the process we spawned.
    drop(page);
    drop(browser_conn);
    driver.abort();
    let _ = child.kill();

    for branch_data in manifests.values_mut() {
        merge_manifests(branch_data, &depots);
    }

    let path = save_manifests(&base_dir, &app_id, &manifests)?;
    send_log(&events, format!("\nUpdated {}.\n", path.display()));
    Ok(path)
}
