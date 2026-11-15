# usuyo-fetch/youtube

YouTube 的获取插件，供 [usuyo](https://github.com/usuyo/usuyo) 调用：读你的 Google Takeout，把播放列表和观看历史列成视频链接。

| 功能 | 实现方式（方法 id） | 账号 | 风险层级 | 状态 |
|---|---|---|---|---|
| 读官方导出：播放列表、观看历史 | Google Takeout（`takeout`） | 不使用（导出时你登录 Google） | `official-export`，默认开 | 已实现（离线测试，样例手写，没有对着真实 Takeout 核对） |
| 按链接取单条视频的字幕 | 字幕（`transcript`） | 不使用 | `private-api`，默认关 | 未实现 |
| 按创作者列视频 | 频道订阅源（`channel-feed`） | 不使用 | `public-web`，默认开 | 未实现 |
| 视频文件 | 无 | 无 | 无 | 未实现 |

风险层级用清单里 `access` 的原词；`private-api`、`client-hook` 默认关，其余层级默认开。

## 方法

### `takeout`

读 Google Takeout 选「YouTube 和 YouTube Music」得到的 zip。播放列表（稍后观看、喜欢的视频、自建的）里的视频带加入的时间和列表名；观看历史选 JSON，作为可选的来源；HTML 形式的历史会提示改选 JSON。播放列表 CSV 的两列形状没有可用许可证的来源佐证。插件不联网。

### 未实现的方法

`transcript`：单条视频的字幕。`channel-feed`：按创作者列出视频。视频文件没有对应的方法。

观看历史的字段参考 [google_takeout_parser](https://github.com/purarue/google_takeout_parser)（MIT，Copyright (c) 2021 purarue）；播放列表 CSV 的两列形状只见于 WatchNow（没有许可证，只作参考）。

## 开发

`Cargo.toml` 里的 `usuyo-plugin`、`usuyo-plugin-kit` 是对 `https://github.com/usuyo/usuyo` 的 git 依赖，钉在固定的 `rev`。

本地开发时用旁边 `usuyo/` 的工作区代替它们：`usuyo-fetch/.cargo/config.toml`（不进任何插件仓库）写了下面的覆盖，cargo 从当前目录往上找这个文件，在插件文件夹里跑 cargo 就生效；从别处跑要加 `--config <这个文件>`。

```toml
[patch."https://github.com/usuyo/usuyo"]
usuyo-plugin = { path = "../usuyo/crates/usuyo-plugin" }
usuyo-plugin-kit = { path = "../usuyo/crates/usuyo-plugin-kit" }
```

`Cargo.lock` 记的是覆盖后的路径形式，所以 CI 不加 `--locked` 构建，cargo 只重新锁这两个 usuyo crate。
