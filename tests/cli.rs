//! 全部离线：列和来源只读本机的文件，错误在读文件时就答。前半起插件进程（清空环境，代理指向关着的端口），
//! 后半（文件太大）用小的上限直接调库函数，不必真造几百 MB。

mod common;

use usuyo_plugin::{CallLimits, CallOptions, HostConfig, ListRequest, PluginHost, discover};

use std::path::{Path, PathBuf};

use common::{envelope, fixture, lone_file, plugin, takeout_zip, zip_of};
use serde_json::{Value, json};
use usuyo_fetch_youtube::{Error, Lang, Limits, takeout};
use usuyo_plugin::protocol::{
    Access, Account, ListOrder, ListPage, Login, PluginKind, PluginManifest,
};

fn manifest() -> PluginManifest {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("usuyo-plugin.yaml"),
    )
    .unwrap();
    let parsed = PluginManifest::from_yaml(&text).unwrap();
    assert!(
        parsed.unknown_fields.is_empty(),
        "{:?}",
        parsed.unknown_fields
    );
    parsed.value
}

#[test]
fn the_manifest_lists_every_row_of_the_route_table() {
    let m = manifest();
    let PluginKind::Fetch(spec) = &m.kind else {
        panic!("not a fetch plugin");
    };
    let ids: Vec<&str> = spec.methods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["transcript", "takeout", "channel-feed"]);
    let takeout = spec.method("takeout").unwrap();
    assert!(takeout.built);
    assert_eq!(
        (takeout.access, takeout.login, takeout.account),
        (Access::OfficialExport, Login::None, Account::None)
    );
    let list = takeout.capabilities.list.as_ref().unwrap();
    assert_eq!(list.order, ListOrder::NewestFirst);
    assert!(list.reads.is_some() && !list.files.is_empty());
    for (id, access) in [
        ("transcript", Access::PrivateApi),
        ("channel-feed", Access::PublicWeb),
    ] {
        let m = spec.method(id).unwrap();
        assert!(!m.built, "{id}");
        assert_eq!(m.access, access, "{id}");
    }
    assert!(spec.match_patterns.is_empty());
}

fn list(source: &str, input: &Path, cursor: Option<&str>) -> Value {
    let mut args = vec!["list", source, "--method", "takeout"];
    if let Some(c) = cursor {
        args.extend(["--cursor", c]);
    }
    let out = plugin(&args, Some(input));
    assert!(out.status.success(), "{out:?}");
    envelope(&out)
}

fn urls(value: &Value) -> Vec<(String, Option<String>)> {
    value["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["url"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("https://www.youtube.com/watch?v=")
                    .unwrap()
                    .to_owned(),
                i["actedAt"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn at(id: &str, time: Option<&str>) -> (String, Option<String>) {
    (id.to_owned(), time.map(str::to_owned))
}

#[test]
fn file_lists_the_playlists_newest_first_without_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let zip = takeout_zip(dir.path());
    let value = list("file:takeout.zip", &zip, None);
    let mut page = value.clone();
    page.as_object_mut().unwrap().remove("ok");
    let parsed: ListPage = serde_json::from_value(page).unwrap();
    assert_eq!(parsed.next_cursor, None);
    // 两个列表合在一起，同一个视频留较新的时间；没有时间的在最后；元数据 CSV 和订阅 CSV 不算
    assert_eq!(
        urls(&value),
        [
            at("demoVideo02", Some("2026-09-05T09:00:00+00:00")),
            at("demoVideo03", Some("2026-09-02T08:30:00+00:00")),
            at("demoVideo01", Some("2026-09-01T10:00:00+00:00")),
            at("demoVideo05", Some("2026-08-30T00:00:00+00:00")),
            at("demoVideo04", None),
        ]
    );
    let first = &value["items"][0];
    assert_eq!(
        first["identity"],
        json!({"platform": "youtube", "contentId": "demoVideo02", "url": "https://www.youtube.com/watch?v=demoVideo02"})
    );
    assert_eq!(first["action"], "favorite");
    assert!(first.get("ref").is_none() && first.get("title").is_none());
    assert_eq!(value["evidence"]["access"], "official-export");
}

#[test]
fn sources_name_each_playlist_and_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let zip = takeout_zip(dir.path());
    let out = plugin(&["sources", "--method", "takeout"], Some(&zip));
    let v = envelope(&out);
    let s = v["sources"].as_array().unwrap();
    let rows: Vec<(&str, &str, u64)> = s
        .iter()
        .map(|x| {
            (
                x["source"].as_str().unwrap(),
                x["title"].as_str().unwrap(),
                x["count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "youtube:playlist:Takeout/YouTube and YouTube Music/playlists/Liked videos-videos.csv",
                "Liked videos",
                3
            ),
            (
                "youtube:playlist:Takeout/YouTube and YouTube Music/playlists/Watch later-videos.csv",
                "Watch later",
                3
            ),
            ("youtube:watch-history", "观看历史", 3),
        ]
    );
    // 来源能直接拿来列：只列这个列表
    let one = list(rows[1].0, &zip, None);
    assert_eq!(urls(&one).len(), 3);
    assert_eq!(urls(&one)[0].0, "demoVideo02");
    // 不带文件：固定的两个来源，不读东西
    let v = envelope(&plugin(&["sources", "--method", "takeout"], None));
    let fixed: Vec<&str> = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["source"].as_str().unwrap())
        .collect();
    assert_eq!(fixed, ["youtube:playlists", "youtube:watch-history"]);
}

#[test]
fn the_watch_history_is_an_optional_source() {
    let dir = tempfile::tempdir().unwrap();
    let zip = takeout_zip(dir.path());
    let value = list("youtube:watch-history", &zip, None);
    // 已删除的视频、社区帖子、广告、搜索不算；重复看的留最新；链接做标题的没有标题；音乐的也认
    assert_eq!(
        urls(&value),
        [
            at("demoVideo01", Some("2026-09-10T08:00:00.123Z")),
            at("demoVideo06", Some("2026-09-09T08:00:00.000Z")),
            at("demoVideo08", Some("2026-09-08T08:00:00.000Z")),
        ]
    );
    let items = value["items"].as_array().unwrap();
    assert_eq!(items[0]["title"], "Pour-over basics");
    assert_eq!(items[0]["action"], "view");
    assert_eq!(items[0]["author"]["name"], "Coffee Daily");
    assert_eq!(
        items[0]["author"]["url"],
        "https://www.youtube.com/channel/UCdemo000000000000000001"
    );
    assert!(items[2].get("title").is_none());
    // 单个文件：观看历史的 JSON 放在 file: 上就列历史（没有播放列表可列）
    let lone = lone_file(dir.path(), "input", &fixture("watch-history.json"));
    assert_eq!(urls(&list("file:watch-history.json", &lone, None)).len(), 3);
    // 单个播放列表的 CSV
    let csv = lone_file(dir.path(), "input", &fixture("watch-later.csv"));
    assert_eq!(
        urls(&list("file:Watch later-videos.csv", &csv, None)).len(),
        3
    );
}

#[test]
fn names_in_other_languages_are_matched_by_structure() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("zh.zip");
    zip_of(
        &zip,
        &[
            (
                "Takeout/YouTube 和 YouTube Music/播放列表/稍后观看-视频.csv",
                &fixture("watch-later.csv"),
            ),
            (
                "Takeout/YouTube 和 YouTube Music/播放列表/咖啡-视频.csv",
                &fixture("localised-playlist.csv"),
            ),
            (
                "Takeout/YouTube 和 YouTube Music/历史记录/观看记录.json",
                &fixture("watch-history.json"),
            ),
        ],
    );
    let v = envelope(&plugin(&["sources", "--method", "takeout"], Some(&zip)));
    let titles: Vec<&str> = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["咖啡", "稍后观看", "观看历史"]);
    assert_eq!(urls(&list("file:x.zip", &zip, None)).len(), 4);
    assert_eq!(urls(&list("youtube:watch-history", &zip, None)).len(), 3);
}

#[test]
fn a_history_in_html_is_answered_with_a_hint_to_choose_json() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("html.zip");
    zip_of(
        &zip,
        &[
            (
                "Takeout/YouTube and YouTube Music/playlists/Watch later-videos.csv",
                &fixture("watch-later.csv"),
            ),
            (
                "Takeout/YouTube and YouTube Music/history/watch-history.html",
                &fixture("watch-history.html"),
            ),
        ],
    );
    // 播放列表照常；选观看历史才回提示
    assert_eq!(urls(&list("file:x.zip", &zip, None)).len(), 3);
    let out = plugin(
        &["list", "youtube:watch-history", "--method", "takeout"],
        Some(&zip),
    );
    let v = envelope(&out);
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(v["error"]["hint"].as_str().unwrap().contains("JSON"));
    // 单独一个 HTML
    let lone = lone_file(dir.path(), "input", &fixture("watch-history.html"));
    let v = envelope(&plugin(
        &["list", "file:x", "--method", "takeout"],
        Some(&lone),
    ));
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(v["error"]["hint"].as_str().unwrap().contains("JSON"));
}

#[test]
fn list_pages_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = String::from("Video ID,Playlist Video Creation Timestamp\n");
    for n in 0..1200u32 {
        csv.push_str(&format!(
            "v{n:010},2026-01-01T00:{:02}:{:02}+00:00\n",
            (n / 60) % 60,
            n % 60
        ));
    }
    let path = lone_file(dir.path(), "input", csv.as_bytes());
    let mut cursor: Option<String> = None;
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let value = list("file:p.csv", &path, cursor.as_deref());
        pages += 1;
        ids.extend(urls(&value).into_iter().map(|u| u.0));
        match value["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!((pages, ids.len()), (3, 1200));
    assert_eq!(ids[0], "v0000001199");
    assert_eq!(ids[1199], "v0000000000");
}

fn failure(path: &Path, source: &str) -> Value {
    let out = plugin(&["list", source, "--method", "takeout"], Some(path));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    envelope(&out)
}

fn detail(v: &Value) -> &str {
    v["error"]["detail"].as_str().unwrap()
}

#[test]
fn malformed_files_are_answered_with_the_file_and_field() {
    let dir = tempfile::tempdir().unwrap();
    let bad = |bytes: &[u8], source: &str| failure(&lone_file(dir.path(), "input", bytes), source);
    let v = bad(b"just text, not csv", "file:x");
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(detail(&v).contains("file: input"));
    let v = bad(b"{\"a\": 1}", "file:x");
    assert_eq!(v["error"]["code"], "unsupported-input");
    // 认得出是播放列表（头几行对），后面有一行坏了：字段变了，写第几行第几列
    let mut csv = String::from("Video ID,Time\n");
    for _ in 0..2 {
        csv.push_str("demoVideo01,2026-09-01T10:00:00+00:00\n");
    }
    // 头 4096 字节之后才出现坏行
    for n in 0..200 {
        csv.push_str(&format!("v{n:010},2026-09-01T10:00:00+00:00\n"));
    }
    csv.push_str("broken,2026-09-01T10:00:00+00:00\n");
    let v = bad(csv.as_bytes(), "file:x");
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(
        detail(&v).contains("file: input") && detail(&v).contains("column 1"),
        "{}",
        detail(&v)
    );
    // 观看历史：形状不对（数组里是数字）
    let v = bad(
        b"[ {\"header\": \"YouTube\", \"time\": \"x\", \"title\": 5} ]",
        "youtube:watch-history",
    );
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(detail(&v).contains("file: input"));
    // 不是 JSON 的开头像历史
    let v = bad(
        b"[ {\"header\": \"YouTube\", \"time\": ",
        "youtube:watch-history",
    );
    assert_eq!(v["error"]["code"], "unsupported-input");
    // 带引号的字段没关上
    let v = bad(b"Video ID,Time\n\"demoVideo01,2026\n", "file:x");
    assert_eq!(v["error"]["code"], "unsupported-input");
    // zip：里面什么都没有，坏 zip
    let none = dir.path().join("none.zip");
    zip_of(&none, &[("readme.txt", b"hi")]);
    let v = failure(&none, "file:x");
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(v["error"]["hint"].as_str().unwrap().contains("播放列表"));
    let broken = dir.path().join("broken.zip");
    let mut bytes = std::fs::read(takeout_zip(dir.path())).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&broken, bytes).unwrap();
    assert_eq!(
        failure(&broken, "file:x")["error"]["code"],
        "unsupported-input"
    );
    // 有播放列表、没有观看历史
    let lists = dir.path().join("lists.zip");
    zip_of(&lists, &[("p/a-videos.csv", &fixture("watch-later.csv"))]);
    let v = failure(&lists, "youtube:watch-history");
    assert!(v["error"]["hint"].as_str().unwrap().contains("历史记录"));
    // 没有这个播放列表、不认的来源
    let v = failure(&lists, "youtube:playlist:nope.csv");
    assert_eq!(v["error"]["code"], "invalid-args");
    assert_eq!(
        failure(&lists, "youtube:other")["error"]["code"],
        "invalid-args"
    );
}

#[test]
fn a_zip_with_hostile_entry_names_writes_nothing_and_reads_nothing_outside() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let evil = b"Video ID,Time\nevilVideo01,2026-09-01T10:00:00+00:00\n";
    let zip = dir.path().join("hostile.zip");
    zip_of(
        &zip,
        &[
            ("../../evil.csv", evil),
            ("/tmp/usuyo-escape/evil.csv", evil),
            ("..\\evil.csv", evil),
            ("a/../../evil.csv", evil),
            ("ok/watch-later.csv", &fixture("watch-later.csv")),
        ],
    );
    let ids: Vec<String> = urls(&list("file:x.zip", &zip, None))
        .into_iter()
        .map(|u| u.0)
        .collect();
    assert_eq!(ids, ["demoVideo02", "demoVideo03", "demoVideo01"]);
    let only = dir.path().join("only.zip");
    zip_of(&only, &[("../evil.csv", evil)]);
    assert_eq!(
        failure(&only, "file:x")["error"]["code"],
        "unsupported-input"
    );
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["hostile.zip", "only.zip", "work"]);
    assert!(!Path::new("/tmp/usuyo-escape").exists());
}

#[test]
fn health_canonicalize_and_other_calls_send_no_request() {
    let v = envelope(&plugin(&["health"], None));
    assert_eq!(
        (v["ok"].clone(), v["ready"].clone(), v["protocol"].clone()),
        (json!(true), json!(true), json!(1))
    );
    assert_eq!(
        envelope(&plugin(&["health", "--probe"], None))["ready"],
        true
    );
    assert_eq!(
        envelope(&plugin(
            &[
                "canonicalize",
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
            ],
            None
        )),
        json!({"ok": true, "match": false})
    );
    let dir = tempfile::tempdir().unwrap();
    let out_arg = dir.path().to_str().unwrap();
    let v = envelope(&plugin(
        &["fetch", "x", "--out", out_arg, "--method", "takeout"],
        None,
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
    let v = envelope(&plugin(&["list", "file:x", "--method", "takeout"], None));
    assert_eq!(v["error"]["code"], "invalid-args");
    let zip = takeout_zip(dir.path());
    let v = envelope(&plugin(
        &["list", "file:x", "--method", "transcript"],
        Some(&zip),
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
    let v = envelope(&plugin(
        &["list", "file:x", "--method", "takeout", "--cursor", "abc"],
        Some(&zip),
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
    assert_eq!(plugin(&["unfavorite", "x"], None).status.code(), Some(2));
}

fn small() -> Limits {
    Limits {
        zip_entries: 4,
        csv_bytes: 40,
        json_bytes: 200,
        csv_files: 1,
        history_files: 1,
        items: 2,
    }
}

fn unsupported_detail(e: Error) -> String {
    match e {
        Error::Unsupported { detail, .. } => detail,
        other => panic!("{other:?}"),
    }
}

#[test]
fn files_over_the_limits_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let list = |input: &Path, source: &str, l: &Limits| {
        takeout::list(source, Some(input), None, l).unwrap_err()
    };
    // 单个文件太大
    let big = lone_file(dir.path(), "input", &fixture("watch-history.json"));
    assert!(unsupported_detail(list(&big, "file:x", &small())).contains("over the 200 byte limit"));
    // zip 里的 CSV 太大（认形状只读开头，读全文时才发现）
    let zip = dir.path().join("big.zip");
    zip_of(&zip, &[("p/a-videos.csv", &fixture("watch-later.csv"))]);
    let e = list(&zip, "file:x", &small());
    assert!(unsupported_detail(e).contains("file: p/a-videos.csv"));
    // 条目太多
    let many = dir.path().join("many.zip");
    zip_of(
        &many,
        &[
            ("a", b"1"),
            ("b", b"1"),
            ("c", b"1"),
            ("d", b"1"),
            ("e", b"1"),
        ],
    );
    assert!(unsupported_detail(list(&many, "file:x", &small())).contains("5 entries"));
    // 播放列表文件太多
    let lists = dir.path().join("lists.zip");
    let csv = b"Video ID,T\ndemoVideo01,\n";
    zip_of(&lists, &[("a.csv", csv), ("b.csv", csv)]);
    assert!(unsupported_detail(list(&lists, "file:x", &small())).contains("2 playlist files"));
    // 观看历史文件太多
    let hist = dir.path().join("hist.zip");
    let h = b"[{\"header\": \"YouTube\", \"time\": \"2026-09-01T00:00:00Z\"}]";
    zip_of(&hist, &[("a.json", h), ("b.json", h)]);
    assert!(
        unsupported_detail(list(&hist, "youtube:watch-history", &small()))
            .contains("2 history files")
    );
    // 条目太多
    let l = Limits {
        csv_bytes: 1 << 20,
        json_bytes: 1 << 20,
        history_files: 4,
        ..small()
    };
    let json = lone_file(dir.path(), "input", &fixture("watch-history.json"));
    assert!(
        unsupported_detail(list(&json, "youtube:watch-history", &l))
            .contains("more than 2 entries")
    );
    // 来源标题随界面语言
    let s = takeout::sources(None, Lang::En, &Limits::default()).unwrap();
    assert_eq!(s.sources[1].title, "Watch history");
}

// ---- 清单经宿主装上，列（和取）经宿主调一次：输出要过宿主的 schema 和路径检查。
// 文件按宿主的方式交来：副本固定叫 `input.<扩展名>`，路径放在 `USUYO_INPUT`，来源是 `file:<显示名>`。

const HOST_BIN: &str = env!("CARGO_BIN_EXE_usuyo-fetch-youtube");

/// 清单装成宿主认的样子：`<root>/usuyo-fetch/youtube/<version>/`。
fn host() -> (tempfile::TempDir, PluginHost) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plugins/usuyo-fetch/youtube/0.1.0");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("usuyo-plugin.yaml");
    std::fs::copy(manifest, dir.join("usuyo-plugin.yaml")).unwrap();
    std::fs::copy(HOST_BIN, dir.join("usuyo-fetch-youtube")).unwrap();
    let found = discover(&tmp.path().join("plugins")).unwrap();
    assert!(found.rejected.is_empty(), "{:?}", found.rejected);
    let host = PluginHost::new(
        HostConfig {
            work_dir: tmp.path().join("work"),
            cache_dir: tmp.path().join("cache"),
            locale: "zh-CN".into(),
            limits: CallLimits::default(),
        },
        found.fetch,
    );
    (tmp, host)
}

#[tokio::test]
async fn the_manifest_loads_and_a_file_is_listed_through_the_host() {
    let (tmp, host) = host();
    let plugin = &host.plugins()[0];
    assert_eq!(plugin.id().as_str(), "usuyo-fetch/youtube");
    let method = plugin.spec.method("takeout").unwrap();
    let list = method.capabilities.list.as_ref().unwrap();
    assert!(!list.files.is_empty() && list.reads.is_some());
    // 副本固定叫 input.zip
    let made = takeout_zip(tmp.path());
    let input = tmp.path().join("input.zip");
    std::fs::copy(made, &input).unwrap();
    let page = host
        .list(
            plugin,
            ListRequest {
                method: "takeout".into(),
                source: "file:takeout.zip".into(),
                cursor: None,
                since: None,
                input_file: Some(input.clone()),
            },
            &CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 5);
    assert!(
        page.items
            .iter()
            .all(|i| i.url.is_some() || i.reference.is_some())
    );
    let sources = host
        .sources(plugin, "takeout", None, &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(sources.len(), 2);

    let health = host.health(plugin, &CallOptions::default()).await.unwrap();
    assert!(health.ready);
}
