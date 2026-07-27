# EAiCoding Streamable HTTP MCP Bridge

This Python service keeps the existing Rust EAiCoding MCP server as the tool
execution backend and exposes it through Streamable HTTP at `/mcp`.

## Start

1. Start the existing Rust service on port `8765`.
2. Install the Python dependencies:

   ```powershell
   python -m pip install -r requirements.txt
   ```

3. Start the bridge:

   ```powershell
   python eaicoding_streamable_mcp.py
   ```

The bridge listens on `http://127.0.0.1:8766/mcp` by default. Override the
legacy backend with `EAICODING_LEGACY_URL` and the listening port with
`EAICODING_MCP_PORT` when needed.

## Codex Configuration

```toml
[mcp_servers.eaicoding-mcp]
url = "http://127.0.0.1:8766/mcp"
```

Do not configure `command` for this bridge. The Rust SSE service and this
Python Streamable HTTP service must both be running before Codex creates a
new task.
