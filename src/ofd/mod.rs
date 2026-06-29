//! 解析层：OFD 包/XML -> 文档模型
pub mod archive;
pub mod model;
pub mod parse;

pub use model::{ImageData, ImageObj, Obj, OfdDoc, OutlineNode, Page, PathObj, TextObj};

use anyhow::Result;

/// 打开并解析 OFD 文件
pub fn open(path: &str) -> Result<OfdDoc> {
    parse::parse_ofd(path)
}
