//! 打开交来的文件：整个 Takeout 的 zip，或单个文件（一个播放列表的 CSV、观看历史的 JSON 或 HTML）。
//!
//! zip 里的条目只按名字读进内存；条目名从不拼成要写的路径。
//! 名字有 `..`、以 `/` 开头、含反斜杠或控制字符的条目当没有。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Error, NOT_AN_EXPORT, TOO_LARGE};
use crate::limits::Limits;

/// 交来的单个文件在报错和来源里叫什么。
pub const PLAIN: &str = "input";

pub enum Archive {
    /// 单个文件：已经读进内存，`ext` 是按内容认出的 `csv`、`json` 或 `html`。
    Lone { bytes: Vec<u8>, ext: &'static str },
    Zip {
        archive: Box<ZipArchive<File>>,
        /// 名字安全的条目：`(名字, 序号)`，按名字排好。
        entries: Vec<(String, usize)>,
    },
}

/// 名字能不能当 zip 里的一条正常条目：不会指到别处。
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && !name.split('/').any(|seg| seg == "..")
        && name.as_bytes().get(1) != Some(&b':')
}

/// 单个文件是什么：HTML 以 `<` 开头，JSON 以 `[` 或 `{` 开头，别的当 CSV。
fn sniff(bytes: &[u8]) -> &'static str {
    let head = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match head.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'<') => "html",
        Some(b'[' | b'{') => "json",
        _ => "csv",
    }
}

pub fn open(path: &Path, limits: &Limits) -> Result<Archive, Error> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    let n = file.read(&mut magic)?;
    if n == 4 && magic.starts_with(b"PK") && matches!(&magic[2..], [3, 4] | [5, 6]) {
        return open_zip(file, limits);
    }
    let cap = limits.json_bytes.max(limits.csv_bytes);
    let len = file.metadata()?.len();
    if len > cap {
        return Err(Error::unsupported(
            PLAIN,
            "(whole file)",
            format!("the file is {len} bytes, over the {cap} byte limit"),
            TOO_LARGE,
        ));
    }
    let mut bytes = magic[..n].to_vec();
    file.read_to_end(&mut bytes)?;
    let ext = sniff(&bytes);
    Ok(Archive::Lone { bytes, ext })
}

fn open_zip(file: File, limits: &Limits) -> Result<Archive, Error> {
    let archive = ZipArchive::new(file).map_err(|e| {
        Error::unsupported(
            PLAIN,
            "(zip)",
            format!("not a readable zip file: {e}"),
            NOT_AN_EXPORT,
        )
    })?;
    if archive.len() > limits.zip_entries {
        return Err(Error::unsupported(
            PLAIN,
            "(zip entries)",
            format!(
                "the zip has {} entries, over the {} limit",
                archive.len(),
                limits.zip_entries
            ),
            TOO_LARGE,
        ));
    }
    let archive = Box::new(archive);
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        // 名字读不出来（不是 UTF-8）的条目当没有
        if let Some(Ok(name)) = archive.name_for_index(i)
            && !name.ends_with('/')
            && safe_name(&name)
        {
            entries.push((name.into_owned(), i));
        }
    }
    entries.sort();
    entries.dedup_by(|a, b| a.0 == b.0);
    Ok(Archive::Zip { archive, entries })
}

impl Archive {
    /// 扩展名（小写）是 `ext` 的条目名。
    pub fn names(&self, ext: &str) -> Vec<String> {
        match self {
            Archive::Lone { ext: e, .. } => {
                if *e == ext {
                    vec![PLAIN.into()]
                } else {
                    Vec::new()
                }
            }
            Archive::Zip { entries, .. } => entries
                .iter()
                .filter(|(n, _)| {
                    n.rsplit_once('.')
                        .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
                })
                .map(|(n, _)| n.clone())
                .collect(),
        }
    }

    /// 读一个条目的前 `limit` 字节；条目更长时 `Ok(None)`，由调用方决定怎么说。
    pub fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, Error> {
        match self {
            Archive::Lone { bytes, .. } => Ok((name == PLAIN).then(|| bytes.clone())),
            Archive::Zip { archive, entries } => {
                let Some(&(_, index)) = entries.iter().find(|(n, _)| n == name) else {
                    return Ok(None);
                };
                let entry = archive.by_index(index).map_err(|e| {
                    Error::unsupported(
                        name,
                        "(zip entry)",
                        format!("cannot read the entry: {e}"),
                        NOT_AN_EXPORT,
                    )
                })?;
                if entry.size() > limit {
                    return Err(Error::unsupported(
                        name,
                        "(zip entry)",
                        format!(
                            "the entry is {} bytes, over the {limit} byte limit",
                            entry.size()
                        ),
                        TOO_LARGE,
                    ));
                }
                let mut bytes = Vec::new();
                entry.take(limit + 1).read_to_end(&mut bytes).map_err(|e| {
                    Error::unsupported(
                        name,
                        "(zip entry)",
                        format!("cannot read the entry: {e}"),
                        NOT_AN_EXPORT,
                    )
                })?;
                if bytes.len() as u64 > limit {
                    return Err(Error::unsupported(
                        name,
                        "(zip entry)",
                        format!("the entry is over the {limit} byte limit when unpacked"),
                        TOO_LARGE,
                    ));
                }
                Ok(Some(bytes))
            }
        }
    }

    /// 读一个条目的开头最多 `n` 字节，用来在不读完整个文件的情况下认形状。
    pub fn head(&mut self, name: &str, n: usize) -> Result<Vec<u8>, Error> {
        match self {
            Archive::Lone { bytes, .. } => Ok(bytes[..bytes.len().min(n)].to_vec()),
            Archive::Zip { archive, entries } => {
                let Some(&(_, index)) = entries.iter().find(|(e, _)| e == name) else {
                    return Ok(Vec::new());
                };
                let entry = archive.by_index(index).map_err(|e| {
                    Error::unsupported(
                        name,
                        "(zip entry)",
                        format!("cannot read the entry: {e}"),
                        NOT_AN_EXPORT,
                    )
                })?;
                let mut head = Vec::new();
                entry.take(n as u64).read_to_end(&mut head).map_err(|e| {
                    Error::unsupported(
                        name,
                        "(zip entry)",
                        format!("cannot read the entry: {e}"),
                        NOT_AN_EXPORT,
                    )
                })?;
                Ok(head)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_kinds() {
        assert!(safe_name(
            "Takeout/YouTube and YouTube Music/playlists/Watch later-videos.csv"
        ));
        assert!(!safe_name("../a.csv"));
        assert!(!safe_name("/a.csv"));
        assert!(!safe_name("a\\b.csv"));
        assert!(!safe_name("C:/a.csv"));
        assert_eq!(sniff(b"\xef\xbb\xbf <html>"), "html");
        assert_eq!(sniff(b" [ {\"a\": 1} ]"), "json");
        assert_eq!(sniff(b"Video ID,time\n"), "csv");
    }
}
