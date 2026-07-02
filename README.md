# EAiCoding Rust MCP 服务

> **Model Context Protocol (MCP) Server for EasyLanguage (EPL) Development**

EAiCoding Rust MCP 服务是一款面向**易语言 (EPL)** 开发者及 AI 客户端的本地大模型上下文协议 (Model Context Protocol) 插件。

本项目去掉了复杂的前端 UI 壳，将二进制的易语言文件处理、精易模块内置知识库检索、基于 Search-Replace 的局部修补、工程静态安全与排重诊断以及命令行自动编译验证封装为一个基于 **Stdio (JSON-RPC 2.0)** 传输的独立 Rust 后端服务。

这使得您可以在 **Cursor**, **Claude Desktop** 或 **VSCode** 等现代 AI 编辑器中一键对接本地易语言编译与开发链，让 AI 能够“看懂”二进制易语言代码，并可靠地在本地修改和编译。

---

## 🛠️ 功能特性

*   **一键环境装载 (`setup_env`)：** 自动下载官方 Release 发布的易语言编译器与命令行工具依赖包（`e.zip`），免去手动配置路径的繁琐。
*   **二进制解析与转换：** 解析 `.e`/`.ec` 文件的全局结构，或将其完美导出为 UTF-8 编码的 `ecode` 文本工程目录。
*   **增量修补（`patch_file`）：** 大模型通过 Search-Replace 语法在 Rust 侧对代码文件做增量 Patch 修改，极大节省上下文 Token 并保证修改鲁棒性。
*   **编译诊断回归（`compile_efile`）：** 自动化调用命令行编译器 `ecl.exe` + `e.exe` 进行回归，并拦截编译错误日志以供大模型纠错。
*   **精易模块检索（`search_jingyi_module`）：** 纯 Rust 实现的多路混合检索，可根据中文功能意图秒级定位并推荐最佳的 API 实现路径。
*   **并行工程诊断（`analyze_project`）：** 扫描整个文本工程，在 Rust 侧多线程查找硬编码 URL、不安全明文传输以及重复的代码逻辑，输出诊断报告。
*   **自适应编码读写：** 后台读写时自动在 **GBK** 和 **UTF-8** 之间进行无损双向转换，防止模型在编辑易语言文本源码时中文发生乱码。

---

## 🔌 配置与集成指南

### 1. 编译发布包

在已经安装了 Rust 语言开发环境的 Windows 机器上，转到项目根目录，执行 release 构建：

```powershell
# 使用自动化打包脚本编译并生成打包 ZIP 分发包
powershell -File .\package.ps1
```

编译成功后，生成的独立可执行文件和分发包将输出在：
📂 `release/eaicoding-mcp-portable/eaicoding-mcp.exe`
📂 `release/eaicoding-mcp-portable.zip`

---

### 2. 客户端集成配置

本服务支持 **Stdio (命令行进程)** 与 **SSE (Server-Sent Events HTTP)** 两种调用协议。推荐使用 **SSE** 协议，支持跨进程、跨局域网调用，且服务运行更稳定。

#### 2.1 SSE (HTTP) 服务运行与配置 (推荐)

在终端启动 MCP 服务：
```powershell
# 启动后默认在本地监听 8765 端口
.\release\eaicoding-mcp-portable\eaicoding-mcp.exe
```

##### 2.1.1 客户端使用 `.agents/mcp_config.json` 自动配置
本项目的 `.agents/mcp_config.json` 为 MCP 客户端定义了 SSE 的服务连接地址：
```json
{
  "mcpServers": {
    "eaicoding-mcp": {
      "url": "http://127.0.0.1:8765/sse"
    }
  }
}
```
如果您使用的是支持自动加载 `.agents/` 目录的 Agent 工具（如 Antigravity / Gemini Code Assist 插件等），它会自动读取并接入该 SSE 服务。

##### 2.1.2 客户端手动配置 (如 Cursor / Cline)
1. 打开 Cursor 的 **Settings** -> **Features** -> **MCP**。
2. 点击 **+ Add New MCP Server**。
3. 填写以下信息：
   * **Name**: `eaicoding-mcp`
   * **Type**: `SSE`
   * **URL**: `http://127.0.0.1:8765/sse`
4. 保存即可。

---

#### 2.2 Stdio (进程拉起) 客户端配置

如果您不需要保持后台服务常驻，而是由 AI 客户端每次自动拉起进程，可以使用 Stdio 模式配置：

##### 2.2.1 客户端配置：Claude Desktop
打开 `Claude Desktop` 的配置文件（通常在 `%APPDATA%\Claude\claude_desktop_config.json`），添加如下项：
```json
{
  "mcpServers": {
    "eaicoding-mcp": {
      "command": "C:\\path\\to\\your\\eaicoding-mcp.exe",
      "args": []
    }
  }
}
```
*（请将 `C:\\path\\to\\your\\eaicoding-mcp.exe` 替换为您本地存放该 `.exe` 文件的实际绝对路径，注意转义反斜杠 `\\`）*。

##### 2.2.2 客户端配置：Cursor (Stdio 模式)
1. 打开 Cursor **Settings** -> **Features** -> **MCP** -> **+ Add New MCP Server**。
2. 填写配置：
   * **Name:** `eaicoding`
   * **Type:** `stdio`
   * **Command:** `C:\path\to\your\eaicoding-mcp.exe`
3. 保存即可。

---

## 💡 技能 (Skills) 运行机制与易语言运维规范

项目内置了针对 AI 开发助手的技能指南，存放在 [SKILL.md](file:///C:/Users/whaty/Desktop/eMcp/.agents/skills/eaicoding_mcp_development/SKILL.md)。当 AI 助手（如 Antigravity）在面对本工程的开发、修改以及易语言项目维护任务时，会**自动激活该技能**。

### 1. 技能核心规范

*   **平铺文本双重追踪 (plain-text Git tracking)**：易语言二进制文件 `.e`/`.ec` 无法进行 Diff 和 Merge。技能规范要求：每次修改完代码并保存后，**必须**运行 `export_ecode` 导出为 `ecode` 文本工程目录（`ecode_output/[项目名]/`）并提交至 Git，以便进行 Code Review 和分支合并。
*   **一键“修改-回编-编译”闭环**：
    1. 使用 `patch_file` 差分修补文本源码；
    2. 调用 `generate_efile` 反向回编生成二进制 `.e` 文件；
    3. 调用 `compile_efile` 一键拉起易语言命令行编译器，检查是否存在编译错误，直至通过。
*   **高保真迁移映射**：技能包中提供了一套完善的易语言常用函数/组件与现代语言（Python/Go/Rust）的**迁移映射表**。涵盖 UI（PyQt/Slint/Tauri）、HTTP请求、并发多线程（许可证/Mutex）、INI配置读写、数据库（SQLite）以及 V8/JS 引擎的无损迁移方案。
*   **工程静态诊断**：利用 `analyze_project` 扫描源码中硬编码的 URL、敏感字段（如 token、密码）和不安全明文传输，并输出诊断报告保存至 `docs/` 目录中。

---

## 📂 目录结构

```text
eaicoding-mcp/
  .agents/
    mcp_config.json            MCP 客户端 SSE 连接配置文件
    skills/
      eaicoding_mcp_development/
        SKILL.md               项目开发与易语言逆向迁移技能指南
  src/                         Rust 源码
    main.rs                    MCP 消息路由与 SSE HTTP 服务器入口
    lib.rs                     项目底层库声明
    eagent_tools.rs            编译链环境检测与自包含管理
    ecode_parser.rs            易语言二进制与文本转换核心
    jingyi_search.rs           精易模块多路召回知识库
    patch.rs                   Search-Replace 局部修补引擎
    analyze.rs                 并行项目诊断引擎
    local_files.rs             GBK <-> UTF-8 读写转换
    easy_language_sdk.rs       易语言安装包环境扫描
  resources/eagent-tools/      随应用打包的易语言工具链 (包含 ecl, e2txt 等)
  package.ps1                  自动化打包脚本
  Cargo.toml                   Cargo 项目配置文件
  README.md                    本项目说明文档
```

---

## 📦 MCP Tools 接口定义与入参说明

MCP 服务端启动后，会向客户端汇报以下 Tool 接口：

### 1. `inspect_env`
*   **用途：** 检查本地易语言工具链环境。
*   **入参：** 无

### 2. `setup_env`
*   **用途：** 一键下载 Release 包 `e.zip` 并提取到本地 `%USERPROFILE%\.eaicoding\eagent-tools` 目录中。
*   **入参：** 无

### 3. `parse_efile`
*   **用途：** 提取 `.e`/`.ec` 工程文件内部的公开 API 和程序集概况。
*   **入参：**
    *   `file_path` (string, 必须): 二进制源文件的绝对路径。

### 4. `export_ecode`
*   **用途：** 将二进制的 `.e` 文件导出为方便 LLM 编辑的 `ecode` 文本工程目录。
*   **入参：**
    *   `source_path` (string, 必须): 二进制源文件路径。
    *   `output_dir` (string, 可选): 导出的文本工程输出目录。默认在 `%USERPROFILE%\.eaicoding\ecode` 下。

### 5. `generate_efile`
*   **用途：** 将文本形式的 `ecode` 工程目录重新回编生成二进制的 `.e` 文件。
*   **入参：**
    *   `ecode_dir` (string, 必须): 文本工程目录路径。
    *   `output_path` (string, 可选): 生成的 `.e` 二进制文件绝对路径。

### 6. `patch_file`
*   **用途：** 大模型局部修改源码的最优方式。接收 search-replace 格式的差异块对目标文件进行原子的替换。
*   **入参：**
    *   `file_path` (string, 必须): 目标文件绝对路径。
    *   `patch` (string, 必须): 包含 `<<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE` 的代码块。
*   **补丁格式示例：**
    ```text
    <<<<<<< SEARCH
    信息框 (“原始内容”, 0, , )
    =======
    信息框 (“修改后的新内容”, 0, , )
    >>>>>>> REPLACE
    ```

### 7. `compile_efile`
*   **用途：** 自动化命令行编译并回归，拦截编译错误日志。
*   **入参：**
    *   `source_path` (string, 必须): 二进制 `.e` 源文件绝对路径。
    *   `output_path` (string, 可选): 编译生成的 `.exe` 可执行文件绝对路径。
    *   `static_link` (boolean, 可选): 是否静态链接，默认 `true`。
    *   `module_paths` (array[string], 可选): 编译所依赖的 `.ec` 外部模块绝对路径列表。

### 8. `search_jingyi_module`
*   **用途：** 检索内置的精易模块知识库，支持高级打分与 Reranker。
*   **入参：**
    *   `query` (string, 必须): 检索关键字或中文需求描述。
    *   `limit` (integer, 可选): 返回的最佳 API 匹配结果限制条数，默认 8。

### 9. `analyze_project`
*   **用途：** 静态扫描与重复逻辑排查。
*   **入参：**
    *   `ecode_dir` (string, 必须): 文本工程目录的绝对路径。

### 10. `read_ecode_file`
*   **用途：** 读取源码文件，后台自适应处理 GBK -> UTF-8 解码，防乱码。
*   **入参：**
    *   `file_path` (string, 必须): 目标文件绝对路径。
    *   `max_chars` (integer, 可选): 字符截断上限，默认 12000。

### 11. `write_ecode_file`
*   **用途：** 覆盖写入源码文件，自动处理 UTF-8 -> GBK 转码和 Windows CRLF 换行规范。
*   **入参：**
    *   `file_path` (string, 必须): 目标文件绝对路径。
    *   `content` (string, 必须): 写入的源码内容。

---

## 🔎 技术要点解析

### 1. 字符集转码 (GBK <-> UTF-8)
由于大模型客户端只能理解 `UTF-8` 编码，而易语言编译器和支持库原生仅支持 Windows 中文系统下的 `GBK` 字符集。如果在写入或读取时不进行正确转码，生成的中文代码中包含的字符串（如 `信息框 (“你好”, 0, , )`）在编译后会产生乱码甚至导致编译器崩溃。本服务在底层读写时引入 `encoding_rs` 对数据流做原子的、双向的 GBK 编码解析与格式化。

### 2. 命令行编译常见错误排查
*   **提示找不到 `VC98linker\bin\link.exe`：**
    说明未正确配置编译器的连接器环境。请调用 `setup_env` 工具以确保本地数据目录中已解压了完整的易语言编译器包。
*   **提示未引用的支持库/模块：**
    由于命令行编译器不会自动载入未与源文件一同打包或在参数中声明的 `.ec` 文件，请确保在调用 `compile_efile` 时，通过 `module_paths` 传入所有缺失的模块的绝对路径。
*   **静态链接失败：**
    部分易语言支持库（如 `.fne`）由于历史遗留原因不支持静态编译为 `.obj` 格式。请尝试在调用 `compile_efile` 时，将 `static_link` 参数设置为 `false`（使用独立编译生成带外部 dll/支持库引用的 exe 文件）。
