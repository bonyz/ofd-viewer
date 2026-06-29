//! OFD 包读取：把 .ofd 当 ZIP，按内部路径取字节。
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::Read;

pub struct Archive {
    files: HashMap<String, Vec<u8>>,
}

impl Archive {
    pub fn open(path: &str) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("打开文件失败: {}", path))?;
        let mut za = zip::ZipArchive::new(file)
            .context("不是有效的 ZIP/OFD 文件（可能已损坏）")?;
        let mut files = HashMap::new();
        for i in 0..za.len() {
            let mut entry = za.by_index(i)?;
            let name = entry.name().to_string();
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            files.insert(name, buf);
        }
        Ok(Self { files })
    }

    pub fn get(&self, name: &str) -> Result<&[u8]> {
        self.files
            .get(name)
            .map(|v| v.as_slice())
            .with_context(|| format!("缺少 OFD 内部文件: {}", name))
    }

    /// 所有内部文件名
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(|s| s.as_str())
    }
}
