//! OFD 文档模型
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct OfdDoc {
    pub pages: Vec<Page>,
    pub images: HashMap<String, ImageData>,
    pub fonts: HashMap<String, Vec<u8>>,
    /// 声明未内嵌的字体：Font ID -> FontName（渲染时按名匹配系统字体）
    pub font_names: HashMap<String, String>,
    /// 目录书签树（Document.xml Outlines，OutlineElem 递归）
    pub outlines: Vec<OutlineNode>,
    /// Document.xml 各 Page 的 ID 顺序（跳转 PageID -> 页索引用）
    pub page_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Page {
    pub id: String,
    pub physical_box: Option<(f32, f32, f32, f32)>,
    pub objects: Vec<Obj>,
    pub stamps: Vec<Stamp>,
}

#[derive(Clone, Debug, Default)]
pub struct Stamp {
    pub boundary: (f32, f32, f32, f32),
    pub resource_id: String,
}

/// 目录书签节点（递归）。对应 Document.xml 的 OutlineElem / Bookmark。
#[derive(Clone, Debug, Default)]
pub struct OutlineNode {
    pub title: String,
    pub page_id: Option<String>, // Dest PageID
    pub left: Option<f32>,       // Dest Left（mm）
    pub top: Option<f32>,        // Dest Top（mm）
    pub children: Vec<OutlineNode>,
}

#[derive(Debug)]
pub enum Obj {
    Text(TextObj),
    Image(ImageObj),
    Path(PathObj),
}

#[derive(Debug, Default)]
pub struct TextObj {
    pub boundary: (f32, f32, f32, f32),
    pub font_id: String,
    pub size: f32,
    pub x: f32,
    pub y: f32,
    pub delta_x: Vec<f32>,
    pub text: String,
    /// 变换矩阵 [a,b,c,d,e,f]（国标 CTM）；None=单位矩阵
    pub ctm: Option<(f32, f32, f32, f32, f32, f32)>,
    /// Stroke="true" 文字描边（加粗）
    pub stroke: bool,
    /// 描边线宽 mm
    pub line_width: f32,
    /// 填充色（DrawParam FillColor，RGB）；None=默认黑
    pub fill_color: Option<[u8; 3]>,
}

#[derive(Debug, Default)]
pub struct ImageObj {
    pub boundary: (f32, f32, f32, f32),
    pub resource_id: String,
}

#[derive(Debug, Default)]
pub struct PathObj {
    pub boundary: (f32, f32, f32, f32),
    pub abbreviated: String,
    pub stroke: Option<String>,
    pub fill: Option<String>,
    pub line_width: f32,
    /// 描边色（DrawParam StrokeColor，RGB）；None=默认黑
    pub stroke_color: Option<[u8; 3]>,
    /// 填充色（DrawParam FillColor，RGB）；None=默认黑
    pub fill_color: Option<[u8; 3]>,
}
