use std::path::PathBuf;
use crate::local_files::{read_text_file_for_agent, write_text_file};

#[derive(Debug)]
pub struct PatchBlock {
    pub search: String,
    pub replace: String,
}

pub fn parse_patches(patch_text: &str) -> Vec<PatchBlock> {
    let mut patches = Vec::new();
    let lines: Vec<&str> = patch_text.lines().collect();
    let mut i = 0;
    
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("<<<<<<< SEARCH") {
            let mut search_lines = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("=======") {
                search_lines.push(lines[i]);
                i += 1;
            }
            
            if i < lines.len() && lines[i].trim().starts_with("=======") {
                let mut replace_lines = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with(">>>>>>> REPLACE") {
                    replace_lines.push(lines[i]);
                    i += 1;
                }
                
                let search = search_lines.join("\n");
                let replace = replace_lines.join("\n");
                patches.push(PatchBlock { search, replace });
            }
        }
        i += 1;
    }
    patches
}

pub async fn apply_patch_file(file_path: String, patch_content: String) -> Result<String, String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在：{}", file_path));
    }

    let patches = parse_patches(&patch_content);
    if patches.is_empty() {
        return Err("未在输入的 patch 中解析到有效的 <<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE 块".to_string());
    }

    // 读取目标文件所有字符
    let file_result = read_text_file_for_agent(file_path.clone(), Some(10_000_000)).await?;
    let mut current_content = file_result.content;
    let original_encoding = file_result.encoding;

    for (index, patch) in patches.iter().enumerate() {
        // 统一换行符进行匹配
        let norm_content = current_content.replace("\r\n", "\n").replace('\r', "\n");
        let norm_search = patch.search.replace("\r\n", "\n").replace('\r', "\n");
        let norm_replace = patch.replace.replace("\r\n", "\n").replace('\r', "\n");

        if norm_search.trim().is_empty() {
            return Err(format!("第 {} 个 Patch 的 SEARCH 块为空，无法进行替换", index + 1));
        }

        // 进行精确匹配检索
        let matches: Vec<_> = norm_content.match_indices(&norm_search).collect();
        if matches.is_empty() {
            // 如果没找到，我们可以做一下更加宽松的 trim 匹配
            let trim_content = norm_content.trim();
            let trim_search = norm_search.trim();
            if trim_content.contains(trim_search) && trim_content.match_indices(trim_search).count() == 1 {
                let search_idx = norm_content.find(trim_search).unwrap();
                let before = &norm_content[..search_idx];
                let after = &norm_content[search_idx + trim_search.len()..];
                current_content = format!("{}{}{}", before, norm_replace.trim(), after);
            } else {
                return Err(format!(
                    "第 {} 个 Patch 块在文件中找不到指定的 SEARCH 代码片段。原 SEARCH 段：\n{}",
                    index + 1, patch.search
                ));
            }
        } else if matches.len() > 1 {
            return Err(format!(
                "第 {} 个 Patch 块在文件中找到多个重复的匹配项（共 {} 处），替换不唯一，拒绝应用。请提供更多上下文行。",
                index + 1, matches.len()
            ));
        } else {
            // 唯一匹配，进行替换
            let (match_idx, _) = matches[0];
            let before = &norm_content[..match_idx];
            let after = &norm_content[match_idx + norm_search.len()..];
            current_content = format!("{}{}{}", before, norm_replace, after);
        }
    }

    // 重新写回文件，维持原来的编码
    let write_encoding = Some(original_encoding.to_lowercase());
    write_text_file(file_path, current_content.clone(), write_encoding).await?;

    Ok(format!("成功应用 {} 个 Patch 块修改。", patches.len()))
}
