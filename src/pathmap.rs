use std::collections::HashMap;

/// Translate a Windows download path (as stored in Config.json) into a
/// container path suitable for aria2's `dir` option.
///
/// The JSON files keep Windows paths because the external web app (cont5) and
/// SMB both operate on the real Windows filesystem. When running inside a
/// Docker container on the same Windows host, those paths are bind-mounted at
/// a container path (e.g. `D:\[HttpSharing]` -> `/data/downloads`). This
/// module rewrites the prefix before the path is sent to aria2.
pub fn translate_download_path(
    raw: &str,
    path_mapping: &HashMap<String, String>,
    default_container_dir: &str,
) -> String {
    let normalized = raw.replace('\\', "/");

    // Longest-prefix match first so more specific mappings win.
    let mut candidates: Vec<(&String, &String)> =
        path_mapping.iter().collect();
    candidates.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));

    for (win, container) in candidates {
        let win_norm = win.replace('\\', "/");
        if matches_prefix(&normalized, &win_norm) {
            let rest = &normalized[win_norm.len()..];
            let rest = rest.trim_start_matches('/');
            let joined = format!("{}/{}", container.trim_end_matches('/'), rest);
            let joined = joined.trim_end_matches('/').to_string();
            return if joined.is_empty() {
                container.trim_end_matches('/').to_string()
            } else {
                joined
            };
        }
    }

    // No mapping matched. If the path looks like the default download root,
    // fall back to the default container dir. Otherwise return the normalized
    // path as-is (best effort; the user is responsible for making it valid in
    // the container).
    if normalized.trim() == "D:/[HttpSharing]/AnimeDL"
        || normalized.trim() == "D:/[HttpSharing]/AnimeDL/"
    {
        default_container_dir.to_string()
    } else {
        normalized
    }
}

/// Case-insensitive check that `path` starts with `prefix` at a component
/// boundary (so `D:\Http` does not match `D:\[HttpSharing]`).
fn matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    if path.len() < prefix.len() {
        return false;
    }
    let path_upper = path.to_uppercase();
    let prefix_upper = prefix.to_uppercase();
    if !path_upper.starts_with(&prefix_upper) {
        return false;
    }
    if path.len() == prefix.len() {
        return true;
    }
    // The character after the prefix must be a path separator (/) to avoid
    // partial-component matches.
    matches!(path.as_bytes().get(prefix.len()), Some(b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn mapping() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("D:\\[HttpSharing]".to_string(), "/data/downloads".to_string());
        m
    }

    #[test]
    fn translates_prefixed_path() {
        let out = translate_download_path(
            "D:\\[HttpSharing]\\Anime\\Foo",
            &mapping(),
            "/data/downloads/AnimeDL",
        );
        assert_eq!(out, "/data/downloads/Anime/Foo");
    }

    #[test]
    fn default_root_maps_to_default_dir() {
        let out = translate_download_path(
            "D:\\[HttpSharing]\\AnimeDL",
            &mapping(),
            "/data/downloads/AnimeDL",
        );
        assert_eq!(out, "/data/downloads/AnimeDL");
    }

    #[test]
    fn empty_prefix_default() {
        let out = translate_download_path(
            "D:\\[HttpSharing]\\AnimeDL",
            &mapping(),
            "/data/downloads/AnimeDL",
        );
        assert_eq!(out, "/data/downloads/AnimeDL");
    }

    #[test]
    fn no_mapping_passthrough() {
        let empty = HashMap::new();
        let out = translate_download_path(
            "D:\\[HttpSharing]\\Anime",
            &empty,
            "/data/downloads/AnimeDL",
        );
        assert_eq!(out, "D:/[HttpSharing]/Anime");
    }
}
