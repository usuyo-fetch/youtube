//! 测试用的小工具：按宿主的方式起插件进程、拼 zip、读样例。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEAD_PROXY: &str = "http://127.0.0.1:9";

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// 清空环境，代理指向一个关着的本机端口：插件万一发请求就会失败。
pub fn plugin(args: &[&str], input: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_usuyo-fetch-youtube"));
    cmd.env_clear()
        .env("USUYO_PROTOCOL", "1")
        .env("USUYO_LOCALE", "zh-CN")
        .env("HTTP_PROXY", DEAD_PROXY)
        .env("HTTPS_PROXY", DEAD_PROXY)
        .env("ALL_PROXY", DEAD_PROXY)
        .args(args);
    if let Some(i) = input {
        cmd.env("USUYO_INPUT", i);
    }
    cmd.output().unwrap()
}

/// stdout 上只有一个 JSON 值。
pub fn envelope(out: &Output) -> serde_json::Value {
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout is one JSON value: {stdout}"
    );
    serde_json::from_str(&stdout).unwrap()
}

/// 按名字写一个 zip；名字原样写进去，包括带 `..` 的。
pub fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

pub fn lone_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

const BASE: &str = "Takeout/YouTube and YouTube Music";

/// 整个 Takeout 的 zip，英文名字。
pub fn takeout_zip(dir: &Path) -> PathBuf {
    let path = dir.join("takeout-20261101T000000Z-001.zip");
    zip_of(
        &path,
        &[
            (&format!("{BASE}/playlists/Watch later-videos.csv"), &fixture("watch-later.csv")),
            (&format!("{BASE}/playlists/Liked videos-videos.csv"), &fixture("liked-videos.csv")),
            (&format!("{BASE}/playlists/playlists.csv"), &fixture("playlists-meta.csv")),
            (&format!("{BASE}/subscriptions/subscriptions.csv"), &fixture("subscriptions.csv")),
            (&format!("{BASE}/history/watch-history.json"), &fixture("watch-history.json")),
            (&format!("{BASE}/history/search-history.json"), b"[{\"header\": \"YouTube\", \"title\": \"Searched for x\", \"titleUrl\": \"https://www.youtube.com/results?search_query=x\", \"time\": \"2026-09-01T00:00:00.000Z\"}]"),
        ],
    );
    path
}
