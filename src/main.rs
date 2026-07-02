use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use std::sync::Arc;

use base64::Engine;
use eaicoding_mcp::eagent_tools::{detect_eagent_tools, app_tools_root, get_eaicoding_dir, unzip_eroot, ToolCheck};
use eaicoding_mcp::ecode_parser::{parse_efile, export_efile_to_ecode, generate_efile_from_ecode, compile_efile};
use eaicoding_mcp::easy_language_sdk::scan_easy_language_env;
use eaicoding_mcp::jingyi_search::search_jingyi_module_rust;
use eaicoding_mcp::local_files::{read_text_file_for_agent, write_text_file};
use eaicoding_mcp::patch::apply_patch_file;
use eaicoding_mcp::analyze::analyze_project_rust;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    id: Option<Value>,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

type SseConnections = Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>;

#[tokio::main]
async fn main() {
    print_startup_banner();
    initialize_environment().await;

    let bind_addr = env::var("EAICODING_MCP_BIND")
        .or_else(|_| env::var("MCP_HTTP_BIND"))
        .unwrap_or_else(|_| "0.0.0.0:8765".to_string());

    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("HTTP 服务监听失败 {}: {}", bind_addr, err);
            std::process::exit(1);
        }
    };

    let sse_connections: SseConnections = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    eprintln!("EAiCoding MCP SSE HTTP 服务已启动: http://{}", bind_addr);
    eprintln!("SSE 连接端点: GET /sse");
    eprintln!("上传 .e 文件: POST /upload");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let conns = sse_connections.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_http_connection(stream, peer.to_string(), conns).await {
                        eprintln!("HTTP 请求处理失败 {}: {}", peer, err);
                    }
                });
            }
            Err(err) => eprintln!("接收 HTTP 连接失败: {}", err),
        }
    }
}

fn print_startup_banner() {
    eprintln!("EAiCoding MCP 服务 v{} 正在启动...", env!("CARGO_PKG_VERSION"));
}

async fn initialize_environment() {
    eprintln!("[环境检测] 开始扫描易语言工具链组件...");
    let detected = detect_eagent_tools();

    print_env_status("EBuild.exe",      &detected.ebuild_exe);
    print_env_status("e2txt.exe",       &detected.e2txt_exe);
    print_env_status("ecl.exe",         &detected.ecl_exe);
    print_env_status("eparser32.exe",   &detected.eparser_exe);
    print_env_status("ECodeParser.dll", &detected.eparser_dll);
    print_env_status("ecode模板目录",    &detected.ecode_template_dir);
    print_env_status("e.exe",           &detected.e_exe);
    print_env_status("el.exe",          &detected.el_exe);
    print_env_status("link.dll",        &detected.link_dll);
    print_env_status("VC98 link.exe",   &detected.vc_link_exe);
    print_env_status("static_lib/",     &detected.static_lib_dir);

    if !detected.e_exe.exists {
        eprintln!("[自动安装] 检测到 e.exe 缺失，开始自动安装...");
        match setup_env_tool_impl().await {
            Ok(msg) => eprintln!("[自动安装] 安装成功: {}", msg),
            Err(err) => eprintln!("[自动安装] 安装失败，部分功能可能受限: {}", err),
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn handle_http_connection(mut stream: TcpStream, peer: String, sse_connections: SseConnections) -> Result<(), String> {
    let request = read_http_request(&mut stream).await?;
    eprintln!(
        "[HTTP] {} {} from {} body={} bytes",
        request.method,
        request.path,
        peer,
        request.body.len()
    );

    if request.method == "OPTIONS" {
        let response = http_empty(204);
        stream.write_all(response.as_bytes()).await.map_err(|err| format!("写入 HTTP OPTIONS 响应失败: {}", err))?;
        return Ok(());
    }

    let path_part = request.path.split('?').next().unwrap_or("/");
    if request.method == "GET" && path_part == "/sse" {
        handle_sse_connection(stream, peer, sse_connections).await?;
    } else {
        let response = route_http_request(request, sse_connections).await;
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|err| format!("写入 HTTP 响应失败: {}", err))?;
        eprintln!("[HTTP] response sent to {}", peer);
    }
    Ok(())
}

async fn handle_sse_connection(mut stream: TcpStream, peer: String, sse_connections: SseConnections) -> Result<(), String> {
    let connection_id = format!(
        "conn_{}_{}",
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        rand::random::<u32>()
    );

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    {
        let mut conns = sse_connections.lock().await;
        conns.insert(connection_id.clone(), tx);
    }
    eprintln!("[SSE] 客户端已连接. Peer: {}, ConnectionID: {}", peer, connection_id);

    let headers = "HTTP/1.1 200 OK\r\n\
                   Content-Type: text/event-stream\r\n\
                   Cache-Control: no-cache\r\n\
                   Connection: keep-alive\r\n\
                   Access-Control-Allow-Origin: *\r\n\
                   Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                   Access-Control-Allow-Headers: Content-Type, X-File-Name\r\n\
                   \r\n";
    if let Err(err) = stream.write_all(headers.as_bytes()).await {
        let mut conns = sse_connections.lock().await;
        conns.remove(&connection_id);
        return Err(format!("写入 SSE 响应头失败 Peer {}: {}", peer, err));
    }

    let endpoint_event = format!("event: endpoint\ndata: /message?connectionId={}\n\n", connection_id);
    if let Err(err) = stream.write_all(endpoint_event.as_bytes()).await {
        let mut conns = sse_connections.lock().await;
        conns.remove(&connection_id);
        return Err(format!("写入 endpoint 事件失败 Peer {}: {}", peer, err));
    }

    let mut check_buf = [0u8; 1];
    loop {
        tokio::select! {
            msg_opt = rx.recv() => {
                match msg_opt {
                    Some(msg) => {
                        let sse_data = format!("data: {}\n\n", msg);
                        if let Err(err) = stream.write_all(sse_data.as_bytes()).await {
                            eprintln!("[SSE] 写入消息失败 Peer {}: {}. 断开连接。", peer, err);
                            break;
                        }
                    }
                    None => break,
                }
            }
            read_res = stream.read(&mut check_buf) => {
                match read_res {
                    Ok(0) => {
                        eprintln!("[SSE] 客户端主动关闭了连接. Peer: {}, ConnectionID: {}", peer, connection_id);
                        break;
                    }
                    Err(err) => {
                        eprintln!("[SSE] 读取连接检测失败 Peer {}: {}. 断开连接。", peer, err);
                        break;
                    }
                    Ok(_) => {}
                }
            }
        }
    }

    {
        let mut conns = sse_connections.lock().await;
        conns.remove(&connection_id);
    }
    eprintln!("[SSE] 连接已释放. Peer: {}, ConnectionID: {}", peer, connection_id);
    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end;

    loop {
        let read = match tokio::time::timeout(std::time::Duration::from_secs(15), stream.read(&mut chunk)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(format!("读取 HTTP 请求失败: {}", e)),
            Err(_) => return Err("读取 HTTP 请求超时".to_string()),
        };
        if read == 0 {
            return Err("连接已关闭".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = find_header_end(&buffer) {
            header_end = pos;
            break;
        }
        if buffer.len() > 1024 * 1024 {
            return Err("HTTP 请求头过大".to_string());
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| "缺少请求行".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    if content_length > 0 {
        buffer.reserve(content_length);
    }
    while buffer.len().saturating_sub(body_start) < content_length {
        let read = match tokio::time::timeout(std::time::Duration::from_secs(30), stream.read(&mut chunk)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(format!("读取 HTTP body 失败: {}", e)),
            Err(_) => return Err("读取 HTTP body 超时".to_string()),
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let available = buffer.len().saturating_sub(body_start);
    let body_len = available.min(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: buffer[body_start..body_start + body_len].to_vec(),
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|item| item == b"\r\n\r\n")
}

async fn route_http_request(request: HttpRequest, sse_connections: SseConnections) -> String {
    let path = request.path.split('?').next().unwrap_or("/");
    match (request.method.as_str(), path) {
        ("GET", "/health") => http_json(200, serde_json::json!({
            "ok": true,
            "name": "eaicoding-mcp",
            "version": env!("CARGO_PKG_VERSION")
        })),
        ("POST", "/message") => handle_sse_message(&request.path, request.body, &sse_connections).await,
        ("POST", "/upload") => handle_http_upload(request).await,
        _ => http_json(404, serde_json::json!({
            "error": "not_found",
            "message": "支持的接口: GET /health, GET /sse, POST /message, POST /upload"
        })),
    }
}

async fn handle_sse_message(path: &str, body: Vec<u8>, sse_connections: &SseConnections) -> String {
    let Some(connection_id) = parse_query_param(path, "connectionId") else {
        eprintln!("[POST /message] 缺失 connectionId 参数");
        return http_json(400, serde_json::json!({
            "error": "missing_connection_id",
            "message": "缺少 connectionId 参数"
        }));
    };

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("[POST /message] 解析 JSON-RPC 请求失败: {}", err);
            return http_json(400, serde_json::json!({
                "error": "parse_error",
                "message": format!("Parse error: {}", err)
            }));
        }
    };

    if request.jsonrpc != "2.0" {
        return http_json(400, serde_json::json!({
            "error": "invalid_request",
            "message": "Invalid JSON-RPC version"
        }));
    }

    let Some(id) = request.id else {
        eprintln!("[POST /message] 接收到 MCP 通知: method={}", request.method);
        return http_empty(202);
    };

    let conns_clone = sse_connections.clone();
    let conn_id_clone = connection_id.clone();

    tokio::spawn(async move {
        let started = Instant::now();
        eprintln!("[RPC] start method={} id={} conn={}", request.method, id, conn_id_clone);
        let response_val = match handle_mcp_call(&request.method, request.params, id.clone()).await {
            Ok(result) => {
                eprintln!(
                    "[RPC] ok method={} id={} conn={} elapsed={}ms",
                    request.method,
                    id,
                    conn_id_clone,
                    started.elapsed().as_millis()
                );
                serde_json::to_value(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Some(id),
                    result: Some(result),
                    error: None,
                }).unwrap()
            }
            Err(err) => {
                eprintln!(
                    "[RPC] error method={} id={} conn={} elapsed={}ms error={}",
                    request.method,
                    id,
                    conn_id_clone,
                    started.elapsed().as_millis(),
                    err
                );
                serde_json::to_value(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Some(id),
                    result: None,
                    error: Some(JsonRpcError { code: -32603, message: err }),
                }).unwrap()
            }
        };

        if let Ok(serialized) = serde_json::to_string(&response_val) {
            let conns = conns_clone.lock().await;
            if let Some(tx) = conns.get(&conn_id_clone) {
                if let Err(e) = tx.send(serialized) {
                    eprintln!("[RPC] 往 SSE 发送数据失败 conn={}: {:?}", conn_id_clone, e);
                }
            } else {
                eprintln!("[RPC] 未找到活跃的 SSE 连接 conn={}", conn_id_clone);
            }
        }
    });

    http_empty(202)
}

fn parse_query_param(path: &str, key: &str) -> Option<String> {
    let query_start = path.find('?')?;
    let query = &path[query_start + 1..];
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

async fn handle_http_upload(request: HttpRequest) -> String {
    let content_type = request
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_default();
    eprintln!(
        "[UPLOAD] start content_type={} size={} bytes",
        content_type,
        request.body.len()
    );

    let result = if content_type.contains("multipart/form-data") {
        save_multipart_efile(&content_type, &request.body).await
    } else if content_type.contains("application/json") {
        save_json_upload(&request.body).await
    } else {
        let file_name = request
            .headers
            .get("x-file-name")
            .map(String::as_str)
            .unwrap_or("upload.e");
        save_uploaded_efile(file_name, &request.body).await
    };

    match result {
        Ok(value) => {
            eprintln!("[UPLOAD] ok");
            http_json(200, value)
        }
        Err(err) => {
            eprintln!("[UPLOAD] error {}", err);
            http_json(400, serde_json::json!({
                "success": false,
                "error": err
            }))
        }
    }
}

async fn save_json_upload(body: &[u8]) -> Result<Value, String> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|err| format!("解析上传 JSON 失败: {}", err))?;
    let file_name = payload
        .get("file_name")
        .or_else(|| payload.get("filename"))
        .and_then(|value| value.as_str())
        .unwrap_or("upload.e");
    let content_base64 = payload
        .get("content_base64")
        .or_else(|| payload.get("base64"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| "缺少 content_base64".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64)
        .map_err(|err| format!("Base64 解码失败: {}", err))?;
    save_uploaded_efile(file_name, &bytes).await
}

fn split_bytes<'a>(data: &'a [u8], separator: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut current = data;
    while let Some(pos) = current.windows(separator.len()).position(|w| w == separator) {
        parts.push(&current[..pos]);
        current = &current[pos + separator.len()..];
    }
    parts.push(current);
    parts
}

async fn save_multipart_efile(content_type: &str, body: &[u8]) -> Result<Value, String> {
    let boundary = content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .map(|value| value.trim_matches('"').to_string())
        .ok_or_else(|| "multipart 缺少 boundary".to_string())?;
    let marker = format!("--{}", boundary).into_bytes();

    for part in split_bytes(body, &marker) {
        let Some(sep_idx) = part.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers_bytes = &part[..sep_idx];
        let headers_text = String::from_utf8_lossy(headers_bytes);
        if !headers_text.contains("Content-Disposition") {
            continue;
        }
        let file_name = headers_text
            .split(';')
            .find_map(|item| item.trim().strip_prefix("filename="))
            .map(|value| value.trim_matches('"'))
            .unwrap_or("upload.e");

        let mut payload = &part[sep_idx + 4..];
        // Strip leading \r\n if present
        if payload.starts_with(b"\r\n") {
            payload = &payload[2..];
        }
        // Strip trailing \r\n if present
        if payload.ends_with(b"\r\n") {
            payload = &payload[..payload.len() - 2];
        }
        // Also strip trailing -- if it's the end of multipart
        if payload.ends_with(b"--") {
            payload = &payload[..payload.len() - 2];
        }
        if payload.ends_with(b"\r\n") {
            payload = &payload[..payload.len() - 2];
        }

        return save_uploaded_efile(file_name, payload).await;
    }

    Err("multipart 中没有找到文件字段".to_string())
}

async fn upload_efile_from_tool(args: Value) -> Result<Value, String> {
    let file_name = args
        .get("file_name")
        .or_else(|| args.get("filename"))
        .and_then(|value| value.as_str())
        .unwrap_or("upload.e");
    let content_base64 = args
        .get("content_base64")
        .or_else(|| args.get("base64"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| "参数缺失: content_base64".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64)
        .map_err(|err| format!("Base64 解码失败: {}", err))?;
    save_uploaded_efile(file_name, &bytes).await
}

async fn save_uploaded_efile(file_name: &str, bytes: &[u8]) -> Result<Value, String> {
    let safe_name = sanitize_upload_name(file_name)?;
    let ext = Path::new(&safe_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !ext.eq_ignore_ascii_case("e") {
        return Err("只允许上传 .e 文件".to_string());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("系统时间异常: {}", err))?
        .as_millis();
    let upload_dir = get_eaicoding_dir()
        .ok_or_else(|| "无法获取 EAiCoding 数据目录".to_string())?
        .join("uploads")
        .join(timestamp.to_string());
    fs::create_dir_all(&upload_dir)
        .map_err(|err| format!("创建上传目录失败 {}: {}", upload_dir.display(), err))?;

    let path = upload_dir.join(safe_name);
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|err| format!("保存上传文件失败 {}: {}", path.display(), err))?;

    eprintln!(
        "[UPLOAD] saved file={} size={} bytes",
        path.display(),
        bytes.len()
    );

    Ok(serde_json::json!({
        "success": true,
        "file_path": path.to_string_lossy().to_string(),
        "size": bytes.len()
    }))
}

fn sanitize_upload_name(file_name: &str) -> Result<String, String> {
    let name = PathBuf::from(file_name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("upload.e")
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>();

    if name.trim().is_empty() {
        Err("文件名不能为空".to_string())
    } else {
        Ok(name)
    }
}

fn http_empty(status: u16) -> String {
    http_response(status, "application/json; charset=utf-8", Vec::new())
}

fn http_json(status: u16, value: Value) -> String {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    http_response(status, "application/json; charset=utf-8", body)
}

fn http_response(status: u16, content_type: &str, body: Vec<u8>) -> String {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body_text = String::from_utf8_lossy(&body);
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, X-File-Name\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        content_type,
        body.len(),
        body_text
    )
}

async fn handle_mcp_call(method: &str, params: Option<Value>, _id: Value) -> Result<Value, String> {
    match method {
        "initialize" => {
            Ok(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "eaicoding-mcp",
                    "version": "0.1.2"
                },
                "capabilities": {
                    "tools": {}
                }
            }))
        }
        "tools/list" => {
            let tools = get_mcp_tools_schema();
            Ok(serde_json::json!({
                "tools": tools
            }))
        }
        "tools/call" => {
            let params = params.ok_or_else(|| "缺少 params 参数".to_string())?;
            let call_name = params.get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "缺少 tool 的 name 字段".to_string())?;
            let args = params.get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            eprintln!("开始执行 MCP Tool: name={}", call_name);
            let started = Instant::now();
            eprintln!("[TOOL] start name={}", call_name);
            let result_str = match execute_tool(call_name, args).await {
                Ok(result) => {
                    eprintln!(
                        "[TOOL] ok name={} elapsed={}ms",
                        call_name,
                        started.elapsed().as_millis()
                    );
                    result
                }
                Err(err) => {
                    eprintln!(
                        "[TOOL] error name={} elapsed={}ms error={}",
                        call_name,
                        started.elapsed().as_millis(),
                        err
                    );
                    return Err(err);
                }
            };
            
            // 返回符合 MCP Spec 的规范响应：内容数组包
            Ok(serde_json::json!({
                "content": [
                    {
                        "type": "text",
                        "text": result_str
                    }
                ]
            }))
        }
        _ => Err(format!("未知的 JSON-RPC 方法: {}", method)),
    }
}

async fn execute_tool(name: &str, args: Value) -> Result<String, String> {
    match name {
        "upload_efile" => {
            let res = upload_efile_from_tool(args).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "inspect_env" => {
            let detected = detect_eagent_tools();
            Ok(serde_json::to_string_pretty(&detected).unwrap_or_default())
        }
        "setup_env" => {
            setup_env_tool_impl().await
        }
        "parse_efile" => {
            let file_path = args.get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: file_path".to_string())?;
            let res = parse_efile(file_path.to_string()).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "export_ecode" => {
            let source_path = args.get("source_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: source_path".to_string())?;
            let output_dir = args.get("output_dir")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            let res = export_efile_to_ecode(source_path.to_string(), output_dir).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "generate_efile" => {
            let ecode_dir = args.get("ecode_dir")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: ecode_dir".to_string())?;
            let output_path = args.get("output_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let res = generate_efile_from_ecode(ecode_dir.to_string(), output_path).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "patch_file" => {
            let file_path = args.get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: file_path".to_string())?;
            let patch = args.get("patch")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: patch".to_string())?;
            let res = apply_patch_file(file_path.to_string(), patch.to_string()).await?;
            Ok(res)
        }
        "compile_efile" => {
            let source_path = args.get("source_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: source_path".to_string())?;
            let output_path = args.get("output_path")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            let static_link = args.get("static_link")
                .and_then(|v| v.as_bool());
            let module_paths = args.get("module_paths")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>());
            
            let res = compile_efile(
                source_path.to_string(),
                output_path,
                static_link,
                module_paths,
                None, // easy_language_root
            ).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "search_jingyi_module" => {
            let query = args.get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: query".to_string())?;
            let limit = args.get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let res = search_jingyi_module_rust(query.to_string(), limit)?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "analyze_project" => {
            let ecode_dir = args.get("ecode_dir")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: ecode_dir".to_string())?;
            let res = analyze_project_rust(ecode_dir.to_string()).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "read_ecode_file" => {
            let file_path = args.get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: file_path".to_string())?;
            let max_chars = args.get("max_chars")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let res = read_text_file_for_agent(file_path.to_string(), max_chars).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "write_ecode_file" => {
            let file_path = args.get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: file_path".to_string())?;
            let content = args.get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "参数缺失: content".to_string())?;
            // 写入 ecode 源码默认用 GBK 编码（兼容易语言文本格式）
            let res = write_text_file(file_path.to_string(), content.to_string(), Some("gbk".to_string())).await?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        "scan_env" => {
            let path = args.get("root_path").and_then(|v| v.as_str()).map(|s| s.to_string());
            let res = scan_easy_language_env(path)?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        _ => Err(format!("未支持的 MCP Tool: {}", name)),
    }
}

async fn setup_env_tool_impl() -> Result<String, String> {
    let tools_root = match app_tools_root() {
        Some(path) => path,
        None => return Err("无法获取本地应用数据目录".to_string()),
    };

    let target_dir = tools_root.join("eroot");
    let target_exe = target_dir.join("e.exe");
    if target_exe.exists() {
        return Ok("环境已配置完毕。易语言运行环境已存在于本地目录。".to_string());
    }

    let url = "https://github.com/ordinarykeys/eaicoding/releases/download/v0.1.1/e.zip";
    eprintln!("正在从远程下载易语言运行环境 e.zip，地址: {}", url);

    fs::create_dir_all(&tools_root)
        .map_err(|err| format!("创建目录失败 {}: {}", tools_root.display(), err))?;

    let zip_path = tools_root.join("e_temp.zip");
    
    // 发送网络请求流式下载
    let client = reqwest::Client::new();
    let response = client.get(url)
        .send()
        .await
        .map_err(|err| format!("网络请求失败: {:?}", err))?;

    if !response.status().is_success() {
        return Err(format!("无法下载依赖包，网络错误码: {}", response.status()));
    }

    let mut file = fs::File::create(&zip_path)
        .map_err(|err| format!("创建临时压缩包失败: {}", err))?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    
    use futures_util::StreamExt; // 引入 stream 处理
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|err| format!("读取下载数据块失败: {:?}", err))?;
        std::io::copy(&mut &*chunk, &mut file)
            .map_err(|err| format!("写入临时压缩包失败: {}", err))?;
        downloaded += chunk.len() as u64;
        
        // 打印简短的下载进度日志到 stderr
        if downloaded % (5 * 1024 * 1024) == 0 {
            eprintln!("下载中: 已下载 {} 字节", downloaded);
        }
    }
    
    drop(file);
    eprintln!("e.zip 下载完毕。开始解压环境...");

    let unzip_res = unzip_eroot(&zip_path, &target_dir);
    let _ = fs::remove_file(&zip_path); // 无论成功与否均清理临时文件

    unzip_res?;

    if target_exe.exists() {
        Ok(format!("易语言依赖环境一键安装成功！已解压至 {}", target_dir.display()))
    } else {
        Err("下载解压已完成，但未在目标位置检测到 e.exe，请检查下载包是否完整。".to_string())
    }
}

fn get_mcp_tools_schema() -> Value {
    serde_json::json!([
        {
            "name": "upload_efile",
            "description": "上传 .e 文件到 MCP 服务端，返回可用于 parse_efile/export_ecode/compile_efile 的服务端本地路径",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_name": {
                        "type": "string",
                        "description": "上传文件名，必须以 .e 结尾"
                    },
                    "content_base64": {
                        "type": "string",
                        "description": ".e 文件的 Base64 内容"
                    }
                },
                "required": ["file_name", "content_base64"]
            }
        },
        {
            "name": "inspect_env",
            "description": "检查本地是否配置了易语言命令行编译器与运行环境",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "setup_env",
            "description": "一键自动初始化环境。流式下载并解压易语言核心编译组件 e.zip，免除手动下载痛点",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "parse_efile",
            "description": "解析二进制易语言 .e / .ec 模块文件结构，提取其内公开的子程序、程序集及全局变量索引",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "本地易语言二进制文件的绝对路径"
                    }
                },
                "required": ["file_path"]
            }
        },
        {
            "name": "export_ecode",
            "description": "将易语言二进制 .e / .ec 文件导出为文本形式的 ecode 工程目录，便于读写与版本比对",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": {
                        "type": "string",
                        "description": "本地二进制源文件路径"
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "可选。导出的文本工程目录路径。不传则默认生成在 .eaicoding/ecode 目录下"
                    }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "generate_efile",
            "description": "将文本格式的 ecode 目录工程重新反向回编生成二进制的 .e 文件",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ecode_dir": {
                        "type": "string",
                        "description": "本地文本工程目录的绝对路径"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "可选。生成的目标 .e 文件绝对路径。不传则默认生成在 .eaicoding/auto-runs 目录下"
                    }
                },
                "required": ["ecode_dir"]
            }
        },
        {
            "name": "patch_file",
            "description": "针对文本工程中的源码文件进行局部 Search-Replace 差分修改修补，免除长文件重写开销",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "本地要修补的文件绝对路径"
                    },
                    "patch": {
                        "type": "string",
                        "description": "包含 <<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE 的 Patch 差异块内容"
                    }
                },
                "required": ["file_path", "patch"]
            }
        },
        {
            "name": "compile_efile",
            "description": "调用易语言命令行编译器对生成的 .e 文件进行编译验证，返回编译成功状态和 Stdout/Stderr 编译器日志",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": {
                        "type": "string",
                        "description": "本地二进制 .e 文件的绝对路径"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "可选。编译输出的目标 .exe 路径。默认替换后缀为 .exe"
                    },
                    "static_link": {
                        "type": "boolean",
                        "description": "是否使用静态链接。默认 true"
                    },
                    "module_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选。编译所需的依赖外部模块 (.ec) 的本地绝对路径列表"
                    }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "search_jingyi_module",
            "description": "检索本地精易模块的 API 库。输入功能中文描述，输出匹配的 API 列表、签名和推荐组合路线",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索意图或特定 API 关键字（如 '多线程POST'）"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "可选。最大返回条数，默认 8"
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "analyze_project",
            "description": "扫描易语言文本工程，进行硬编码 URL、不安全明文 HTTP 请求、前端元素选择器、缺少异常处理的网络调用以及重复代码等项目的静态诊断并输出报告",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ecode_dir": {
                        "type": "string",
                        "description": "本地文本工程目录路径"
                    }
                },
                "required": ["ecode_dir"]
            }
        },
        {
            "name": "read_ecode_file",
            "description": "读取文本工程中指定的文件内容。后台自动处理 GBK 字符集解码，返回正常的 UTF-8 文本内容，防止中文乱码",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "本地要读取的文件绝对路径"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "可选。读取的字符最大限制，默认 12000"
                    }
                },
                "required": ["file_path"]
            }
        },
        {
            "name": "write_ecode_file",
            "description": "将 UTF-8 文本内容安全地写入到工程指定位置。后台自动将其编码转换为 GBK 字符集并写入为 Windows 换行符（CRLF），确保易语言支持库与编译器正常处理",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "本地要写入的文件绝对路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "要写入的纯文本 UTF-8 源码内容"
                    }
                },
                "required": ["file_path", "content"]
            }
        },
        {
            "name": "scan_env",
            "description": "扫描指定路径或默认位置下的完整易语言开发环境（编译器、子目录组件及支持库等）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root_path": {
                        "type": "string",
                        "description": "可选。易语言安装根目录"
                    }
                }
            }
        }
    ])
}

/// 格式化打印单个工具链组件的检测状态到 stderr
fn print_env_status(name: &str, check: &ToolCheck) {
    if check.exists {
        let path = check.path.as_deref().unwrap_or("未知路径");
        eprintln!("[环境检测]     {:<18} => {}", name, path);
    } else {
        let reason = check.reason.as_deref().unwrap_or("未知原因");
        eprintln!("[环境检测]  x  {:<18} => {}", name, reason);
    }
}
