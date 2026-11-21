mod aria2;
mod models;
mod pathmap;
mod store;

use aria2::Aria2Client;
use models::{Config, DownloadedFile, FailedDownloadedFile};
use regex::Regex;
use std::time::Duration;

/// Interval between runs, matching the C# `15` minute schedule.
const RUN_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How long to sleep between checking whether it is time to run.
const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Default container directory used when the configured Windows path maps to
/// nothing. Bind mount the download root here in the container.
const DEFAULT_CONTAINER_DIR: &str = "/data/downloads/AnimeDL";

#[tokio::main]
async fn main() {
    // Config directory can be overridden; default matches the C# layout.
    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "Config".to_string());
    let store = store::Store::new(&config_dir);
    if let Err(e) = store.init() {
        eprintln!("[fatal] cannot initialise config dir '{}': {}", config_dir, e);
        std::process::exit(1);
    }

    process_loop(&store).await;
}

async fn process_loop(store: &store::Store) {
    let mut play_time = chrono_now();
    loop {
        let config = store.load_config();
        let run_immediately = config.run_immediately;
        let now = chrono_now();

        if !run_immediately && now < play_time {
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        }

        log(&format!("runImmediately: {}", run_immediately));
        log("DownloadMagnet Start");
        match download_magnet(store, &config).await {
            Ok(()) => log("DownloadMagnet End"),
            Err(e) => log(&format!("DownloadMagnet error: {}", e)),
        }

        play_time = now + RUN_INTERVAL;
        if run_immediately {
            // Clear the immediate-run flag so the next cycle schedules normally.
            let mut c = store.load_config();
            c.run_immediately = false;
            let _ = store.save_config(&c);
        }
        log(&format!("Next Run {}", format_dt(play_time)));

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// One scan: walk all RSS URLs, match titles against the regex rules, push
/// magnets to aria2, then retry the failed list. Mirrors C# DownloadMagnet().
async fn download_magnet(store: &store::Store, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let url_list = &config.rss_url;
    let mut failed_url_list: Vec<String> = Vec::new();
    let latest_dt = config.latest_successful_run_dt.clone();
    let search_all = config.search_all;
    log(&format!("searchAll: {}", search_all));

    let client = Aria2Client::new(
        config.aria_json_rpc_url.clone(),
        config.aria_json_rpc_token.clone(),
    );

    for url in url_list {
        let feed = match fetch_feed(url).await {
            Ok(f) => f,
            Err(e) => {
                log(&format!("Error accessing {}: {}", url, e));
                failed_url_list.push(url.clone());
                continue;
            }
        };

        // Determine which items are new: everything unless search_all.
        let items: Vec<feed_rs::model::Entry> = if search_all {
            feed.entries
        } else {
            feed.entries
                .into_iter()
                .filter(|e| {
                    // Compare as absolute instants, not RFC3339 strings, so the
                    // feed's published offset (often +00:00) and our UTC+8
                    // `latest_dt` compare correctly regardless of zone.
                    let published = e.published.map(|p| p.timestamp_millis());
                    let last = latest_dt.as_ref().and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
                    });
                    last.map_or(true, |last| published.map_or(false, |dt| dt >= last))
                })
                .collect()
        };
        log(&format!("Feed Items Count: {}", items.len()));

        let downloaded = store.load_downloaded();
        let video_regex = store.load_video_regex(&config.download_path_delimiter);

        for item in &items {
            let title_text = item
                .title
                .as_ref()
                .map(|t| html_unescape(&t.content))
                .unwrap_or_default();
            if title_text.trim().is_empty() {
                continue;
            }

            for rule in &video_regex {
                let re = match Regex::new(&rule.file_regex) {
                    Ok(r) => r,
                    Err(e) => {
                        log(&format!("[warn] bad regex '{}': {}", rule.file_regex, e));
                        continue;
                    }
                };

                // Trim on compare so a stale trailing/leading space in either
                // file can't cause a spurious re-download of an already-saved item.
                if re.is_match(&title_text)
                    && !downloaded
                        .iter()
                        .any(|d| d.file_name.trim() == title_text.trim())
                {
                    let magnet = get_magnet_link(item);
                    let magnet = match magnet {
                        Some(m) => m,
                        None => {
                            log(&format!("No magnet link for: {}", title_text));
                            break;
                        }
                    };

                    let dir = compute_dir(config, rule.download_path.as_deref());
                    let ok = client.add_uri(&magnet, dir.as_deref()).await;

                    match ok {
                        Ok(_) => {
                            let _ = store.remove_failed(&title_text);
                            let rec = DownloadedFile {
                                file_name: title_text.clone(),
                                downloaded_dt: chrono_now().to_rfc3339(),
                                source: url.clone(),
                                download_path: dir.clone(),
                            };
                            let _ = store.push_downloaded(rec);
                            log(&format!("Downloading: {}", title_text));
                        }
                        Err(e) => {
                            let rec = FailedDownloadedFile {
                                file_name: title_text.clone(),
                                magnet_link: magnet.clone(),
                                attempt_dt: chrono_now().to_rfc3339(),
                                download_path: dir.clone(),
                                source: url.clone(),
                            };
                            let _ = store.push_failed(rec);
                            log(&format!("AddDownload Failed: {} ({})", title_text, e));
                        }
                    }
                    break;
                }
            }
        }
    }

    // Retry the failed download list.
    let failed_list = store.load_failed();
    log(&format!(
        "Scan Failed Download File List, Count: {}",
        failed_list.len()
    ));
    let downloaded_once = store.load_downloaded();
    for item in &failed_list {
        // Never re-try a download that already succeeded: the same item can
        // linger in the failed list (e.g. after a connectivity blip) even
        // though it is already recorded in DownloadedFile.json.
        if downloaded_once
            .iter()
            .any(|d| d.file_name.trim() == item.file_name.trim())
        {
            continue;
        }
        let dir = translate_dir(&item.download_path, config);
        let ok = client.add_uri(&item.magnet_link, dir.as_deref()).await;
        match ok {
            Ok(_) => {
                let _ = store.remove_failed(&item.file_name);
                let rec = DownloadedFile {
                    file_name: item.file_name.clone(),
                    downloaded_dt: chrono_now().to_rfc3339(),
                    source: item.source.clone(),
                    download_path: item.download_path.clone(),
                };
                let _ = store.push_downloaded(rec);
                log(&format!("Downloading: {}", item.file_name));
            }
            Err(e) => {
                log(&format!("AddDownload Failed: {} ({})", item.file_name, e));
            }
        }
    }

    // Only record a successful run timestamp if at least one RSS URL worked,
    // matching the C# `if (failedUrlList.Count != urlList.Length)`.
    if failed_url_list.len() != url_list.len() {
        let mut c = store.load_config();
        c.latest_successful_run_dt = Some(chrono_now().to_rfc3339());
        let _ = store.save_config(&c);
        log(&format!("Latest Download DT Saved: {}", format_dt(chrono_now())));
    }

    Ok(())
}

/// Build the aria2 `dir` option for a matched rule.
fn compute_dir(config: &Config, rule_path: Option<&str>) -> Option<String> {
    let raw = match (config.effective_prefix(), rule_path) {
        (prefix, Some(sub)) if !sub.is_empty() => {
            let prefix = prefix.trim_end_matches(['\\', '/']);
            let sub = sub.trim_start_matches(['\\', '/']);
            format!("{}\\{}", prefix, sub)
        }
        (prefix, _) => prefix.to_string(),
    };
    translate_dir(&Some(raw), config)
}

/// Translate a stored Windows path into a container path for aria2.
fn translate_dir(stored: &Option<String>, config: &Config) -> Option<String> {
    match stored {
        Some(p) if !p.trim().is_empty() => Some(pathmap::translate_download_path(
            p,
            &config.path_mapping,
            DEFAULT_CONTAINER_DIR,
        )),
        _ => None,
    }
}

/// Find the magnet (application/x-bittorrent) link in an RSS entry.
///
/// RSS feeds typically provide the torrent as an `<enclosure>`, which feed-rs
/// v2 parses into `entry.media[].content[]`, or as a `<link>` which lands in
/// `entry.links`. We search both.
fn get_magnet_link(entry: &feed_rs::model::Entry) -> Option<String> {
    // 1. Media enclosures (RSS <enclosure>).
    for mo in &entry.media {
        for mc in &mo.content {
            let is_torrent = mc
                .content_type
                .as_ref()
                .map(|t| t.to_string().to_lowercase() == "application/x-bittorrent")
                .unwrap_or(false);
            if is_torrent {
                if let Some(u) = &mc.url {
                    return Some(u.to_string());
                }
            }
        }
    }
    // 2. <link rel="enclosure"> entries.
    entry.links.iter().find_map(|l| {
        let is_enclosure = l.rel.as_deref().unwrap_or("").eq_ignore_ascii_case("enclosure")
            || l.rel.as_deref().unwrap_or("") == "related";
        let is_torrent = l
            .media_type
            .as_deref()
            .map(|t| t.to_lowercase() == "application/x-bittorrent")
            .unwrap_or(false);
        if is_enclosure && is_torrent {
            Some(l.href.clone())
        } else if is_torrent {
            Some(l.href.clone())
        } else {
            None
        }
    })
}

async fn fetch_feed(url: &str) -> Result<feed_rs::model::Feed, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let body = client.get(url).send().await?.text().await?;
    let feed = feed_rs::parser::parse(body.as_bytes())?;
    Ok(feed)
}

/// Decode common HTML entities in a title (replaces C# HttpUtility.HtmlDecode).
fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            if let Some(len) = read_entity(&chars[i..], &mut out) {
                i += len;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Try to parse an HTML entity starting at `&`. Returns the number of chars
/// consumed if a valid entity was found and appended to `out`.
fn read_entity(chars: &[char], out: &mut String) -> Option<usize> {
    // Numeric: &#123; or &#xAB;
    if chars.get(1) == Some(&'#') {
        let mut j = 2;
        let hex = chars.get(2) == Some(&'x') || chars.get(2) == Some(&'X');
        if hex {
            j = 3;
        }
        let start = j;
        while j < chars.len() && chars[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j > start && chars.get(j) == Some(&';') {
            let num_str: String = chars[start..j].iter().collect();
            if let Ok(cp) = if hex {
                u32::from_str_radix(&num_str, 16)
            } else {
                num_str.parse()
            } {
                if let Some(ch) = char::from_u32(cp) {
                    out.push(ch);
                    return Some(j + 1);
                }
            }
        }
        return None;
    }

    // Named entities we care about for anime/movie titles.
    const NAMED: &[(&str, char)] = &[
        ("amp;", '&'),
        ("lt;", '<'),
        ("gt;", '>'),
        ("quot;", '"'),
        ("apos;", '\''),
        ("nbsp;", '\u{00a0}'),
        ("hellip;", '\u{2026}'),
    ];
    for (name, ch) in NAMED {
        if chars.len() >= name.len() + 1 {
            let head: String = chars[1..name.len() + 1].iter().collect();
            if head == *name {
                out.push(*ch);
                return Some(name.len() + 1);
            }
        }
    }
    None
}

fn chrono_now() -> chrono::DateTime<chrono::FixedOffset> {
    // Fixed UTC+8 (no DST) so timestamps match the user's other apps. The
    // container runs in UTC, so we can't rely on the system local timezone.
    let offset_utc8 = chrono::FixedOffset::east_opt(8 * 3600)
        .expect("valid fixed offset UTC+8");
    chrono::Utc::now().with_timezone(&offset_utc8)
}

fn format_dt(t: chrono::DateTime<chrono::FixedOffset>) -> String {
    t.format("%d/%m/%Y %H:%M:%S +08:00").to_string()
}

fn log(msg: &str) {
    println!("{} {}", format_dt(chrono_now()), msg);
}
