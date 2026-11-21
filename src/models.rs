use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Mirror of the C# `Config` class. All field names and semantics match the
/// existing production Config.json so existing files can be reused as-is.
/// JSON keys are PascalCase (as written by the C# app / Newtonsoft.Json).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Config {
    pub aria_json_rpc_url: String,
    pub aria_json_rpc_token: String,
    pub run_immediately: bool,
    pub rss_url: Vec<String>,
    pub download_path_prefix: Option<String>,
    pub download_path_delimiter: String,
    pub latest_successful_run_dt: Option<String>,
    pub search_all: bool,
    /// Added for Docker: maps Windows download roots to container paths.
    /// E.g. { "D:\\[HttpSharing]": "/data/downloads" }
    #[serde(rename = "PathMapping")]
    pub path_mapping: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            aria_json_rpc_url: String::new(),
            aria_json_rpc_token: String::new(),
            run_immediately: false,
            rss_url: Vec::new(),
            download_path_prefix: None,
            download_path_delimiter: "|||".to_string(),
            latest_successful_run_dt: None,
            search_all: false,
            path_mapping: HashMap::new(),
        }
    }
}

impl Config {
    /// The raw download path prefix to use, falling back to the default
    /// `D:\[HttpSharing]\AnimeDL` when no prefix is configured.
    pub fn effective_prefix(&self) -> &str {
        match &self.download_path_prefix {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => "D:\\[HttpSharing]\\AnimeDL",
        }
    }
}

/// One entry in VideoRegex.json. In production this file is an array of
/// plain strings like `"regex|||subfolder"`. We deserialize leniently to
/// support both the string format and an object format.
#[derive(Debug, Clone)]
pub struct VideoRegex {
    pub file_regex: String,
    pub download_path: Option<String>,
}

impl VideoRegex {
    pub fn from_string(s: &str, delimiter: &str) -> Self {
        if let Some(idx) = s.find(delimiter) {
            let regex = s[..idx].to_string();
            let path = s[idx + delimiter.len()..].to_string();
            Self {
                file_regex: regex,
                download_path: if path.trim().is_empty() {
                    None
                } else {
                    Some(path)
                },
            }
        } else {
            Self {
                file_regex: s.to_string(),
                download_path: None,
            }
        }
    }
}

/// A parsed entry of VideoRegex.json. Supports string entries
/// (`"regex|||path"`) and object entries (`{"FileRegex":"..","DownloadPath":".."}`).
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VideoRegexEntry {
    String(String),
    Object(VideoRegexObject),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct VideoRegexObject {
    pub file_regex: String,
    #[serde(default)]
    pub download_path: Option<String>,
}

/// One record in DownloadedFile.json.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DownloadedFile {
    pub file_name: String,
    pub downloaded_dt: String,
    pub source: String,
    pub download_path: Option<String>,
}

/// One record in FailedDownloadedFile.json.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FailedDownloadedFile {
    pub file_name: String,
    pub magnet_link: String,
    pub attempt_dt: String,
    pub download_path: Option<String>,
    pub source: String,
}

/// JSON-RPC request body for aria2.addUri.
#[derive(Debug, Serialize)]
pub struct AriaAddUri<'a> {
    pub jsonrpc: &'a str,
    pub method: &'a str,
    pub id: &'a str,
    pub params: Vec<serde_json::Value>,
}
