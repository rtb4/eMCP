"""Expose the legacy EAiCoding SSE server through Streamable HTTP MCP."""

from __future__ import annotations

import asyncio
import contextlib
import http.client
import json
import logging
import os
import uuid
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlparse

from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import JSONResponse, Response
from starlette.routing import Route
import uvicorn


LOGGER = logging.getLogger("eaicoding.streamable_mcp")
MCP_PROTOCOL_VERSION = "2025-03-26"


class LegacyMcpError(RuntimeError):
    """Represents a failure while communicating with the legacy SSE server."""


@dataclass
class LegacySseConnection:
    """Keeps an old-style SSE connection alive until one response is received."""

    connection: http.client.HTTPConnection
    response: http.client.HTTPResponse
    message_path: str

    def close(self) -> None:
        """Release the socket held by the legacy SSE endpoint."""
        with contextlib.suppress(Exception):
            self.response.close()
        with contextlib.suppress(Exception):
            self.connection.close()


class LegacySseBridge:
    """Translates one Streamable HTTP request into one legacy SSE exchange."""

    def __init__(self, base_url: str, timeout_seconds: float) -> None:
        """Store the legacy server location and the request timeout."""
        parsed = urlparse(base_url)
        if parsed.scheme != "http" or not parsed.hostname:
            raise ValueError("EAICODING_LEGACY_URL must be an http URL")

        self._host = parsed.hostname
        self._port = parsed.port or 80
        self._base_path = parsed.path.rstrip("/")
        self._timeout_seconds = timeout_seconds

    def call(self, request: dict[str, Any]) -> dict[str, Any]:
        """Forward a JSON-RPC request and return the JSON-RPC response."""
        sse_connection = self._open_sse_connection()
        try:
            self._post_message(sse_connection.message_path, request)
            return self._read_json_rpc_response(sse_connection.response, request["id"])
        finally:
            sse_connection.close()

    def _open_sse_connection(self) -> LegacySseConnection:
        """Create a legacy SSE connection and obtain its message endpoint."""
        connection = http.client.HTTPConnection(
            self._host,
            self._port,
            timeout=self._timeout_seconds,
        )
        sse_path = f"{self._base_path}/sse" or "/sse"
        connection.request("GET", sse_path, headers={"Accept": "text/event-stream"})
        response = connection.getresponse()
        if response.status != 200:
            response_body = response.read().decode("utf-8", errors="replace")
            connection.close()
            raise LegacyMcpError(f"Legacy SSE connection failed: {response.status} {response_body}")

        event_name, event_data = self._read_sse_event(response)
        if event_name != "endpoint" or not event_data.startswith("/message?"):
            response.close()
            connection.close()
            raise LegacyMcpError("Legacy SSE server did not return an endpoint event")

        return LegacySseConnection(connection, response, event_data)

    def _post_message(self, message_path: str, request: dict[str, Any]) -> None:
        """Send the request body to the message endpoint announced by SSE."""
        body = json.dumps(request, ensure_ascii=False).encode("utf-8")
        connection = http.client.HTTPConnection(
            self._host,
            self._port,
            timeout=self._timeout_seconds,
        )
        try:
            connection.request(
                "POST",
                message_path,
                body=body,
                headers={"Content-Type": "application/json", "Content-Length": str(len(body))},
            )
            response = connection.getresponse()
            response_body = response.read().decode("utf-8", errors="replace")
            if response.status not in (200, 202, 204):
                raise LegacyMcpError(
                    f"Legacy MCP message request failed: {response.status} {response_body}"
                )
        finally:
            connection.close()

    def _read_json_rpc_response(
        self,
        response: http.client.HTTPResponse,
        request_id: Any,
    ) -> dict[str, Any]:
        """Read legacy SSE events until the matching JSON-RPC response arrives."""
        while True:
            _, event_data = self._read_sse_event(response)
            try:
                response_data = json.loads(event_data)
            except json.JSONDecodeError as error:
                raise LegacyMcpError(f"Legacy SSE returned invalid JSON: {error}") from error

            if response_data.get("id") == request_id:
                return response_data

    @staticmethod
    def _read_sse_event(response: http.client.HTTPResponse) -> tuple[str, str]:
        """Read one SSE event without assuming a particular event name."""
        event_name = "message"
        data_lines: list[str] = []

        while True:
            raw_line = response.readline()
            if not raw_line:
                raise LegacyMcpError("Legacy SSE connection closed before a response was received")

            line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
            if not line:
                if data_lines:
                    return event_name, "\n".join(data_lines)
                continue
            if line.startswith("event:"):
                event_name = line[6:].strip()
            elif line.startswith("data:"):
                data_lines.append(line[5:].lstrip())


def json_rpc_error(request_id: Any, code: int, message: str) -> dict[str, Any]:
    """Build a JSON-RPC error response with the original request identifier."""
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def mcp_headers(session_id: str | None = None) -> dict[str, str]:
    """Return Streamable HTTP MCP response headers."""
    headers = {"MCP-Protocol-Version": MCP_PROTOCOL_VERSION}
    if session_id:
        headers["Mcp-Session-Id"] = session_id
    return headers


async def handle_mcp(request: Request) -> Response:
    """Serve one Streamable HTTP MCP JSON-RPC request at POST /mcp."""
    if request.method == "DELETE":
        return Response(status_code=204)
    if request.method != "POST":
        return Response(status_code=405, headers={"Allow": "POST, DELETE"})

    if "application/json" not in request.headers.get("content-type", ""):
        return Response(status_code=415, content="Content-Type must be application/json")

    try:
        payload = await request.json()
    except json.JSONDecodeError:
        return JSONResponse(json_rpc_error(None, -32700, "Parse error"), status_code=400)

    if not isinstance(payload, dict) or payload.get("jsonrpc") != "2.0" or "method" not in payload:
        return JSONResponse(json_rpc_error(None, -32600, "Invalid Request"), status_code=400)

    request_id = payload.get("id")
    method = payload["method"]
    if request_id is None:
        # Legacy eMCP does not keep client state for MCP notifications.
        return Response(status_code=202)
    if method == "ping":
        return JSONResponse({"jsonrpc": "2.0", "id": request_id, "result": {}})

    bridge: LegacySseBridge = request.app.state.bridge
    try:
        legacy_response = await asyncio.to_thread(bridge.call, payload)
    except LegacyMcpError as error:
        LOGGER.warning("Legacy MCP request failed: %s", error)
        return JSONResponse(json_rpc_error(request_id, -32000, str(error)), status_code=502)

    session_id = request.headers.get("mcp-session-id")
    if method == "initialize" and "result" in legacy_response:
        # The Rust backend advertises the old SSE-era protocol version.
        legacy_response["result"]["protocolVersion"] = MCP_PROTOCOL_VERSION
        legacy_response["result"]["serverInfo"] = {
            "name": "eaicoding-streamable-mcp",
            "version": "1.0.0",
        }
        session_id = uuid.uuid4().hex

    return JSONResponse(legacy_response, headers=mcp_headers(session_id))


async def health(_: Request) -> Response:
    """Return a lightweight health response for local diagnostics."""
    return JSONResponse({"status": "ok", "transport": "streamable-http"})


def create_app() -> Starlette:
    """Create the Streamable HTTP MCP ASGI application."""
    legacy_url = os.getenv("EAICODING_LEGACY_URL", "http://127.0.0.1:8765")
    timeout_seconds = float(os.getenv("EAICODING_LEGACY_TIMEOUT_SECONDS", "120"))
    application = Starlette(
        routes=[
            Route("/mcp", handle_mcp, methods=["POST", "DELETE"]),
            Route("/health", health, methods=["GET"]),
        ]
    )
    application.state.bridge = LegacySseBridge(legacy_url, timeout_seconds)
    return application


app = create_app()


def main() -> None:
    """Run the local Streamable HTTP MCP bridge."""
    logging.basicConfig(level=os.getenv("LOG_LEVEL", "INFO"))
    uvicorn.run(
        app,
        host=os.getenv("EAICODING_MCP_HOST", "127.0.0.1"),
        port=int(os.getenv("EAICODING_MCP_PORT", "8766")),
        log_level=os.getenv("LOG_LEVEL", "info").lower(),
    )


if __name__ == "__main__":
    main()
