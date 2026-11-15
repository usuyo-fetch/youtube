# usuyo-fetch/youtube

YouTube 的获取插件，供 [usuyo](https://github.com/usuyo/usuyo) 调用：读你的 Google Takeout，把播放列表和观看历史列成视频链接。

| 方法 | 清单里的 id | 取得 | 账号 | 层级 | 现状 |
|---|---|---|---|---|---|
| Google Takeout | `takeout` | 存过的：播放列表（稍后观看、喜欢的视频、自建的）里的视频，带加入的时间和列表名；观看历史（选 JSON）作为可选的来源；HTML 形式的历史会提示改选 JSON | 不使用（导出时你登录 Google） | `official-export` | 已实现；只做了离线测试，样例手写，没有对着真实 Takeout 核对；播放列表 CSV 的两列形状没有可用许可证的来源佐证 |
| 字幕 | `transcript` | 单条字幕 | 不使用 | `private-api` | 未实现 |
| 频道订阅源 | `channel-feed` | 按创作者列 | 不使用 | `public-web` | 未实现 |

观看历史的字段参考 [google_takeout_parser](https://github.com/purarue/google_takeout_parser)（MIT，Copyright (c) 2021 purarue）；播放列表 CSV 的两列形状只见于 WatchNow（没有许可证，只作参考）；字幕将来包 [youtube-transcript-api](https://github.com/jdepoix/youtube-transcript-api)（MIT）。

## 开发

`Cargo.toml` 里的 `usuyo-plugin`、`usuyo-plugin-kit` 是对 `https://github.com/usuyo/usuyo` 的 git 依赖，钉在固定的 `rev`。

本地开发时用旁边 `usuyo/` 的工作区代替它们：`usuyo-fetch/.cargo/config.toml`（不进任何插件仓库）写了下面的覆盖，cargo 从当前目录往上找这个文件，在插件文件夹里跑 cargo 就生效；从别处跑要加 `--config <这个文件>`。

```toml
[patch."https://github.com/usuyo/usuyo"]
usuyo-plugin = { path = "../usuyo/crates/usuyo-plugin" }
usuyo-plugin-kit = { path = "../usuyo/crates/usuyo-plugin-kit" }
```

`Cargo.lock` 记的是覆盖后的路径形式，所以 CI 不加 `--locked` 构建，cargo 只重新锁这两个 usuyo crate。
