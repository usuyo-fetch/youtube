//! 交来的文件不可信：各处读多少、数多少都有上限。测试用小的上限，运行用 [`Limits::default`]。

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// zip 里最多多少个条目。
    pub zip_entries: usize,
    /// 一个 CSV 最多多少字节。
    pub csv_bytes: u64,
    /// 一个观看历史 JSON 最多多少字节。
    pub json_bytes: u64,
    /// 最多读几个 CSV。
    pub csv_files: usize,
    /// 最多读几个观看历史 JSON。
    pub history_files: usize,
    /// 列出的条目最多多少条。
    pub items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            zip_entries: 500_000,
            csv_bytes: 32 << 20,
            json_bytes: 512 << 20,
            csv_files: 2_000,
            history_files: 8,
            items: 2_000_000,
        }
    }
}
