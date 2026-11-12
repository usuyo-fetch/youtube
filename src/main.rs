//! YouTube 获取插件的入口：子命令交给 [`usuyo_fetch_youtube::cli::run`]。

use std::process::ExitCode;

use usuyo_fetch_youtube::cli;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(cli::run(&args))
}
