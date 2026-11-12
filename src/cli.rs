//! 子命令照协议 §3.2：`canonicalize`、`sources`、`list`、`health`，读参数用 plugin-kit。错误只在这里变成信封。
//!
//! 只读两个环境变量：`USUYO_LOCALE`（`hint` 和来源标题的语言）和 `USUYO_INPUT`（交来的 Takeout 文件的副本）。
//! 方法 `takeout` 只有 `list`：条目都是视频链接，内容由别的插件按链接去取。插件不连任何网络。
//! 来源：`file:…`（播放列表）、`youtube:playlists`、`youtube:playlist:<条目>`、`youtube:watch-history`。

use std::path::PathBuf;

use serde::Serialize;
use usuyo_plugin_kit::fetch::{Call, USAGE};
use usuyo_plugin_kit::protocol::{CallFailure, Canonicalized, Envelope, ErrorCode, FetchHealth};

use crate::error::{Error, Lang};
use crate::limits::Limits;
use crate::takeout;

/// 跑一个子命令，stdout 上写一个信封，返回退出码。
pub fn run(args: &[String]) -> u8 {
    let lang = Lang::from_locale(std::env::var("USUYO_LOCALE").ok().as_deref());
    let Some(call) = Call::parse(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let input = std::env::var_os("USUYO_INPUT").map(PathBuf::from);
    let limits = Limits::default();
    let method = call.method().map(str::to_owned);
    match (method.as_deref(), call) {
        // 插件不认任何链接：清单没有 `match`，宿主不会调到这里
        (_, Call::Canonicalize { .. }) => print(Ok(Canonicalized::NoMatch), lang),
        (Some("takeout"), Call::Sources { link: None, .. }) => {
            print(takeout::sources(input.as_deref(), lang, &limits), lang)
        }
        (Some("takeout"), Call::List { source, cursor, .. }) => print(
            takeout::list(&source, input.as_deref(), cursor.as_deref(), &limits),
            lang,
        ),
        // 不向任何服务发请求；没有本机程序要查
        (_, Call::Health { .. }) => print(Ok(FetchHealth::new(env!("CARGO_PKG_VERSION"))), lang),
        (method, _) => print::<()>(
            Err(Error::InvalidArgs(format!(
                "this call does not apply to method {method:?}"
            ))),
            lang,
        ),
    }
}

fn print<T: Serialize>(result: Result<T, Error>, lang: Lang) -> u8 {
    let envelope = match result {
        Ok(body) => Envelope::Ok(body),
        Err(error) => Envelope::<T>::Err(error.to_failure(lang)),
    };
    match serde_json::to_string(&envelope) {
        Ok(json) => {
            println!("{json}");
            u8::from(envelope.exit_code() != 0)
        }
        Err(e) => {
            // 信封写不出来：当作插件自己的错，写一个最小的信封
            let failure = CallFailure::new(
                ErrorCode::Internal,
                format!("cannot write the envelope: {e}"),
            );
            println!(
                "{}",
                serde_json::to_string(&Envelope::<()>::Err(failure)).unwrap_or_default()
            );
            1
        }
    }
}
