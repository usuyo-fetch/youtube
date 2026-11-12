//! usuyo 的 YouTube 获取插件：读你的 Google Takeout（整个 zip，或 `YouTube and YouTube Music` 文件夹压成的 zip），
//! 把播放列表（稍后观看、喜欢的视频、自建的列表）和观看历史列成视频链接。不联网。
//!
//! - [`cli`]：子命令和信封，入口只调它。
//! - `input`：打开交来的文件（zip 或单个文件），按名字读 zip 里的条目，不解压到磁盘。
//! - [`takeout`]：按结构（不按文件夹、文件的名字，它们随界面语言改名）找出播放列表和观看历史，列出视频。
//! - `csv`：播放列表的 CSV。

pub mod cli;
mod csv;
mod error;
mod input;
mod limits;
pub mod takeout;

pub use error::{Error, Lang};
pub use limits::Limits;
