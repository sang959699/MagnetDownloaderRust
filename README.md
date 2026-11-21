# MagnetDownloader (Rust)

A Rust rewrite of the C# [MagnetDownloader](https://github.com/sang959699/MagnetDownloader).
It periodically scans configured RSS feeds, matches item titles against regex
rules, and pushes the matching magnet links to **aria2** over its JSON-RPC API.

Behaviour is a faithful port of the original:

- Runs every 15 minutes, or immediately when `RunImmediately` is set (then cleared).
- `SearchAll`: when true, scans every feed item; otherwise only items newer than
  `LatestSuccessfulRunDt`.
- Skips titles already present in `DownloadedFile.json` (dedup).
- Records failed `aria2.addUri` calls in `FailedDownloadedFile.json` and retries
  them on every run, removing them once they succeed.
- Saves `LatestSuccessfulRunDt` only when at least one RSS URL was reached.

Differences from the C# version (no behaviour change, just correctness):

- JSON files are written atomically (temp file + rename), so the external web
  app never observes a half-written file.
- The same production JSON files are read, so no config migration is needed.

## JSON files (Config/)

All four files keep the exact production formats. `Config.json` gains one extra
field, `PathMapping`, used only to translate Windows paths into container paths
for aria2 when running inside Docker.

| File | Schema |
|------|--------|
| `Config.json` | `AriaJsonRpcUrl`, `AriaJsonRpcToken`, `RunImmediately`, `RssUrl[]`, `DownloadPathPrefix`, `DownloadPathDelimiter`, `LatestSuccessfulRunDt`, `SearchAll`, `PathMapping` |
| `VideoRegex.json` | array of strings `"<regex>|||<subpath>"` (subpath optional; the delimiter comes from `DownloadPathDelimiter`) |
| `DownloadedFile.json` | array of `{FileName, DownloadedDt, Source, DownloadPath}` |
| `FailedDownloadedFile.json` | array of `{FileName, MagnetLink, AttemptDt, DownloadPath, Source}` |

### `PathMapping` and download paths

Download paths in the JSON are **Windows paths** (e.g. `D:\[HttpSharing]\Anime`)
because the external web app (cont5) and SMB operate on the real Windows
filesystem. When the tool and aria2 run in containers on that same Windows host,
the download root is bind-mounted at a container path. `PathMapping` tells the
tool how to translate a Windows prefix into the equivalent container path before
sending `dir` to aria2:

```json
"PathMapping": { "D:\\[HttpSharing]": "/data/downloads" }
```

- `DownloadPathPrefix` empty → default root `D:\[HttpSharing]\AnimeDL`.
- Otherwise target = `DownloadPathPrefix + <subpath>`.
- Each rule's `dir` option is the translated path.

## Building

```bash
cargo build --release
# binary at target/release/magnetdownloader
```

## Docker

The Dockerfile produces a fully static `scratch`-based image (no libc, no shell).
`docker compose up -d --build` brings up the tool plus `aria2` (p3terx/aria2-pro).

See `docker-compose.yml` for the bind mounts. Two volumes are used on Windows
(Docker Desktop translates `D:\...` automatically):

- `D:\MagnetDownloader\Config` → `/app/config` (the four JSON files)
- `D:\[HttpSharing]` → `/data/downloads` (the download root)

Set `PathMapping` accordingly in `Config.json` and align `aria_json_rpc_url` /
`aria_json_rpc_token` with the aria2 service.

## Running locally

Point `CONFIG_DIR` at the folder holding the JSON files:

```bash
CONFIG_DIR=/path/to/Config ./target/release/magnetdownloader
```

## Tests

```bash
cargo test
```

Covers the Windows→container path translation (default root, prefixed paths,
subpaths, no-mapping passthrough).