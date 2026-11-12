//! 读 Google Takeout 里的 YouTube 数据（协议 §3.8、§3.9），按结构找文件，不按名字：
//! 文件夹和文件的名字随 Google 账号的语言改（`playlists/Watch later-videos.csv`、别的语言里完全不同）。
//!
//! - **播放列表**：任何一个 `.csv`，只要数据行的第一列是 11 位的视频 id，第二列（有的话）是时间。
//!   表头的字不看。依据：播放列表的 CSV 是 `Video ID` 加 `Playlist Video Creation Timestamp` 两列，
//!   WatchNow（ItsMaciek/WatchNow，没有许可证，只当作格式的参考、没有用它的代码）读的就是这种文件。
//! - **观看历史**：任何一个 `.json`，开头像「我的活动」的数组（有 `header`、`time`）；条目的字段照
//!   purarue/google_takeout_parser（MIT）的 `_parse_json_activity`：`title`、`titleUrl`、`time`、`subtitles`、`details`。
//!   HTML 形式的历史认得出来，回 `unsupported-input` 加请用户选 JSON 的提示。
//!
//! 视频链接和 `contentId` 照 usuyo-fetch/ytdlp：`https://www.youtube.com/watch?v=<11 位 id>`。

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use usuyo_plugin_kit::protocol::{
    Access, Author, Evidence, Identity, ListAction, ListItem, ListPage, PlatformId, Rfc3339,
    SourceInfo, SourcesOutput,
};

use crate::csv;
use crate::error::{Error, HTML_HISTORY, Lang, NO_HISTORY, NO_PLAYLISTS, NOT_AN_EXPORT, TOO_LARGE};
use crate::input::{self, Archive, PLAIN};
use crate::limits::Limits;

/// 一页最多多少条。
pub const PAGE: usize = 500;
const PLAYLISTS: &str = "youtube:playlists";
const PLAYLIST: &str = "youtube:playlist:";
const HISTORY: &str = "youtube:watch-history";

/// 认形状时读开头多少字节。
const SNIFF_CSV: usize = 4096;
const SNIFF_JSON: usize = 8192;
const SNIFF_HTML: usize = 64 << 10;

/// 11 位的视频 id：`[A-Za-z0-9_-]`。
fn video_id(text: &str) -> bool {
    text.len() == 11
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// RFC 3339 带偏移的时间；Takeout 有时写 `2023-01-15 10:20:30 UTC`，也认。
fn time(text: &str) -> Option<Rfc3339> {
    let text = text.trim();
    Rfc3339::try_from(text.to_owned()).ok().or_else(|| {
        let utc = text.strip_suffix(" UTC")?;
        Rfc3339::try_from(format!("{}Z", utc.replacen(' ', "T", 1))).ok()
    })
}

/// 观看历史链接里的视频 id：`youtube.com/watch?v=…`（含 `music.`、`m.`）。
fn watch_id(href: &str) -> Option<String> {
    let url = url::Url::parse(href).ok()?;
    if !matches!(
        url.host_str()?,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com"
    ) || url.path() != "/watch"
    {
        return None;
    }
    let id = url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned();
    video_id(&id).then_some(id)
}

struct Video {
    id: String,
    at: Option<Rfc3339>,
    title: Option<String>,
    author: Option<Author>,
}

fn item(video: Video, action: ListAction) -> ListItem {
    let url = format!("https://www.youtube.com/watch?v={}", video.id);
    ListItem {
        url: Some(url.clone()),
        reference: None,
        identity: PlatformId::try_from("youtube")
            .ok()
            .map(|platform| Identity::Platform {
                platform,
                content_id: video.id,
                url: Some(url),
            }),
        title: video.title,
        author: video.author,
        published_at: None,
        action: Some(action),
        acted_at: video.at,
    }
}

/// 一个视频只留一条，留最新的时间；新的在前，没有时间的在最后；时间相同按 id 排，翻页时顺序才不变。
fn merged(videos: Vec<Video>) -> Vec<Video> {
    let mut by_id: BTreeMap<String, Video> = BTreeMap::new();
    for v in videos {
        match by_id.get_mut(&v.id) {
            None => {
                by_id.insert(v.id.clone(), v);
            }
            Some(old) => {
                if v.at.as_ref().map(Rfc3339::timestamp) > old.at.as_ref().map(Rfc3339::timestamp) {
                    old.at = v.at;
                }
                if old.title.is_none() {
                    old.title = v.title;
                }
                if old.author.is_none() {
                    old.author = v.author;
                }
            }
        }
    }
    let mut all: Vec<Video> = by_id.into_values().collect();
    all.sort_by(|a, b| {
        (Reverse(a.at.as_ref().map(Rfc3339::timestamp)), &a.id)
            .cmp(&(Reverse(b.at.as_ref().map(Rfc3339::timestamp)), &b.id))
    });
    all
}

// ---- 播放列表 ----

struct Playlist {
    /// 条目名（zip 里的路径），来源 `youtube:playlist:<条目名>` 用它找回这个列表。
    entry: String,
    title: String,
    videos: Vec<Video>,
}

/// 数据行都像播放列表：第一列是视频 id，第二列（有的话）空或是时间。`rows[0]` 是表头。
fn playlist_rows(rows: &[Vec<String>]) -> bool {
    rows.len() > 1
        && rows[1..].iter().all(|r| {
            video_id(&r[0])
                && r.get(1)
                    .is_none_or(|t| t.trim().is_empty() || time(t).is_some())
        })
}

fn csv_head(archive: &mut Archive, name: &str) -> Result<Vec<Vec<String>>, Error> {
    let head = archive.head(name, SNIFF_CSV)?;
    // 开头可能在一行中间截断：只看到最后一个换行为止
    let cut = if head.len() == SNIFF_CSV {
        head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
    } else {
        head.len()
    };
    Ok(csv::parse(&head[..cut]).unwrap_or_default())
}

/// 文件名去掉扩展名；文件夹不要。
fn stem(entry: &str) -> &str {
    let file = entry.rsplit('/').next().unwrap_or(entry);
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// 播放列表的名字：文件名是 `<列表名>-<后缀>`，后缀（英文是 `videos`）随语言变。
/// 去掉 `-videos`；有两个以上列表时，所有文件名共有的最后一段也去掉。
fn titles(entries: &[String]) -> Vec<String> {
    let stems: Vec<&str> = entries.iter().map(|e| stem(e)).collect();
    let common = (stems.len() >= 2)
        .then(|| stems[0].rsplit_once('-').map(|(_, tail)| tail))
        .flatten()
        .filter(|tail| !tail.is_empty() && stems.iter().all(|s| s.ends_with(&format!("-{tail}"))));
    stems
        .iter()
        .map(|s| {
            let stripped = common
                .and_then(|t| s.strip_suffix(&format!("-{t}")))
                .or_else(|| {
                    let at = s.len().checked_sub("-videos".len())?;
                    (s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case("-videos"))
                        .then(|| &s[..at])
                })
                .unwrap_or(s);
            if stripped.is_empty() {
                PLAIN.to_owned()
            } else {
                stripped.to_owned()
            }
        })
        .collect()
}

fn playlists(archive: &mut Archive, limits: &Limits) -> Result<Vec<Playlist>, Error> {
    let names = archive.names("csv");
    let mut found = Vec::new();
    for name in names {
        if playlist_rows(&csv_head(archive, &name)?) {
            found.push(name);
        }
    }
    if found.len() > limits.csv_files {
        return Err(Error::unsupported(
            PLAIN,
            "(playlist files)",
            format!(
                "{} playlist files, over the {} limit",
                found.len(),
                limits.csv_files
            ),
            TOO_LARGE,
        ));
    }
    let titles = titles(&found);
    let mut out = Vec::new();
    for (entry, title) in found.into_iter().zip(titles) {
        let bytes = archive.read(&entry, limits.csv_bytes)?.unwrap_or_default();
        let rows = csv::parse(&bytes)
            .map_err(|e| Error::unsupported(&entry, "(csv)", e, NOT_AN_EXPORT))?;
        let mut videos = Vec::new();
        for (n, row) in rows.iter().enumerate().skip(1) {
            if !video_id(&row[0]) {
                return Err(Error::changed(
                    &entry,
                    &format!("row {} column 1", n + 1),
                    "not an 11 character video id",
                ));
            }
            videos.push(Video {
                id: row[0].clone(),
                at: row.get(1).and_then(|t| time(t)),
                title: None,
                author: None,
            });
        }
        out.push(Playlist {
            entry,
            title,
            videos,
        });
    }
    Ok(out)
}

// ---- 观看历史 ----

#[derive(Deserialize)]
struct Activity {
    #[serde(rename = "titleUrl")]
    title_url: Option<String>,
    title: Option<String>,
    time: Option<String>,
    subtitles: Option<Vec<Subtitle>>,
    details: Option<Vec<Detail>>,
}

#[derive(Deserialize)]
struct Subtitle {
    name: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct Detail {
    name: Option<String>,
}

/// 观看历史在这份文件里是什么样的。
struct History {
    json: Vec<String>,
    html: bool,
}

fn find_history(archive: &mut Archive, limits: &Limits) -> Result<History, Error> {
    let mut json = Vec::new();
    for name in archive.names("json") {
        let head = archive.head(&name, SNIFF_JSON)?;
        let text = String::from_utf8_lossy(&head);
        let start = text.trim_start_matches('\u{feff}').trim_start();
        if start.starts_with('[') && text.contains("\"header\"") && text.contains("\"time\"") {
            json.push(name);
        }
    }
    if json.len() > limits.history_files {
        return Err(Error::unsupported(
            PLAIN,
            "(history files)",
            format!(
                "{} history files, over the {} limit",
                json.len(),
                limits.history_files
            ),
            TOO_LARGE,
        ));
    }
    let mut html = false;
    for name in archive.names("html") {
        let head = archive.head(&name, SNIFF_HTML)?;
        if head.windows(20).any(|w| w == b"youtube.com/watch?v=") {
            html = true;
            break;
        }
    }
    Ok(History { json, html })
}

fn history_videos(
    archive: &mut Archive,
    history: &History,
    limits: &Limits,
) -> Result<Vec<Video>, Error> {
    let mut videos = Vec::new();
    for name in &history.json {
        let bytes = archive.read(name, limits.json_bytes)?.unwrap_or_default();
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        let activities: Vec<Activity> = serde_json::from_slice(bytes).map_err(|e| {
            if e.is_data() {
                Error::changed(name, "(array of activities)", e.to_string())
            } else {
                Error::unsupported(
                    name,
                    "(whole file)",
                    format!("not valid JSON: {e}"),
                    NOT_AN_EXPORT,
                )
            }
        })?;
        for a in activities {
            let Some(id) = a.title_url.as_deref().and_then(watch_id) else {
                // 已删除的视频（没有链接）、社区帖子、搜索
                continue;
            };
            // 广告
            if a.details
                .iter()
                .flatten()
                .any(|d| d.name.as_deref() == Some("From Google Ads"))
            {
                continue;
            }
            let title = a
                .title
                .as_deref()
                .map(|t| t.strip_prefix("Watched ").unwrap_or(t).trim())
                .filter(|t| !t.is_empty() && url::Url::parse(t).is_err())
                .map(str::to_owned);
            let author = a
                .subtitles
                .iter()
                .flatten()
                .find_map(|s| {
                    s.name
                        .as_deref()
                        .filter(|n| !n.is_empty())
                        .map(|n| (n, s.url.as_deref()))
                })
                .map(|(name, url)| Author {
                    name: name.to_owned(),
                    platform_id: None,
                    url: url.map(str::to_owned),
                });
            videos.push(Video {
                id,
                at: a.time.as_deref().and_then(time),
                title,
                author,
            });
            if videos.len() > limits.items {
                return Err(Error::unsupported(
                    name,
                    "(entries)",
                    format!("more than {} entries", limits.items),
                    TOO_LARGE,
                ));
            }
        }
    }
    Ok(videos)
}

// ---- 来源 ----

enum Pick {
    /// `file:…`：播放列表；这份文件里没有播放列表、只有观看历史时，列观看历史。
    File,
    Playlists,
    Playlist(String),
    History,
}

fn pick(source: &str) -> Result<Pick, Error> {
    if source.starts_with("file:") {
        Ok(Pick::File)
    } else if source == PLAYLISTS {
        Ok(Pick::Playlists)
    } else if source == HISTORY {
        Ok(Pick::History)
    } else if let Some(entry) = source.strip_prefix(PLAYLIST) {
        Ok(Pick::Playlist(entry.to_owned()))
    } else {
        Err(Error::InvalidArgs(format!("unknown source `{source}`")))
    }
}

fn history_error(archive: &Archive, history: &History) -> Error {
    if history.html {
        Error::unsupported(
            PLAIN,
            "(history)",
            "the watch history is HTML, not JSON",
            HTML_HISTORY,
        )
    } else if matches!(archive, Archive::Lone { .. }) {
        Error::unsupported(
            PLAIN,
            "(whole file)",
            "not a playlist CSV or a watch history JSON",
            NOT_AN_EXPORT,
        )
    } else {
        Error::unsupported(
            PLAIN,
            "(zip entries)",
            "no watch history JSON in the zip",
            NO_HISTORY,
        )
    }
}

fn open(input: Option<&Path>, limits: &Limits) -> Result<Archive, Error> {
    let input = input
        .ok_or_else(|| Error::InvalidArgs("USUYO_INPUT is not set for a file import".into()))?;
    input::open(input, limits)
}

/// 这份文件里用户能选的来源：每个播放列表一个，加一个观看历史。
pub fn sources(input: Option<&Path>, lang: Lang, limits: &Limits) -> Result<SourcesOutput, Error> {
    let history_title = || match lang {
        Lang::Zh => "观看历史".to_owned(),
        Lang::En => "Watch history".to_owned(),
    };
    // 宿主调 `sources` 时不带文件（协议 §3.8）：给两个固定的来源，`list` 才读文件
    let Some(input) = input else {
        let (playlists, history) = match lang {
            Lang::Zh => ("播放列表", history_title()),
            Lang::En => ("Playlists", history_title()),
        };
        return Ok(SourcesOutput {
            sources: vec![
                SourceInfo {
                    source: PLAYLISTS.into(),
                    title: playlists.into(),
                    action: Some(ListAction::Favorite),
                    count: None,
                },
                SourceInfo {
                    source: HISTORY.into(),
                    title: history,
                    action: Some(ListAction::View),
                    count: None,
                },
            ],
        });
    };
    let mut archive = open(Some(input), limits)?;
    let lists = playlists(&mut archive, limits)?;
    let history = find_history(&mut archive, limits)?;
    if lists.is_empty() && history.json.is_empty() && !history.html {
        return Err(match &archive {
            Archive::Lone { .. } => history_error(&archive, &history),
            Archive::Zip { .. } => Error::unsupported(
                PLAIN,
                "(zip entries)",
                "no playlist CSV and no watch history in the zip",
                NO_PLAYLISTS,
            ),
        });
    }
    let mut out: Vec<SourceInfo> = lists
        .iter()
        .map(|p| SourceInfo {
            source: format!("{PLAYLIST}{}", p.entry),
            title: p.title.clone(),
            action: Some(ListAction::Favorite),
            count: Some(p.videos.len() as u64),
        })
        .collect();
    if !history.json.is_empty() || history.html {
        let count = if history.json.is_empty() {
            None
        } else {
            Some(merged(history_videos(&mut archive, &history, limits)?).len() as u64)
        };
        out.push(SourceInfo {
            source: HISTORY.into(),
            title: history_title(),
            action: Some(ListAction::View),
            count,
        });
    }
    Ok(SourcesOutput { sources: out })
}

fn videos_of(
    source: &str,
    input: Option<&Path>,
    limits: &Limits,
) -> Result<(Vec<Video>, ListAction), Error> {
    let pick = pick(source)?;
    let mut archive = open(input, limits)?;
    let from_history = |archive: &mut Archive, history: &History| {
        if history.json.is_empty() {
            return Err(history_error(archive, history));
        }
        Ok((history_videos(archive, history, limits)?, ListAction::View))
    };
    match pick {
        Pick::History => {
            let history = find_history(&mut archive, limits)?;
            from_history(&mut archive, &history)
        }
        Pick::Playlist(entry) => {
            let list = playlists(&mut archive, limits)?
                .into_iter()
                .find(|p| p.entry == entry)
                .ok_or_else(|| Error::InvalidArgs(format!("no playlist `{entry}` in this file")))?;
            Ok((list.videos, ListAction::Favorite))
        }
        Pick::Playlists | Pick::File => {
            let lists = playlists(&mut archive, limits)?;
            if lists.is_empty() {
                let history = find_history(&mut archive, limits)?;
                // 只有观看历史的文件，`file:` 就列观看历史；`youtube:playlists` 没有播放列表就是没有
                if matches!(pick, Pick::File) && !history.json.is_empty() {
                    return from_history(&mut archive, &history);
                }
                return Err(if matches!(archive, Archive::Lone { .. }) {
                    history_error(&archive, &history)
                } else {
                    Error::unsupported(
                        PLAIN,
                        "(zip entries)",
                        "no playlist CSV in the zip",
                        NO_PLAYLISTS,
                    )
                });
            }
            Ok((
                lists.into_iter().flat_map(|p| p.videos).collect(),
                ListAction::Favorite,
            ))
        }
    }
}

/// 列一页。`cursor` 是从第几条开始（插件自己的不透明字符串）。
pub fn list(
    source: &str,
    input: Option<&Path>,
    cursor: Option<&str>,
    limits: &Limits,
) -> Result<ListPage, Error> {
    let start = match cursor {
        None => 0,
        Some(c) => c
            .parse::<usize>()
            .map_err(|_| Error::InvalidArgs(format!("cursor `{c}` is not ours")))?,
    };
    let fetched_at = Rfc3339::from_timestamp(jiff::Timestamp::now());
    let (videos, action) = videos_of(source, input, limits)?;
    let all = merged(videos);
    let total = all.len();
    let end = start.saturating_add(PAGE).min(total);
    let items = all
        .into_iter()
        .enumerate()
        .filter(|(i, _)| (start..end).contains(i))
        .map(|(_, v)| item(v, action))
        .collect::<Vec<_>>();
    let next_cursor = (end < total).then(|| end.to_string());
    Ok(ListPage {
        items,
        next_cursor,
        evidence: Evidence {
            fetched_at,
            access: Access::OfficialExport,
            site: None,
            upstream: None,
            status: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_times() {
        assert!(video_id("dQw4w9WgXcQ"));
        assert!(!video_id("dQw4w9WgXc"));
        assert!(!video_id("dQw4w9WgXc!"));
        assert_eq!(
            time("2023-01-15T10:20:30+00:00").unwrap().as_str(),
            "2023-01-15T10:20:30+00:00"
        );
        assert_eq!(
            time("2023-01-15 10:20:30 UTC").unwrap().as_str(),
            "2023-01-15T10:20:30Z"
        );
        assert!(time("yesterday").is_none());
        assert_eq!(
            watch_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            watch_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=x"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(watch_id("https://www.youtube.com/post/abc"), None);
        assert_eq!(watch_id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
    }

    #[test]
    fn playlist_titles() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            titles(&names(&[
                "T/YouTube and YouTube Music/playlists/Watch later-videos.csv"
            ])),
            ["Watch later"]
        );
        // 别的语言：所有文件共有的后缀去掉
        assert_eq!(
            titles(&names(&["a/稍后观看-视频.csv", "a/咖啡-视频.csv"])),
            ["稍后观看", "咖啡"]
        );
        // 名字里本来就有 `-` 的列表：只去掉共有的最后一段
        assert_eq!(
            titles(&names(&[
                "p/Pour-over tips-videos.csv",
                "p/Liked videos-videos.csv"
            ])),
            ["Pour-over tips", "Liked videos"]
        );
        assert_eq!(titles(&names(&["p/Mixed.csv"])), ["Mixed"]);
    }

    #[test]
    fn playlist_shapes() {
        let rows = |s: &str| csv::parse(s.as_bytes()).unwrap();
        assert!(playlist_rows(&rows(
            "Video ID,Time\ndQw4w9WgXcQ,2023-01-15T10:20:30+00:00\n"
        )));
        assert!(playlist_rows(&rows("视频 ID,时间\ndQw4w9WgXcQ,\n")));
        assert!(playlist_rows(&rows("id\ndQw4w9WgXcQ\n")));
        assert!(!playlist_rows(&rows("Video ID,Time\n")));
        assert!(!playlist_rows(&rows(
            "Channel ID,Url,Title\nUCabc,https://x,y\n"
        )));
        assert!(!playlist_rows(&rows(
            "Video ID,Time\ndQw4w9WgXcQ,not a time\n"
        )));
    }
}
