---
name: eaicoding-mcp-development
description: 维护、开发、扩展、调试与打包测试 EAiCoding 易语言 MCP 服务（eaicoding-mcp.exe）。当需要修改 JSON-RPC 协议、增加新的 MCP Tools、调试易语言文本工程解析与回编机制时，激活此 Skill。
---

# EAiCoding MCP 服务开发与维护指南 (Skill)

本 Skill 指导 AI 开发助手如何正确维护、扩展和调试 **eaicoding-mcp** Rust MCP 服务。服务目前已完全重构为标准的 **SSE (Server-Sent Events) HTTP** 协议规范，便于进行本地或跨电脑远程/局域网调用。

---

## 0. 项目文件结构快速参考

```
789/                              <- 项目根目录
├── src/
│   ├── main.rs                   <- MCP 入口：HTTP SSE 服务路由 + Tool Schema + 异步派发
│   ├── lib.rs                    <- 模块声明（pub mod 列表）
│   ├── eagent_tools.rs           <- 工具链路径检测 + 自动安装逻辑
│   ├── ecode_parser.rs           <- .e/.ec 解析、export_ecode、generate_efile、compile_efile
│   ├── easy_language_sdk.rs      <- 易语言安装目录扫描（scan_env）
│   ├── jingyi_search.rs          <- 精易模块 API 语义检索
│   ├── local_files.rs            <- GBK<->UTF-8 文件读写（read/write_ecode_file）
│   ├── patch.rs                  <- Search-Replace 差分修补（patch_file）
│   └── analyze.rs                <- 静态诊断分析（analyze_project）
├── resources/
│   └── eagent-tools/             <- 本地工具链（EBuild/e2txt/ecl/eparser32/ECodeParser/templates）
├── data/
│   └── jingyi-raw.json           <- 精易模块 API 原始数据（jingyi_search.rs 的数据源）
├── docs/                         <- 项目文档
├── .agents/
│   ├── mcp_config.json           <- MCP 客户端 SSE 连接配置
│   └── skills/eaicoding_mcp_development/SKILL.md
├── Cargo.toml                    <- 项目版本、依赖管理
├── build.rs                      <- 空占位构建脚本
└── package.ps1                   <- 一键编译+打包+SHA256 校验脚本
```

---

## 1. 核心开发规范与原则

### 1.1 HTTP 与 SSE 通信规则（极度重要）

*   **标准 SSE 握手：** 客户端请求 `GET /sse` 以维持 Event-Stream。服务端必须在连接建立后，向流中立刻写入第一个 `endpoint` 事件，通知用于交互的 POST 地址：
    ```
    event: endpoint
    data: /message?connectionId=conn_xxx\n\n
    ```
*   **异步 RPC 处理：** 客户端将 JSON-RPC 推送到 `POST /message?connectionId=conn_xxx`。服务端处理该 POST 请求应立即响应 `202 Accepted`，并在后台使用 `tokio::spawn` 异步并发运行 Tool，执行结束后将 JSON-RPC 响应结果通过先前和该 `connectionId` 绑定的 SSE 长连接推送回客户端。
*   **跨域资源共享 (CORS)：** 本服务需要支持跨电脑或本地网页插件跨域调用，接口必须对 OPTIONS 预检请求返回 204，并且在响应中携带 CORS 首部：
    ```
    Access-Control-Allow-Origin: *
    Access-Control-Allow-Methods: GET, POST, OPTIONS
    Access-Control-Allow-Headers: Content-Type, X-File-Name
    ```

### 1.2 GBK 与 UTF-8 的双向转码

易语言源码（.e.txt）在本地以 GBK 编码保存，LLM 与 JSON-RPC 只支持 UTF-8。
*   读取文件：使用 `read_text_file_for_agent`（local_files.rs）自动探测编码，GBK 自动转 UTF-8 后返回。
*   写入文件：使用 `write_text_file`（local_files.rs），第三个参数传 `Some("gbk".to_string())`，自动将 UTF-8 转 GBK 写入。

### 1.3 换行符 CRLF 规范

易语言编译器（e.exe/ecl.exe）强依赖 Windows CRLF（`\r\n`）。写入源码文件时必须调用 `normalize_line_endings_for_path` 统一为 `\r\n`。

---

## 2. 扩展或修改一个 MCP Tool 的具体步骤

新增工具必须同步修改以下三处：

### 第一步：在 get_mcp_tools_schema() 中注册 Schema

在 `src/main.rs` 的 `get_mcp_tools_schema()` 末尾追加：
```json
{
    "name": "your_tool_name",
    "description": "清晰描述工具用途，大模型依此进行语义路由",
    "inputSchema": {
        "type": "object",
        "properties": {
            "param_1": { "type": "string", "description": "参数说明" }
        },
        "required": ["param_1"]
    }
}
```

### 第二步：在 execute_tool() 中添加路由分支

在 `src/main.rs` 的 `execute_tool()` match 中添加：
```rust
"your_tool_name" => {
    let param_1 = args.get("param_1")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "参数缺失: param_1".to_string())?;
    let result = your_module::your_function(param_1.to_string()).await?;
    Ok(result)
}
```

### 第三步：在子模块中实现业务逻辑

在 `src/` 对应 `.rs` 中编写逻辑，注意：
*   不引入 Tauri 相关依赖。
*   日志及调试信息可以使用 `eprintln!` 输出至终端。
*   新增模块须在 `src/lib.rs` 声明 `pub mod`。

---

## 3. 测试与调试指南

### 3.1 本地编译与运行

```powershell
# 编译
cargo build

# ## 8. 易语言（.e）项目极速、稳健逆向分析与迁移架构规范

本指南详述如何使用 `eaicoding-mcp` 对易语言二进制程序（如 `.e` / `.ec` 文件）进行高速度、高强度的逆向解构、静态安全审查、功能结构拆解，以及生成用于跨语言复刻（Python/Go/Rust等）的保真设计蓝图。

---

### 8.1 性能与分析速度优化 (极速分析)

在处理大型易语言程序（例如总子程序数 >3000）时，反编译导出为文本工程的 IO 开销与分析开销会显著增长。必须落实以下提速规范：

1. **导出增量缓存检查 (Cache Check)**
   * **原理**：二进制 `.e` 反编译导出文本工程（`export_ecode`）耗时通常达 5s-15s。若二进制文件未发生修改，应直接复用已有的导出代码。
   * **落地方案**：分析流水线脚本应首先对比源 `.e` 文件的修改时间（mtime）或 SHA256 哈希值与 `ecode_output/<项目名>/` 目录的生成记录。若匹配则**自动跳过反编译步骤**，直接进入静态分析，从而让暖启动分析耗时降至 1s 以内。

2. **全量源码并行检索 (Parallel Scan)**
   * **原理**：易语言导出后的文本工程包含成百上千个 `.e.txt` 模块。单线程串行读取匹配性能瓶颈明显。
   * **落地方案**：在 Python 审计脚本中，涉及对 `代码/*.e.txt` 全量文本的特征正则挖掘（如 URL 过滤、敏感词搜索）时，使用多线程（如 `concurrent.futures.ThreadPoolExecutor`）进行并发文件读取与正则匹配，以压榨 CPU 多核性能。

3. **暖机连接与状态持久化**
   * **落地方案**：复用已经启动的 `eaicoding-mcp.exe` SSE HTTP 服务，避免每次分析都重新拉起新进程。在调用 MCP 前，通过检测 `8765` 端口的响应状态（发送轻量请求如 `inspect_env`）进行健康检查。

---

### 8.2 稳定性与异常容错机制 (稳健运行)

易语言工程在编码、表单配置导出过程中存在诸多与现代工具链不兼容的异构设计，需要执行以下稳定性加固策略：

1. **UTF-8 BOM (`utf-8-sig`) 严格兼容**
   * **避坑要点**：反编译工具生成的窗体布局文件 `窗口/*.form.json` 带有 UTF-8 字节顺序标记 (BOM, 字节头为 `\xef\xbb\xbf`)。若在 Python 中直接使用普通 `open(..., encoding='utf-8')` 并调用 `json.load()`，会直接触发 `JSONDecodeError` 崩溃。
   * **标准实现**：打开所有导出的 JSON 配置文件时，**必须**使用 `encoding='utf-8-sig'`。

2. **GBK 宽松转码安全网 (Transcoding Fallback)**
   * **避坑要点**：易语言原版代码保存为 GBK 格式。当部分源码包含非标准的中文标点、特殊字符或经过混淆的坏字符时，转码 UTF-8 容易引发 `UnicodeDecodeError` 导致分析中断。
   * **标准实现**：在 Python 扫描工具或 Rust 接口层中，读取文本文件时均须开启容错模式。Python 例：`open(file, 'r', encoding='utf-8', errors='ignore')` 或 `errors='replace'`。

3. **Windows 路径深度转义与命令行沙箱绕过**
   * **避坑要点**：Windows 下的路径分隔符为 `\`。在命令行中拼装 JSON 参数并调用 CLI 时，极易因未正确转义（如 `\u` 误认为 unicode 字符，或空格路径截断）引发解析失败。
   * **标准实现**：
     * 代码内部逻辑一律将 Windows 路径的 `\` 统一替换为 `/`，或者在拼接 JSON 前使用 `json.dumps()` 自动对路径进行标准 JSON 转义。
     * 严禁直接通过终端拼接 naked 字符串。必须像 [call_mcp.py](file:///C:/Users/whaty/.gemini/antigravity-cli/brain/ed331fd7-f318-4aab-bd8c-5e8984b0c9cf/scratch/call_mcp.py) 那样采用 HTTP SSE JSON-RPC 协议标准对象发送。

---

### 8.3 全方位、细致的审计提取规范 (生成准确细致报告)

为确保生成的 `analysis_report.md` 具有极高的迁移实用性，报告必须能够**自底向上**还原整个程序的物理世界模型：

1. **界面交互与核心事件关联树 (UI & Event Correlation)**
   * **窗体控件提取**：从 `*.form.json` 中完整遍历并列出关键的控件（按钮 `按钮`、表格 `超级列表框`/`高级表格`、输入框 `编辑框`、单选/复选框、选择夹 `选择夹` 等）的名称、类型与在表单中的几何定位。
   * **事件句柄绑定追踪**：查阅与之配套的窗体代码 `*.form.e.txt`，匹配如 `_按钮_登录_被单击`、`_高级表格_数据被改变` 等事件处理子程序。将控件的物理展示与它对应的业务入口代码紧密关联，构建 UI 动作驱动逻辑图谱。

2. **程序全局状态机与配置流审计 (Global State & Config)**
   * **全局状态捕捉**：解析 `全局变量.e.txt`，重点挖掘存储 Session、Token、当前用户、运行状态（如 `运行线程数`、`是否停止`、`打标列表`）的全局标识。
   * **INI 配置持久化分析**：在源码中检索 `读配置项` 与 `写配置项` 命令，提取对应的配置文件路径（通常是 `.ini` 格式）、小节名 (Section, 如 `users`)、键名 (Key, 如 `login_password`) 以及缺省参数。在新项目中以此重新构建健壮的配置管理类。

3. **多线程并发安全与许可证追踪 (Concurrency & Mutex Locks)**
   * **多线程调用审计**：检索 `启动线程` 命令，找出传入的子程序指针（线程执行体）和传递的参数。
   * **临界区安全同步**：检索并提取 `创建进入许可证`、`进入许可`、`退出许可`、`删除进入许可证` 的使用逻辑。分析程序在访问共享资源（如全局列表、多线程写日志）时所采用的加锁范围与锁变量，并映射为异构语言的同步设计。

4. **本地持久化与数据库模型扫描 (SQLite & Db Schema)**
   * **数据表生命周期**：检查是否存在 SQLite 相关的库支持，定位 `打开数据库`、`打开` 操作。
   * **SQL 提取**：检索 `执行SQL` 等指令，查找嵌入其中的 `CREATE TABLE`、`INSERT`、`SELECT` 等 SQL 模板，准确还原本地 SQLite3 的表结构、索引与字段定义。

5. **嵌入式 JS 加密与外挂计算引擎 (Embedded JS/Signature Logic)**
   * **JS 计算定位**：易语言自身在进行大型平台（如京东、淘宝）的请求时，常将涉及签名（Sign）、RSA 密文计算的 JS 脚本存放于文本常量中（如 `#常量_sign_js`）。
   * **调用路径还原**：追踪调用 `类_脚本组件` / `类_V8` 或 `执行脚本` 的核心逻辑，指出 JS 引擎加载的文本常量名称，并还原传参方式（如 `V8.运行 ("getSign", param)`）。

---

### 8.4 异构语言高保真迁移技术映射表 (Cross-Language Equivalents)

在复刻与迁移易语言工程时，建议对照以下核心功能的技术栈映射清单进行异构语言重构：

| 功能模块 | 易语言原版命令 / 常用模块 | Python 迁移推荐 | Go 迁移推荐 | Rust 迁移推荐 |
| :--- | :--- | :--- | :--- | :--- |
| **GUI 窗口界面** | Windows 窗口、精易皮肤模块 | `PyQt6` / `PySide6` (支持 QML/动态交互); 网页端 `React` / `Vue` | `Fyne` / `walk` (Windows 原生); 网页端前后端分离 | `Tauri` (Web 技术栈界面); `slint` / `egui` (轻量原生) |
| **HTTP 请求** | `网页_访问_对象`、`网页_访问` (精易模块/WinHttp) | `httpx.Client()` (支持异步异步调用); `requests.Session()` | `go-resty/resty` (链式调用); 原生 `net/http` | `reqwest::Client` (强大的异步底层 HTTP 客户端) |
| **并发与多线程** | `启动线程`、精易线程池 | `threading.Thread`、`concurrent.futures.ThreadPoolExecutor` | `go func()` (协程机制，比线程轻量数倍) | `tokio::spawn` (异步 Runtime 执行体) |
| **线程锁 / 许可证**| `创建进入许可证`、`进入许可`、`退出许可` | `threading.Lock` / `asyncio.Lock` | `sync.Mutex` / `sync.RWMutex` | `std::sync::Mutex` / `tokio::sync::Mutex` |
| **INI 配置读写** | `读配置项`、`写配置项` | `configparser` 库 | `go-ini/ini` 库 | `rust-ini` / `toml` 库 (推荐升级为更安全的 TOML) |
| **本地 SQLite 库** | `Sqlite数据库` 类、`Sqlite表` 类 | `sqlite3` 内置模块 | `modernc.org/sqlite` (纯 Go 实现); `github.com/mattn/go-sqlite3` | `rusqlite` (同步); `sqlx` (异步生态) |
| **嵌入式 JS 计算** | `类_脚本组件`、`类_V8` (精易模块) | `quickjs` (极速轻量); `PyExecJS` | `quickjs-go` (绑定原生 QuickJS); `robertkrimen/otto` (纯 Go) | `quickjs` / `deno_core` (集成 V8 引擎) |
| **编码与字符串** | GBK 字符集、`编码_Utf8到Ansi`、`文本_取出中间文本` | 默认 `UTF-8` 生态; 用正则或 `str.split` 替代“取中间” | 默认 `UTF-8` 生态; 用正则或内置 string 切片处理 | 默认 `UTF-8` 生态; 使用 `regex` 包或模式匹配安全切割 |

---

### 8.5 项目审计与迁移报告输出模板规范

> [!IMPORTANT]
> 所有的逆向审计报告、迁移设计书、架构拆解蓝图等，**必须统一输出并保存至项目根目录下的 `docs/` 目录中**（例如 `docs/analysis_report.md`，或者以项目命名的 `docs/<项目名>_analysis.md`），严禁散落保存在根目录、临时文件夹或桌面等其他非标准位置，以保障项目开发文档的集中管理和团队 Git 版本追踪。


导出的 `docs/analysis_report.md` 报告应始终包含以下结构，帮助开发人员一目了然地吃透项目结构：

```markdown
# 易语言程序 `[程序名称]` 逆向解构与跨语言迁移蓝图

## 一、 项目总体架构与规模度量
* 包含项目基本信息：源程序文件数、子程序总数、硬编码 URL 数量、敏感字段变量数量统计。
* 依赖的支持库与外部模块（如 精易模块、SQLite3支持库等）。

## 二、 窗口 UI 控件布局与事件流逆向映射
* 遍历所有的主窗口，以表格/列表输出控件名、类型、物理坐标。
* 明确列出每一个控件与其绑定的事件子程序（如 `_按钮_登录_被单击`），并贴出不超过25行的核心关键中文逻辑（Ansi到Utf8转码呈现）。

## 三、 硬编码 URL 与 API 通讯边界
* 分组呈现每一个 URL 的调用位置（文件名、行号）与发起网络访问时的参数序列化结构（协议头、Body 字段）。

## 四、 全局状态机、持久化配置与本地数据库
* 全局变量清单及其承担的全局状态属性。
* 读写配置项（INI 文件）的 Section、Key 定义以及对应的缺省值表。
* 本地 SQLite 数据表结构还原（表名、字段类型、关联查询逻辑）。

## 五、 并发同步锁与关键安全防护
* 线程创建入口与共享资源保护许可证锁（Mutex）的逻辑分布。
* 安全限制分析（如设定 DEP 保护、窗口防多开等）。
* JS 算法解密常量还原（说明加密 JS 代码在常量表中的标识与引擎调用点）。

## 六、 异构语言迁移与高保真复刻指南
* 针对该项目的业务特点，详细输出迁移到新语言的模块化解耦建议（例如：如何将易语言中耦合在按钮单击事件中的网络请求与 UI 逻辑剥离开，重构为标准的三层架构/MVC模式）。
* 提供各模块在新语言中的最佳第三方框架组合及核心代码转换逻辑。

## 七、 项目日常维护、版本管理与一键构建指南
* 明确 plain-text 双重追踪规范（ecode 导出目录作为 Git 核心跟踪）。
* 记录“编辑 -> generate_efile -> compile_efile”的一键构建流水线操作。
* 列出外部依赖的 `.ec` 模块以及对应的第三方依赖项。
```

---

### 8.6 易语言（.e）项目版本管理、日常维护与自动化构建规范

除了向新语言的迁移外，保留和维护现有易语言项目同样至关重要。必须遵循以下日常维护与自动化构建规范：

1. **二进制与文本的双重版本控制 (Git & Collaboration)**
   * **痛点**：易语言二进制 `.e` 文件无法在 Git 中进行 Diff 或 Merge，团队协作时极易产生代码冲突且无法进行代码评审（Code Review）。
   * **维护规范**：
     * **代码审查与追踪**：每次修改代码并保存后，**必须**运行 `export_ecode` 将工程导出为 plain-text 文本工程（`ecode_output/<项目名>/`）并提交至 Git。所有的分支合并、比对与 Code Review 均在 `ecode_output` 目录下进行。
     * **Git 忽略项**：为了防止冲突，只在 Git 中追踪导出的文本工程目录。`.e` 二进制文件可以作为阶段性产物忽略，或仅在发布 Tag 时进行归档。

2. **一键式“修改 - 回编 - 编译”闭环 (Continuous Integration / Daily Builds)**
   * **日常维护流程**：
     * **局部修补**：对导出的文本源码文件（`.e.txt`）使用 `patch_file` 补丁工具或 `write_ecode_file` 接口进行快速的局部 Search-Replace 修改，防止因文件过大引发的网络超时。
     * **重新回编**：修改完成后，调用 `generate_efile`，将 `ecode_output/<项目名>` 目录反向生成为对应的二进制 `.e` 文件。
     * **一键编译**：调用 `compile_efile`，自动拉起易语言命令行编译器，将生成的 `.e` 二进制编译为最终的成品 `.exe`。
   * **自动化流水线**：建议将该闭环整理成一键构建脚本（如 `build.ps1`），在开发调试时进行一键回编与测试，确保 plain-text 的任何修改都能正确回编且无编译错误。

3. **模块化依赖 (`.ec` / `.fne`) 的规范管理**
   * **维护规范**：
     * 将项目所有依赖的第三方模块（如 `精易模块[v11.1.5].ec`）集中存放在项目根目录下的 `libs/` 或 `modules/` 文件夹中，严禁分散在系统各处。
     * 编译时，必须在 `compile_efile` 的 `module_paths` 参数中显式传入这些模块的本地路径，保证编译环境的隔离性与一致性。
