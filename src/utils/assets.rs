use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use flate2::read::GzDecoder;
use tar::Archive;
use tracing::{info, warn};

const TWEMOJI_ARCHIVE_URL: &str =
    "https://github.com/jdecked/twemoji/archive/refs/heads/main.tar.gz";
const EMOJI_DIR: &str = "static/emoji";
const READY_MARKER: &str = "static/emoji/.ready";

pub fn spawn_emoji_sync() {
    if Path::new(READY_MARKER).exists() {
        return;
    }

    tokio::spawn(async {
        if let Err(err) = download_twemoji().await {
            warn!(error = %err, "Failed to auto-download Twemoji SVGs");
        }
    });
}

async fn download_twemoji() -> Result<(), String> {
    info!("Twemoji assets missing, downloading SVGs into {EMOJI_DIR}...");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(TWEMOJI_ARCHIVE_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    let count = tokio::task::spawn_blocking(move || extract_svgs(&bytes))
        .await
        .map_err(|e| e.to_string())??;

    info!(count, "Twemoji SVGs extracted successfully");
    Ok(())
}

fn extract_svgs(tar_gz: &[u8]) -> Result<usize, String> {
    let dest_dir = Path::new(EMOJI_DIR);
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let gz = GzDecoder::new(Cursor::new(tar_gz));
    let mut archive = Archive::new(gz);
    let entries = archive.entries().map_err(|e| e.to_string())?;

    let mut count = 0usize;

    for entry_res in entries {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }

        let target = {
            let path = entry.path().map_err(|e| e.to_string())?;
            let is_svg = path.extension().and_then(|e| e.to_str()) == Some("svg")
                && path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    == Some("svg")
                && path
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    == Some("assets");

            if !is_svg {
                continue;
            }

            let Some(file_name) = path.file_name() else {
                continue;
            };
            dest_dir.join(file_name)
        };

        entry.unpack(&target).map_err(|e| e.to_string())?;
        count += 1;
    }

    fs::write(READY_MARKER, b"ok").map_err(|e| e.to_string())?;
    Ok(count)
}
