//! 这个插件会失败的几种方式，和它们对应的协议错误码（协议 §3.3）。
//! 不回网络类的码，也不回 `login-required`：插件只读本机的文件。
//! 读不了文件的错带 `detail`：哪个文件、哪个字段（给修插件的人看，宿主只记进失败日志）。

use usuyo_plugin_kit::protocol::{CallFailure, ErrorCode};

/// 界面上给用户的两句话：中文、英文。
type Hint = (&'static str, &'static str);

pub const NOT_AN_EXPORT: Hint = (
    "这个文件读不出 Google Takeout 里的 YouTube 数据：要的是 Takeout 的 zip（选「YouTube 和 YouTube Music」），或那个文件夹压成的 zip",
    "This file does not read as YouTube data from Google Takeout: give the Takeout zip (choose YouTube and YouTube Music) or that folder zipped",
);
pub const NO_PLAYLISTS: Hint = (
    "这份 Takeout 里没有播放列表：导出「YouTube 和 YouTube Music」时在「所有 YouTube 数据」里勾上「播放列表」",
    "This Takeout has no playlists: when exporting YouTube and YouTube Music, tick Playlists under All YouTube data",
);
pub const NO_HISTORY: Hint = (
    "这份 Takeout 里没有观看历史：导出时勾上「历史记录」，格式选 JSON",
    "This Takeout has no watch history: tick History when exporting, in JSON format",
);
pub const HTML_HISTORY: Hint = (
    "观看历史是 HTML 格式：请重新导出，在「所有 YouTube 数据」里把「历史记录」的格式选成 JSON",
    "The watch history is in HTML: export again and set the format of History to JSON under All YouTube data",
);
pub const TOO_LARGE: Hint = (
    "这个文件超过了插件愿意读的大小：导出时少选一些内容",
    "This file is larger than the plugin will read: export fewer items",
);
pub const CHANGED: Hint = (
    "Google Takeout 里 YouTube 数据的格式和这个插件认得的不一样，插件要更新",
    "The YouTube data in Google Takeout differs from what this plugin reads; the plugin needs an update",
);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// 不是这个插件读得了的文件。
    #[error("unsupported input: {message}")]
    Unsupported {
        message: String,
        detail: String,
        hint: Hint,
    },
    /// 认得出是 Takeout，但某个字段的形状变了。
    #[error("export format changed: {message}")]
    UpstreamChanged { message: String, detail: String },
    #[error("write output: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// `file` 是交来的文件里的哪一个（zip 里的条目名，或 `input`），`field` 是哪个字段。
    pub fn unsupported(file: &str, field: &str, message: impl Into<String>, hint: Hint) -> Self {
        let message = message.into();
        Error::Unsupported {
            detail: format!("file: {file}\nfield: {field}\n{message}"),
            message,
            hint,
        }
    }

    pub fn changed(file: &str, field: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Error::UpstreamChanged {
            detail: format!("file: {file}\nfield: {field}\n{message}"),
            message,
        }
    }
}

/// `hint` 按 `USUYO_LOCALE` 写：`zh` 开头写中文，其余写英文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(l) if !l.to_ascii_lowercase().starts_with("zh") => Lang::En,
            _ => Lang::Zh,
        }
    }

    fn pick(self, hint: Hint) -> String {
        match self {
            Lang::Zh => hint.0.into(),
            Lang::En => hint.1.into(),
        }
    }
}

impl Error {
    pub fn to_failure(&self, lang: Lang) -> CallFailure {
        let message = self.to_string();
        match self {
            Error::InvalidArgs(_) => CallFailure::new(ErrorCode::InvalidArgs, message),
            Error::Unsupported { detail, hint, .. } => CallFailure {
                hint: Some(lang.pick(*hint)),
                ..CallFailure::new(ErrorCode::UnsupportedInput, message)
            }
            .with_detail(detail),
            Error::UpstreamChanged { detail, .. } => CallFailure {
                hint: Some(lang.pick(CHANGED)),
                ..CallFailure::new(ErrorCode::UpstreamChanged, message)
            }
            .with_detail(detail),
            Error::Io(_) => CallFailure::new(ErrorCode::Internal, message),
        }
    }
}
