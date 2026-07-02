use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use rayon::prelude::*;
use crate::ecode_parser::summarize_ecode_project_for_agent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisFinding {
    pub severity: String,
    pub kind: String,
    pub title: String,
    pub path: String,
    pub relative_path: String,
    pub line: usize,
    pub evidence: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateLocation {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    pub line: usize,
    pub line_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub title: String,
    pub normalized_size: usize,
    pub locations: Vec<DuplicateLocation>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisMetrics {
    pub source_file_count: usize,
    pub analyzed_file_count: usize,
    pub skipped_module_file_count: usize,
    pub subprogram_count: usize,
    pub hardcoded_url_count: usize,
    pub insecure_http_url_count: usize,
    pub selector_count: usize,
    pub network_call_count: usize,
    pub duplicate_group_count: usize,
    pub empty_component_count: usize,
    pub sensitive_field_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAnalysisResult {
    pub success: bool,
    pub ecode_dir: String,
    pub summary: String,
    pub metrics: AnalysisMetrics,
    pub findings: Vec<AnalysisFinding>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub note: String,
}

fn normalize_subprogram_body(body: &str) -> String {
    // 移除易语言注释（以 ' 开头的部分）
    let mut normalized = String::new();
    for line in body.lines() {
        let line_trim = line.trim();
        if line_trim.starts_with('\'') {
            continue;
        }
        // 移除行尾注释
        let clean_line = if let Some(idx) = line.find('\'') {
            &line[..idx]
        } else {
            line
        };
        normalized.push_str(clean_line);
    }

    // 移除空白字符、数字、易语言双引号 “ ” 内的字符以获得逻辑骨架
    normalized
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_numeric())
        .collect::<String>()
        .replace('“', "")
        .replace('”', "")
        .replace('"', "")
}

pub async fn analyze_project_rust(ecode_dir: String) -> Result<ProjectAnalysisResult, String> {
    let root = PathBuf::from(&ecode_dir);
    if !root.exists() {
        return Err(format!("文本工程目录不存在：{}", root.display()));
    }

    // 获取整个工程的 AST 结构与摘要
    let project_map = summarize_ecode_project_for_agent(ecode_dir.clone(), Some(true), Some(10000)).await?;
    let source_files = project_map.source_files;

    // 并行读取和静态行扫描
    let scan_results: Vec<Result<(Vec<AnalysisFinding>, Vec<(String, DuplicateLocation, String)>), String>> = source_files
        .par_iter()
        .map(|file| {
            let mut findings = Vec::new();
            let mut subprogram_bodies = Vec::new();

            let bytes = fs::read(&file.path).map_err(|e| format!("无法读取文件 {}: {}", file.relative_path, e))?;
            let content = match String::from_utf8(bytes.clone()) {
                Ok(s) => s,
                Err(_) => encoding_rs::GBK.decode(&bytes).0.to_string(),
            };
            let lines: Vec<&str> = content.lines().collect();

            // 1. 扫描组件为空
            if (file.kind == "类模块" || file.kind == "窗口程序集") && file.subprograms.is_empty() {
                findings.push(AnalysisFinding {
                    severity: "info".to_string(),
                    kind: "empty_component".to_string(),
                    title: "空壳组件".to_string(),
                    path: file.path.clone(),
                    relative_path: file.relative_path.clone(),
                    line: 1,
                    evidence: format!("{} {} 没有定义任何子程序", file.kind, file.assembly.as_deref().unwrap_or("")),
                    suggestion: "如果不再使用，可以删除该类模块以精简工程。".to_string(),
                });
            }

            // 2. 扫描子程序细节
            for sub in &file.subprograms {
                if sub.line_count <= 3 && sub.calls.is_empty() && sub.locals.is_empty() {
                    findings.push(AnalysisFinding {
                        severity: "info".to_string(),
                        kind: "empty_subprogram".to_string(),
                        title: "空子程序".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: sub.line,
                        evidence: sub.signature.clone(),
                        suggestion: "确认是否为事件占位；不需要时可清理。".to_string(),
                    });
                }

                // 提取子程序实体源码用于排重
                let start_idx = sub.line.saturating_sub(1);
                let end_idx = (start_idx + sub.line_count).min(lines.len());
                if start_idx < lines.len() {
                    let body = lines[start_idx..end_idx].join("\n");
                    let normalized = normalize_subprogram_body(&body);
                    if normalized.len() >= 50 && sub.line_count >= 3 {
                        subprogram_bodies.push((
                            normalized,
                            DuplicateLocation {
                                name: sub.name.clone(),
                                path: file.path.clone(),
                                relative_path: file.relative_path.clone(),
                                line: sub.line,
                                line_count: sub.line_count,
                            },
                            body,
                        ));
                    }
                }
            }

            // 3. 逐行匹配诊断
            for (idx, line) in lines.iter().enumerate() {
                let line_no = idx + 1;
                let line_lower = line.to_lowercase();

                // 匹配硬编码 URL
                if line_lower.contains("http://") || line_lower.contains("https://") {
                    let severity = if line_lower.contains("http://") { "risk" } else { "warning" };
                    findings.push(AnalysisFinding {
                        severity: severity.to_string(),
                        kind: "hardcoded_url".to_string(),
                        title: if severity == "risk" { "明文 HTTP 地址硬编码" } else { "URL 地址硬编码" }.to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "建议提取远程配置或常量集中管理，并升级为强制 HTTPS 请求。".to_string(),
                    });
                }

                // 匹配页面选择器
                if line_lower.contains("\"#") || line_lower.contains("“#") {
                    findings.push(AnalysisFinding {
                        severity: "warning".to_string(),
                        kind: "hardcoded_selector".to_string(),
                        title: "前端页面选择器硬编码".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "避免将 HTML 元素选择器写死，防范网页 DOM 结构调整带来的逻辑失效。".to_string(),
                    });
                }

                // 匹配网络调用缺少容错
                if line.contains("网页_访问") || line.contains("网页_访问_对象") || line.contains("网络_") {
                    findings.push(AnalysisFinding {
                        severity: "warning".to_string(),
                        kind: "network_call".to_string(),
                        title: "网络调用隐患".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "网络请求应增加异常判断、超时和重试机制，对结果判断是否为空。".to_string(),
                    });
                }

                // 敏感表单与个人隐私字段
                if line_lower.contains("identitynumber")
                    || line_lower.contains("mobile")
                    || line.contains("身份证")
                    || line.contains("手机号")
                    || line.contains("密码")
                    || line.contains("姓名")
                {
                    findings.push(AnalysisFinding {
                        severity: "risk".to_string(),
                        kind: "sensitive_field".to_string(),
                        title: "个人敏感隐私字段披露".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "传输和存储个人敏感数据时应进行掩码或加密，本地日志严禁明文记录密码和完整身份证。".to_string(),
                    });
                }

                // 临界区锁分析
                if line.contains("创建进入许可证") || line.contains("进入许可") || line.contains("退出许可") {
                    findings.push(AnalysisFinding {
                        severity: "info".to_string(),
                        kind: "concurrency_lock".to_string(),
                        title: "线程同步锁/许可证".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "检测到线程同步临界区锁。迁移到 Python/Go/Rust 时需要对应映射为 Mutex/Lock 等同步原语。".to_string(),
                    });
                }

                // 本地配置项读写
                if line.contains("读配置项") || line.contains("写配置项") {
                    findings.push(AnalysisFinding {
                        severity: "info".to_string(),
                        kind: "config_access".to_string(),
                        title: "本地配置读写操作".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "使用本地配置文件（通常为 INI 格式）。在新语言中，建议改用标准的 TOML/JSON/YAML 进行配置持久化。".to_string(),
                    });
                }

                // 本地数据库访问
                if line.contains("打开数据库") || line.contains("执行SQL") || line_lower.contains("sqlite") {
                    findings.push(AnalysisFinding {
                        severity: "info".to_string(),
                        kind: "database_access".to_string(),
                        title: "本地数据库操作".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "程序操作了本地 SQL/SQLite 数据库。迁移时，应审计表结构并使用目标语言对应的 DB 支持库或 ORM 进行对接。".to_string(),
                    });
                }

                // 子进程/外部命令执行
                if line.contains("运行 (") || line.contains("执行 (") || line.contains("ShellExecute") {
                    findings.push(AnalysisFinding {
                        severity: "warning".to_string(),
                        kind: "subprocess_exec".to_string(),
                        title: "调用外部子进程/系统命令".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "程序调用了 Windows 系统命令或启动外部程序，带来平台锁定与命令注入隐患。迁移时应尽量改用原生库函数，或进行跨平台子进程包装。".to_string(),
                    });
                }

                // 嵌入式脚本引擎运行
                if line.contains("类_脚本组件") || line.contains("类_V8") || line_lower.contains("v8.运行") {
                    findings.push(AnalysisFinding {
                        severity: "info".to_string(),
                        kind: "embedded_script".to_string(),
                        title: "调用外部 JS / 脚本引擎".to_string(),
                        path: file.path.clone(),
                        relative_path: file.relative_path.clone(),
                        line: line_no,
                        evidence: line.trim().to_string(),
                        suggestion: "检测到外部 JS/脚本引擎执行。多用于平台签名校验或解密，迁移时可考虑提取对应的 JS 文件并使用 Python quickjs 或 Go otto 等脚本引擎绑定执行。".to_string(),
                    });
                }
            }

            Ok((findings, subprogram_bodies))
        })
        .collect();

    // 整合分析数据
    let mut all_findings = Vec::new();
    let mut duplicate_map: HashMap<String, Vec<(DuplicateLocation, String)>> = HashMap::new();

    let mut hardcoded_url_count = 0;
    let mut insecure_http_url_count = 0;
    let mut selector_count = 0;
    let mut network_call_count = 0;
    let mut sensitive_field_count = 0;
    let mut empty_component_count = 0;
    let mut subprogram_count = 0;

    for file_summary in &source_files {
        subprogram_count += file_summary.subprograms.len();
    }

    for res in scan_results {
        let (findings, bodies) = res?;
        for f in findings {
            if f.kind == "hardcoded_url" {
                hardcoded_url_count += 1;
                if f.evidence.contains("http://") {
                    insecure_http_url_count += 1;
                }
            } else if f.kind == "hardcoded_selector" {
                selector_count += 1;
            } else if f.kind == "network_call" {
                network_call_count += 1;
            } else if f.kind == "sensitive_field" {
                sensitive_field_count += 1;
            } else if f.kind == "empty_component" || f.kind == "empty_subprogram" {
                empty_component_count += 1;
            }
            all_findings.push(f);
        }

        for (norm_body, location, raw_body) in bodies {
            duplicate_map
                .entry(norm_body)
                .or_default()
                .push((location, raw_body));
        }
    }

    // 重复代码逻辑分类
    let mut duplicate_groups = Vec::new();
    for (norm_body, list) in duplicate_map {
        if list.len() >= 2 {
            let names: Vec<String> = list.iter().map(|(loc, _)| loc.name.clone()).collect();
            let locations: Vec<DuplicateLocation> = list.iter().map(|(loc, _)| loc.clone()).collect();
            duplicate_groups.push(DuplicateGroup {
                title: format!("重复子程序逻辑：{}", names.join(" / ")),
                normalized_size: norm_body.len(),
                locations,
                suggestion: "将此重复片段重构成公共模块或子程序，减少后期维护成本。".to_string(),
            });
        }
    }
    duplicate_groups.sort_by(|a, b| b.normalized_size.cmp(&a.normalized_size));

    let metrics = AnalysisMetrics {
        source_file_count: project_map.source_file_count,
        analyzed_file_count: source_files.len(),
        skipped_module_file_count: project_map.skipped_module_file_count,
        subprogram_count,
        hardcoded_url_count,
        insecure_http_url_count,
        selector_count,
        network_call_count,
        duplicate_group_count: duplicate_groups.len(),
        empty_component_count,
        sensitive_field_count,
    };

    let summary = format!(
        "已分析 {}/{} 个主工程源码文件：发现 {} 组重复逻辑、{} 个硬编码 URL（{} 个不安全的明文 HTTP 链接）、{} 个选择器和 {} 处网络调用，{} 处涉及个人隐私信息的敏感字段。",
        metrics.analyzed_file_count,
        metrics.source_file_count,
        metrics.duplicate_group_count,
        metrics.hardcoded_url_count,
        metrics.insecure_http_url_count,
        metrics.selector_count,
        metrics.network_call_count,
        metrics.sensitive_field_count
    );

    // 按严重程度对发现结果进行排序：risk 优先，其次 warning，最后 info
    all_findings.sort_by(|a, b| {
        let score = |s: &str| match s {
            "risk" => 3,
            "warning" => 2,
            "info" => 1,
            _ => 0,
        };
        score(&b.severity).cmp(&score(&a.severity))
    });

    Ok(ProjectAnalysisResult {
        success: true,
        ecode_dir: project_map.ecode_dir,
        summary,
        metrics,
        findings: all_findings.into_iter().take(3000).collect(), // 截断保留前 3000 个
        duplicate_groups: duplicate_groups.into_iter().take(20).collect(), // 重复最高的前 20 个
        note: "静态诊断已完成，由 Rust 并行计算扫描生成。".to_string(),
    })
}
