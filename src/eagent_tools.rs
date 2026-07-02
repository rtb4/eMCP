use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCheck {
    pub path: Option<String>,
    pub exists: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EAgentTools {
    pub project_root: Option<String>,
    pub bundled_tools_root: Option<String>,
    pub ebuild_exe: ToolCheck,
    pub e2txt_exe: ToolCheck,
    pub ecode_template_dir: ToolCheck,
    pub eparser_exe: ToolCheck,
    pub eparser_dll: ToolCheck,
    pub e_root: ToolCheck,
    pub ecl_exe: ToolCheck,
    pub e_exe: ToolCheck,
    pub el_exe: ToolCheck,
    pub link_dll: ToolCheck,
    pub vc_link_exe: ToolCheck,
    pub static_lib_dir: ToolCheck,
}

pub fn get_eaicoding_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().join(".eaicoding"))
}

fn tool_check(path: Option<PathBuf>, reason: Option<String>) -> ToolCheck {
    let exists = path.as_ref().is_some_and(|item| item.exists());
    ToolCheck {
        path: path.map(|item| item.to_string_lossy().to_string()),
        exists,
        reason: if exists { None } else { reason },
    }
}

fn find_local_root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(current) = env::current_dir() {
        candidates.push(current);
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.to_path_buf());
        }
    }

    for candidate in candidates {
        for ancestor in candidate.ancestors() {
            // 识别项目根目录：包含 Cargo.toml + resources/eagent-tools 的纯 Rust 项目
            if ancestor.join("Cargo.toml").exists()
                && ancestor.join("resources").join("eagent-tools").exists()
            {
                return Some(ancestor.to_path_buf());
            }
            // 兜底：包含 AGENTS.md 的工作区根目录
            if ancestor.join("AGENTS.md").exists() {
                return Some(ancestor.to_path_buf());
            }
        }
    }

    None
}

fn resource_tools_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("eagent-tools"));
            candidates.push(parent.join("resources").join("eagent-tools"));
        }
    }

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("eagent-tools"));
        candidates.push(current_dir.join("resources").join("eagent-tools"));
    }

    if let Some(eaicoding) = get_eaicoding_dir() {
        candidates.push(eaicoding.join("eagent-tools"));
    }

    candidates.into_iter().find(|path| path.exists())
}

pub fn app_tools_root() -> Option<PathBuf> {
    // 1. 优先使用可执行文件同级目录下的 eagent-tools 文件夹作为运行环境根目录
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            // 如果是在开发构建目录下（即 target/debug 或 target/release），我们寻找项目根目录下的 resources/eagent-tools
            let is_cargo_target = parent.ends_with("debug") || parent.ends_with("release");
            if !is_cargo_target {
                return Some(parent.join("eagent-tools"));
            }
        }
    }

    // 2. 如果是开发模式，使用项目根目录下的 resources/eagent-tools 或当前目录下的 eagent-tools
    if let Ok(current_dir) = env::current_dir() {
        let dev_tools = current_dir.join("resources").join("eagent-tools");
        if dev_tools.exists() {
            return Some(dev_tools);
        }
        let local_tools = current_dir.join("eagent-tools");
        if local_tools.exists() {
            return Some(local_tools);
        }
    }

    // 3. 兜底使用全局用户目录 ~/.eaicoding/eagent-tools
    get_eaicoding_dir().map(|path| path.join("eagent-tools"))
}

fn dev_resource_path(relative: &str) -> Option<PathBuf> {
    find_local_root().and_then(|root| {
        // 直接从项目根目录查找 resources/eagent-tools
        let root_path = root.join("resources").join("eagent-tools").join(relative);
        if root_path.exists() {
            Some(root_path)
        } else {
            None
        }
    })
}

fn bundled_or_local_resource(relative: &str) -> Option<PathBuf> {
    resource_tools_root()
        .map(|root| root.join(relative))
        .filter(|path| path.exists())
        .or_else(|| dev_resource_path(relative))
}

pub fn unzip_eroot(zip_path: &Path, target_dir: &Path) -> Result<(), String> {
    let file =
        fs::File::open(zip_path).map_err(|err| format!("打开内置易语言压缩包失败：{}", err))?;
    let mut archive =
        ZipArchive::new(file).map_err(|err| format!("读取内置易语言压缩包失败：{}", err))?;

    fs::create_dir_all(target_dir).map_err(|err| format!("创建易语言运行目录失败：{}", err))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("读取压缩包条目失败：{}", err))?;
        let Some(safe_name) = entry.enclosed_name().map(|name| name.to_owned()) else {
            continue;
        };
        let out_path = target_dir.join(safe_name);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|err| format!("创建目录失败 {}：{}", out_path.display(), err))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("创建目录失败 {}：{}", parent.display(), err))?;
        }

        let mut output = fs::File::create(&out_path)
            .map_err(|err| format!("写入文件失败 {}：{}", out_path.display(), err))?;
        io::copy(&mut entry, &mut output)
            .map_err(|err| format!("解压文件失败 {}：{}", out_path.display(), err))?;
    }

    Ok(())
}

fn ensure_bundled_eroot() -> Option<PathBuf> {
    // 1. 优先使用 app_tools_root() 下的 eroot/e.exe
    let installed_root = app_tools_root()?.join("eroot");
    let installed_exe = installed_root.join("e.exe");
    if installed_exe.exists() {
        return Some(installed_root);
    }

    // 2. 若存在本地或全局的 e.zip，优先解压安装到 app_tools_root() 的 eroot 下
    if let Some(zip_path) = bundled_or_local_resource(r"eroot\e.zip") {
        if let Ok(()) = unzip_eroot(&zip_path, &installed_root) {
            if installed_exe.exists() {
                return Some(installed_root.clone());
            }
        }
    }

    // 3. 兜底：如果 app_tools_root() 是本地便携路径且解压失败，检查全局目录是否已经有 e.exe
    if let Some(eaicoding) = get_eaicoding_dir() {
        let global_eroot = eaicoding.join("eagent-tools").join("eroot");
        if global_eroot.join("e.exe").exists() {
            return Some(global_eroot);
        }
        
        // 尝试兜底解压到全局目录
        if let Some(zip_path) = bundled_or_local_resource(r"eroot\e.zip") {
            if let Ok(()) = unzip_eroot(&zip_path, &global_eroot) {
                if global_eroot.join("e.exe").exists() {
                    return Some(global_eroot);
                }
            }
        }
    }

    None
}

pub fn detect_eagent_tools() -> EAgentTools {
    let root = find_local_root();
    let bundled_root = resource_tools_root().or_else(|| {
        root.as_ref()
            .map(|project_root| project_root.join("resources").join("eagent-tools"))
    });

    let bundled_eparser_exe = bundled_or_local_resource(r"eparser32\eparser32.exe");
    let bundled_eparser_dll = bundled_or_local_resource(r"ecodeparser\ECodeParser.dll");
    let bundled_ebuild_exe = bundled_or_local_resource(r"ebuild\EBuild.exe");
    let bundled_e2txt_exe = bundled_or_local_resource(r"e2txt\e2txt.exe");
    let bundled_template_dir = bundled_or_local_resource(r"templates\console.ecode");
    let bundled_ecl_exe = bundled_or_local_resource(r"ecl\ecl.exe");
    let bundled_e_root = ensure_bundled_eroot();

    let ebuild_exe = bundled_ebuild_exe;
    let e2txt_exe = bundled_e2txt_exe;
    let ecode_template_dir = bundled_template_dir;
    let eparser_exe = bundled_eparser_exe;
    let eparser_dll = bundled_eparser_dll;
    let ecl_exe = bundled_ecl_exe;
    let e_root = bundled_e_root;
    let e_exe = e_root.as_ref().map(|path| path.join("e.exe"));
    let el_exe = e_root.as_ref().map(|path| path.join("el.exe"));
    let link_dll = e_root.as_ref().map(|path| path.join(r"tools\link.dll"));
    let vc_link_exe = e_root
        .as_ref()
        .map(|path| path.join(r"VC98linker\bin\link.exe"));
    let static_lib_dir = e_root.as_ref().map(|path| path.join("static_lib"));

    EAgentTools {
        project_root: root.map(|path| path.to_string_lossy().to_string()),
        bundled_tools_root: bundled_root.map(|path| path.to_string_lossy().to_string()),
        ebuild_exe: tool_check(ebuild_exe, Some("未找到 EBuild.exe".to_string())),
        e2txt_exe: tool_check(e2txt_exe, Some("未找到 e2txt.exe".to_string())),
        ecode_template_dir: tool_check(
            ecode_template_dir,
            Some("未找到内置文本工程模板".to_string()),
        ),
        eparser_exe: tool_check(eparser_exe, Some("未找到 eparser32.exe".to_string())),
        eparser_dll: tool_check(
            eparser_dll,
            Some("未找到可用的 ECodeParser.dll".to_string()),
        ),
        e_root: tool_check(e_root, Some("未找到易语言安装目录".to_string())),
        ecl_exe: tool_check(ecl_exe, Some("未找到 ecl.exe".to_string())),
        e_exe: tool_check(e_exe, Some("未找到 e.exe".to_string())),
        el_exe: tool_check(el_exe, Some("未找到 el.exe".to_string())),
        link_dll: tool_check(link_dll, Some("未找到 tools\\link.dll".to_string())),
        vc_link_exe: tool_check(
            vc_link_exe,
            Some("未找到 VC98linker\\bin\\link.exe".to_string()),
        ),
        static_lib_dir: tool_check(static_lib_dir, Some("未找到 static_lib".to_string())),
    }
}
