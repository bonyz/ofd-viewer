//! OFD XML 解析：OFD.xml -> Document.xml -> 各页 Content.xml
use std::collections::HashMap;

use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::archive::Archive;
use super::model::*;

/// 主入口：打开并解析 OFD 文件
pub fn parse_ofd(path: &str) -> Result<OfdDoc> {
    let arc = Archive::open(path)?;
    let doc_root = find_doc_root(&arc)?;
    let doc_dir = parent_dir(&doc_root);
    let doc_xml = arc.get(&doc_root)?;
    let common_box = parse_first_box(doc_xml);
    let page_refs = parse_document_pages(doc_xml)?;
    let annots = parse_annots(&arc, &doc_dir);
    let page_ids: Vec<String> = page_refs.iter().map(|(id, _)| id.clone()).collect();

    let mut pages = Vec::with_capacity(page_refs.len());
    for (id, base_loc) in page_refs {
        let content_path = join(&doc_dir, &base_loc);
        let objects = arc
            .get(&content_path)
            .ok()
            .and_then(|xml| parse_content(xml).ok())
            .unwrap_or_default();
        let physical_box = arc
            .get(&content_path)
            .ok()
            .and_then(|xml| parse_first_box(xml))
            .or(common_box);
        let stamps = annots.get(&id).cloned().unwrap_or_default();
        pages.push(Page {
            id,
            physical_box,
            objects,
            stamps,
        });
    }

    // 图片资源：资源 ID -> 解码像素
    let medias = parse_medias(&arc);
    let mut images = HashMap::new();
    for (id, (typ, media_file)) in &medias {
        if typ == "Image" && !media_file.is_empty() {
            let p = join(&doc_dir, media_file);
            if let Ok(bytes) = arc.get(&p) {
                if let Ok(data) = decode_image(bytes) {
                    images.insert(id.clone(), data);
                }
            }
        }
    }

    let fonts = parse_fonts(&arc, &doc_dir);
    let outlines = parse_outlines(doc_xml);

    Ok(OfdDoc {
        pages,
        images,
        fonts,
        outlines,
        page_ids,
    })
}

// ---------- 目录书签：Outlines（OutlineElem 递归）+ Bookmarks ----------

fn parse_outlines(doc_xml: &[u8]) -> Vec<OutlineNode> {
    let s = match std::str::from_utf8(doc_xml) {
        Ok(x) => x,
        Err(_) => return vec![],
    };
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack: Vec<OutlineNode> = Vec::new();
    let mut roots: Vec<OutlineNode> = Vec::new();

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let tag = local(e.name().as_ref());
                if tag == "OutlineElem" {
                    let title = attr(&e, "Title")
                        .or_else(|| attr(&e, "Name"))
                        .unwrap_or_default();
                    stack.push(OutlineNode {
                        title,
                        page_id: attr(&e, "PageID"),
                        ..Default::default()
                    });
                } else if tag == "Dest" {
                    if let Some(top) = stack.last_mut() {
                        top.page_id = attr(&e, "PageID");
                        top.left = attr(&e, "Left").and_then(|v| v.parse().ok());
                        top.top = attr(&e, "Top").and_then(|v| v.parse().ok());
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let tag = local(e.name().as_ref());
                if tag == "OutlineElem" {
                    // 自闭合节点（无子）：直接挂父/根
                    let title = attr(&e, "Title")
                        .or_else(|| attr(&e, "Name"))
                        .unwrap_or_default();
                    let node = OutlineNode {
                        title,
                        page_id: attr(&e, "PageID"),
                        ..Default::default()
                    };
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        roots.push(node);
                    }
                } else if tag == "Dest" {
                    if let Some(top) = stack.last_mut() {
                        top.page_id = attr(&e, "PageID");
                        top.left = attr(&e, "Left").and_then(|v| v.parse().ok());
                        top.top = attr(&e, "Top").and_then(|v| v.parse().ok());
                    }
                }
            }
            Ok(Event::End(e)) => {
                let tag = local(e.name().as_ref());
                if tag == "OutlineElem" {
                    if let Some(node) = stack.pop() {
                        if let Some(parent) = stack.last_mut() {
                            parent.children.push(node);
                        } else {
                            roots.push(node);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    roots
}

// ---------- 辅助 ----------

fn local(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    for a in e.attributes().with_checks(false).flatten() {
        if local(a.key.as_ref()) == key {
            return Some(String::from_utf8_lossy(a.value.as_ref()).to_string());
        }
    }
    None
}

fn parse_box(s: &str) -> (f32, f32, f32, f32) {
    let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (
        v.first().copied().unwrap_or(0.),
        v.get(1).copied().unwrap_or(0.),
        v.get(2).copied().unwrap_or(0.),
        v.get(3).copied().unwrap_or(0.),
    )
}

fn parse_floats(s: &str) -> Vec<f32> {
    s.split_whitespace().filter_map(|x| x.parse().ok()).collect()
}

fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        rel.to_string()
    } else {
        format!("{}/{}", base, rel)
    }
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

fn decode_image(bytes: &[u8]) -> Result<ImageData> {
    let img = image::load_from_memory(bytes).context("图片解码失败")?;
    let rgba = img.to_rgba8();
    Ok(ImageData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

// ---------- 资源：MultiMedia（图片）映射 ----------

fn parse_medias(arc: &Archive) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    for name in arc.names() {
        if !(name.ends_with(".xml") && (name.contains("Res") || name.contains("res"))) {
            continue;
        }
        let xml = match arc.get(name) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let s = match std::str::from_utf8(xml) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let mut reader = Reader::from_str(s);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut res_base = String::new();
        let mut cur_id: Option<String> = None;
        let mut cur_type = String::new();
        let mut in_media_file = false;
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let tag = local(e.name().as_ref());
                    if tag == "Res" {
                        res_base = attr(&e, "BaseLoc").unwrap_or_default();
                    } else if tag == "MultiMedia" {
                        cur_id = attr(&e, "ID");
                        cur_type = attr(&e, "Type").unwrap_or_default();
                    } else if tag == "MediaFile" && cur_id.is_some() {
                        in_media_file = true;
                    }
                }
                Ok(Event::Text(e)) if in_media_file => {
                    let mf = String::from_utf8_lossy(&e.into_inner()).trim().to_string();
                    if let Some(id) = cur_id.take() {
                        if !mf.is_empty() {
                            map.insert(id, (cur_type.clone(), join(&res_base, &mf)));
                        }
                    }
                    in_media_file = false;
                }
                Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                    "MediaFile" => in_media_file = false,
                    "MultiMedia" => {
                        cur_id = None;
                        cur_type.clear();
                    }
                    _ => {}
                },
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }
    map
}

// ---------- 字体：Font ID -> 嵌入子集字体字节 ----------

fn parse_fonts(arc: &Archive, doc_dir: &str) -> HashMap<String, Vec<u8>> {
    let mut map = HashMap::new();
    for name in arc.names() {
        if !(name.ends_with(".xml") && (name.contains("Res") || name.contains("res"))) {
            continue;
        }
        let xml = match arc.get(name) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let s = match std::str::from_utf8(xml) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let mut reader = Reader::from_str(s);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut res_base = String::new();
        let mut cur_id: Option<String> = None;
        let mut in_font_file = false;
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let tag = local(e.name().as_ref());
                    if tag == "Res" {
                        res_base = attr(&e, "BaseLoc").unwrap_or_default();
                    } else if tag == "Font" {
                        cur_id = attr(&e, "ID");
                    } else if tag == "FontFile" && cur_id.is_some() {
                        in_font_file = true;
                    }
                }
                Ok(Event::Text(e)) if in_font_file => {
                    let ff = String::from_utf8_lossy(&e.into_inner()).trim().to_string();
                    if let Some(id) = cur_id.take() {
                        if !ff.is_empty() {
                            let p = join(doc_dir, &join(&res_base, &ff));
                            if let Ok(bytes) = arc.get(&p) {
                                map.insert(id, bytes.to_vec());
                            }
                        }
                    }
                    in_font_file = false;
                }
                Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                    "FontFile" => in_font_file = false,
                    "Font" => cur_id = None,
                    _ => {}
                },
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }
    map
}

// ---------- OFD.xml：找 DocRoot ----------

fn find_doc_root(arc: &Archive) -> Result<String> {
    let ofd_xml = arc.get("OFD.xml").context("缺少 OFD.xml 入口")?;
    let s = std::str::from_utf8(ofd_xml).context("OFD.xml 非 UTF-8")?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_doc_root = false;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local(e.name().as_ref()) == "DocRoot" => in_doc_root = true,
            Ok(Event::Text(e)) if in_doc_root => {
                let t = String::from_utf8_lossy(&e.into_inner()).trim().to_string();
                return Ok(t);
            }
            Ok(Event::End(_)) => in_doc_root = false,
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    anyhow::bail!("OFD.xml 未找到 DocRoot");
}

// ---------- Document.xml：页面列表 ----------

fn parse_document_pages(xml: &[u8]) -> Result<Vec<(String, String)>> {
    let s = std::str::from_utf8(xml).context("Document.xml 非 UTF-8")?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut pages = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local(e.name().as_ref()) == "Page" {
                    let id = attr(&e, "ID").unwrap_or_default();
                    let base_loc = attr(&e, "BaseLoc").unwrap_or_default();
                    pages.push((id, base_loc));
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    Ok(pages)
}

// ---------- Content.xml：页面对象 ----------

pub fn parse_content(xml: &[u8]) -> Result<Vec<Obj>> {
    let s = std::str::from_utf8(xml).context("Content.xml 非 UTF-8")?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut objects = Vec::new();

    let mut cur_text: Option<TextObj> = None;
    let mut in_text_code = false;
    let mut cur_path: Option<PathObj> = None;
    let mut in_abbrev = false;

    loop {
        buf.clear();
        let ev = reader.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(e)) => match local(e.name().as_ref()).as_str() {
                "TextObject" => {
                    let ctm = attr(&e, "CTM").and_then(|s| {
                        let v: Vec<f32> =
                            s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                        (v.len() >= 6).then(|| (v[0], v[1], v[2], v[3], v[4], v[5]))
                    });
                    let stroke = attr(&e, "Stroke")
                        .map(|s| s == "true" || s == "1")
                        .unwrap_or(false);
                    let line_width = attr(&e, "LineWidth")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.0);
                    cur_text = Some(TextObj {
                        boundary: parse_box(&attr(&e, "Boundary").unwrap_or_default()),
                        font_id: attr(&e, "Font").unwrap_or_default(),
                        size: attr(&e, "Size").and_then(|v| v.parse().ok()).unwrap_or(0.),
                        x: 0.,
                        y: 0.,
                        delta_x: vec![],
                        text: String::new(),
                        ctm,
                        stroke,
                        line_width,
                    });
                }
                "TextCode" if cur_text.is_some() => {
                    in_text_code = true;
                    if let Some(t) = cur_text.as_mut() {
                        t.x = attr(&e, "X").and_then(|v| v.parse().ok()).unwrap_or(0.);
                        t.y = attr(&e, "Y").and_then(|v| v.parse().ok()).unwrap_or(0.);
                        t.delta_x = parse_floats(&attr(&e, "DeltaX").unwrap_or_default());
                    }
                }
                "ImageObject" => {
                    objects.push(Obj::Image(ImageObj {
                        boundary: parse_box(&attr(&e, "Boundary").unwrap_or_default()),
                        resource_id: attr(&e, "ResourceID").unwrap_or_default(),
                    }));
                }
                "PathObject" => {
                    cur_path = Some(PathObj {
                        boundary: parse_box(&attr(&e, "Boundary").unwrap_or_default()),
                        abbreviated: String::new(),
                        stroke: attr(&e, "Stroke"),
                        fill: attr(&e, "Fill"),
                        line_width: attr(&e, "LineWidth")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0.35),
                    });
                }
                "AbbreviatedData" if cur_path.is_some() => in_abbrev = true,
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if local(e.name().as_ref()) == "ImageObject" {
                    objects.push(Obj::Image(ImageObj {
                        boundary: parse_box(&attr(&e, "Boundary").unwrap_or_default()),
                        resource_id: attr(&e, "ResourceID").unwrap_or_default(),
                    }));
                }
            }
            Ok(Event::Text(e)) => {
                let txt = String::from_utf8_lossy(&e.into_inner()).to_string();
                if in_text_code {
                    if let Some(t) = cur_text.as_mut() {
                        t.text.push_str(&txt);
                    }
                } else if in_abbrev {
                    if let Some(p) = cur_path.as_mut() {
                        p.abbreviated.push_str(&txt);
                    }
                }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                "TextCode" => in_text_code = false,
                "TextObject" => {
                    if let Some(t) = cur_text.take() {
                        if !t.text.is_empty() {
                            objects.push(Obj::Text(t));
                        }
                    }
                }
                "AbbreviatedData" => in_abbrev = false,
                "PathObject" => {
                    if let Some(p) = cur_path.take() {
                        if !p.abbreviated.is_empty() {
                            objects.push(Obj::Path(p));
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("Content.xml 解析错误: {}", e)),
            _ => {}
        }
    }
    Ok(objects)
}

// ---------- 页面尺寸：首个 PhysicalBox（CommonData 或页内 Area）----------

fn parse_first_box(xml: &[u8]) -> Option<(f32, f32, f32, f32)> {
    let s = std::str::from_utf8(xml).ok()?;
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut want = false;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local(e.name().as_ref()) == "PhysicalBox" => want = true,
            Ok(Event::Text(e)) if want => {
                let v: Vec<f32> = String::from_utf8_lossy(&e.into_inner())
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if v.len() >= 4 {
                    return Some((v[0], v[1], v[2], v[3]));
                }
                want = false;
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    None
}

// ---------- 印章注释：Annots（PageID -> Stamp Seal）----------

fn parse_annots(arc: &Archive, doc_dir: &str) -> HashMap<String, Vec<Stamp>> {
    let mut map = HashMap::new();
    let ann_base = join(doc_dir, "Annots");
    let idx_path = join(&ann_base, "Annotations.xml");
    let xml = match arc.get(&idx_path) {
        Ok(x) => x,
        Err(_) => return map,
    };
    let s = match std::str::from_utf8(xml) {
        Ok(x) => x,
        Err(_) => return map,
    };
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut cur_page: Option<String> = None;
    let mut in_fileloc = false;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let tag = local(e.name().as_ref());
                if tag == "Page" {
                    cur_page = attr(&e, "PageID");
                } else if tag == "FileLoc" && cur_page.is_some() {
                    in_fileloc = true;
                }
            }
            Ok(Event::Text(e)) if in_fileloc => {
                let loc = String::from_utf8_lossy(&e.into_inner()).trim().to_string();
                if let Some(pid) = cur_page.clone() {
                    let p = join(&ann_base, &loc);
                    if let Ok(ann_xml) = arc.get(&p) {
                        let stamps = parse_annotation_file(ann_xml);
                        if !stamps.is_empty() {
                            map.insert(pid, stamps);
                        }
                    }
                }
                in_fileloc = false;
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()).as_str() {
                "FileLoc" => in_fileloc = false,
                "Page" => cur_page = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    map
}

fn parse_annotation_file(xml: &[u8]) -> Vec<Stamp> {
    let mut stamps = Vec::new();
    let s = match std::str::from_utf8(xml) {
        Ok(x) => x,
        Err(_) => return stamps,
    };
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut cur_boundary: Option<(f32, f32, f32, f32)> = None;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag = local(e.name().as_ref());
                if tag == "Appearance" {
                    cur_boundary = Some(parse_box(&attr(&e, "Boundary").unwrap_or_default()));
                } else if tag == "ImageObject" {
                    if let Some(b) = cur_boundary {
                        let rid = attr(&e, "ResourceID").unwrap_or_default();
                        if !rid.is_empty() {
                            stamps.push(Stamp {
                                boundary: b,
                                resource_id: rid,
                            });
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if local(e.name().as_ref()) == "Appearance" {
                    cur_boundary = None;
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }
    stamps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试样本路径由环境变量 OFD_SAMPLE 提供，避免在源码中硬编码本地路径。
    fn sample_path() -> Option<String> {
        std::env::var("OFD_SAMPLE").ok().filter(|s| !s.is_empty())
    }

    #[test]
    fn open_nonexistent_returns_err() {
        // 非存在 / 非 OFD 文件应返回错误，而非 panic（T7 异常闭环）
        assert!(parse_ofd("不存在_测试用.ofd").is_err());
    }

    #[test]
    fn parse_sample_5pages_with_text() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let doc = parse_ofd(&sample).expect("解析失败");
        assert_eq!(doc.pages.len(), 5, "应为 5 页");
        let p0 = &doc.pages[0];
        let text_all: String = p0
            .objects
            .iter()
            .filter_map(|o| if let Obj::Text(t) = o { Some(t.text.clone()) } else { None })
            .collect();
        assert!(text_all.contains("关于"), "第1页应含'关于'，实际: {}", text_all);
        let n_text = p0.objects.iter().filter(|o| matches!(o, Obj::Text(_))).count();
        let n_path = p0.objects.iter().filter(|o| matches!(o, Obj::Path(_))).count();
        let n_image = p0.objects.iter().filter(|o| matches!(o, Obj::Image(_))).count();
        println!(
            "第1页: 共{}对象 (文本{} 路径{} 图片{})；解码图片资源 {} 个；文字片段: {}",
            p0.objects.len(),
            n_text,
            n_path,
            n_image,
            doc.images.len(),
            text_all.chars().take(60).collect::<String>()
        );
    }

    #[test]
    fn parse_outlines_sample() {
        let Some(sample) = sample_path() else {
            eprintln!("跳过：未设置 OFD_SAMPLE 环境变量");
            return;
        };
        let doc = parse_ofd(&sample).expect("解析失败");
        assert_eq!(doc.page_ids.len(), 5, "应 5 个 page id");
        assert!(!doc.outlines.is_empty(), "应有目录/书签");
        let first = &doc.outlines[0];
        println!(
            "outlines[0] title={:?} page_id={:?}",
            first.title, first.page_id
        );
        if let Some(pid) = &first.page_id {
            assert!(
                doc.page_ids.iter().any(|id| id == pid),
                "page_id {} 应在 page_ids {:?} 中",
                pid,
                doc.page_ids
            );
        }
    }
}
