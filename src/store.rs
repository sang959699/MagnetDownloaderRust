use crate::same_item;
use crate::models::{
    Config, DownloadedFile, FailedDownloadedFile, VideoRegex, VideoRegexEntry,
};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DOWNLOADED_FILE: &str = "DownloadedFile.json";
pub const FAILED_DOWNLOADED_FILE: &str = "FailedDownloadedFile.json";
pub const CONFIG: &str = "Config.json";
pub const VIDEO_REGEX: &str = "VideoRegex.json";

/// Shared access to the Config/ JSON files. Reads are done fresh on every
/// access so edits made by the external web app (cont5) are picked up. Writes
/// go through a temp file + atomic rename so the external app never observes a
/// half-written file.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Ensure the config directory exists and every expected file exists,
    /// mirroring the C# JsonInit().
    pub fn init(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        for name in [
            DOWNLOADED_FILE,
            FAILED_DOWNLOADED_FILE,
            CONFIG,
            VIDEO_REGEX,
        ] {
            let p = self.path(name);
            if !p.exists() {
                let default = match name {
                    CONFIG => serde_json::to_string_pretty(&Config::default())?,
                    _ => "[]".to_string(),
                };
                fs::write(&p, default)?;
            }
        }
        Ok(())
    }

    // ---- Config ----

    pub fn load_config(&self) -> Config {
        let json = fs::read_to_string(self.path(CONFIG)).unwrap_or_default();
        serde_json::from_str(&json).unwrap_or_default()
    }

    pub fn save_config(&self, config: &Config) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(config)?;
        atomic_write(&self.path(CONFIG), json.as_bytes())
    }

    // ---- DownloadedFile ----

    pub fn load_downloaded(&self) -> Vec<DownloadedFile> {
        read_json_array(&self.path(DOWNLOADED_FILE))
    }

    pub fn save_downloaded(&self, list: &[DownloadedFile]) -> std::io::Result<()> {
        atomic_write_json(&self.path(DOWNLOADED_FILE), list)
    }

    /// Insert a new downloaded record at the front (matching C# behaviour).
    pub fn push_downloaded(&self, rec: DownloadedFile) -> std::io::Result<()> {
        let mut list = self.load_downloaded();
        list.insert(0, rec);
        self.save_downloaded(&list)
    }

    // ---- FailedDownloadedFile ----

    pub fn load_failed(&self) -> Vec<FailedDownloadedFile> {
        read_json_array(&self.path(FAILED_DOWNLOADED_FILE))
    }

    pub fn save_failed(&self, list: &[FailedDownloadedFile]) -> std::io::Result<()> {
        atomic_write_json(&self.path(FAILED_DOWNLOADED_FILE), list)
    }

    pub fn push_failed(&self, rec: FailedDownloadedFile) -> std::io::Result<()> {
        let mut list = self.load_failed();
        if !list.iter().any(|f| same_item(&f.file_name, &rec.file_name)) {
            list.push(rec);
        }
        self.save_failed(&list)
    }

    pub fn remove_failed(&self, file_name: &str) -> std::io::Result<()> {
        let mut list = self.load_failed();
        let before = list.len();
        list.retain(|f| !same_item(&f.file_name, file_name));
        if list.len() != before {
            self.save_failed(&list)?;
        }
        Ok(())
    }

    // ---- VideoRegex ----

    /// Parse VideoRegex.json, supporting both the string array format and an
    /// object array format. The delimiter comes from Config.
    pub fn load_video_regex(&self, delimiter: &str) -> Vec<VideoRegex> {
        let json = fs::read_to_string(self.path(VIDEO_REGEX)).unwrap_or_default();
        match serde_json::from_str::<Vec<VideoRegexEntry>>(&json) {
            Ok(entries) => entries
                .into_iter()
                .map(|e| match e {
                    VideoRegexEntry::String(s) => VideoRegex::from_string(&s, delimiter),
                    VideoRegexEntry::Object(o) => VideoRegex {
                        file_regex: o.file_regex,
                        download_path: o.download_path,
                    },
                })
                .collect(),
            Err(_) => {
                eprintln!("[warn] could not parse {} as a JSON array", VIDEO_REGEX);
                Vec::new()
            }
        }
    }
}

// ---- helpers ----

fn read_json_array<T>(path: &Path) -> Vec<T>
where
    T: serde::de::DeserializeOwned,
{
    use std::io::ErrorKind;

    match fs::read_to_string(path) {
        // A missing file is the legitimate first-run state (nothing downloaded
        // yet), so treat it as an empty list and let the run proceed.
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("[warn] no {} yet (first run) - starting empty", path.display());
            Vec::new()
        }
        // Any other failure (e.g. the file exists but is unreadable, or it
        // exists but does not parse) is a real problem: proceeding with an
        // empty list would make the tool re-download everything already saved.
        // Fail loudly instead of silently corrupting history.
        Err(e) => panic!(
            "Cannot read {}: {} - refusing to proceed with an empty dedup list",
            path.display(),
            e
        ),
        Ok(json) => match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => panic!(
                "Cannot parse {} ({} bytes): {} - refusing to proceed with an empty dedup list",
                path.display(),
                json.len(),
                e
            ),
        },
    }
}

/// Serialize a value and write it atomically (temp file + rename).
fn atomic_write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    atomic_write(path, json.as_bytes())
}

/// Write bytes to a temp file in the same directory, fsync, then rename over
/// the target. The rename is atomic on the same filesystem, so a concurrent
/// reader (the external web app) only ever sees the old or new content, never
/// a partial write.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "magnetdl_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_is_empty_not_panic() {
        let d = tmpdir();
        let p = d.join("DownloadedFile.json"); // does not exist
        let list: Vec<DownloadedFile> = read_json_array(&p);
        assert!(list.is_empty());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    #[should_panic(expected = "Cannot parse")]
    fn corrupt_file_panics() {
        let d = tmpdir();
        let p = d.join("DownloadedFile.json");
        fs::write(&p, "{ this is not valid json").unwrap();
        let _: Vec<DownloadedFile> = read_json_array(&p);
    }

    #[test]
    fn valid_file_parses() {
        let d = tmpdir();
        let p = d.join("DownloadedFile.json");
        // DownloadFile has no required non-null fields beyond FileName.
        fs::write(&p, r#"[{"FileName":"Show - 01"}]"#).unwrap();
        let list: Vec<DownloadedFile> = read_json_array(&p);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_name, "Show - 01");
        fs::remove_dir_all(&d).ok();
    }
}
