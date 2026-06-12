#!/usr/bin/env python3
import json
import os
import socket
import sys
import urllib.request
import urllib.error

DEFAULT_URL = "https://babylon.taild4189d.ts.net/mcp"
PROTOCOL_VERSION = "2025-06-18"
TIMEOUT = 10
MAX_LINES = 10


def load_config():
    token = os.environ.get("BABYLON_TOKEN")
    url = None
    project_dir = os.environ.get("CLAUDE_PROJECT_DIR")
    if project_dir:
        path = os.path.join(project_dir, ".mcp.json")
        try:
            with open(path, "r") as fh:
                data = json.load(fh)
            server = data.get("mcpServers", {}).get("babylon", {})
            url = server.get("url")
            if not token:
                auth = server.get("headers", {}).get("Authorization", "")
                if auth.startswith("Bearer "):
                    token = auth[len("Bearer "):]
                elif auth:
                    token = auth
        except Exception:
            pass
    if not url:
        url = DEFAULT_URL
    return token, url


def post(url, token, body, session_id):
    payload = json.dumps(body).encode("utf-8")
    headers = {
        "Authorization": "Bearer " + token,
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        "MCP-Protocol-Version": PROTOCOL_VERSION,
    }
    if session_id:
        headers["Mcp-Session-Id"] = session_id
    req = urllib.request.Request(url, data=payload, headers=headers, method="POST")
    return urllib.request.urlopen(req, timeout=TIMEOUT)


def extract_session_id(resp):
    for key in resp.headers.keys():
        if key.lower() == "mcp-session-id":
            return resp.headers.get(key)
    return None


def read_sse_result(resp, want_id):
    data_lines = []
    for raw in resp:
        line = raw.decode("utf-8", "replace").rstrip("\r\n")
        if line == "":
            if data_lines:
                chunk = "\n".join(data_lines)
                data_lines = []
                try:
                    obj = json.loads(chunk)
                except Exception:
                    obj = None
                if isinstance(obj, dict) and obj.get("id") == want_id:
                    return obj
            continue
        if line.startswith("data:"):
            data_lines.append(line[5:].lstrip())
    if data_lines:
        try:
            obj = json.loads("\n".join(data_lines))
            if isinstance(obj, dict) and obj.get("id") == want_id:
                return obj
        except Exception:
            pass
    return None


def call_catch_up(url, token):
    init_body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "babylon-notify", "version": "0"},
        },
    }
    resp = post(url, token, init_body, None)
    session_id = extract_session_id(resp)
    init_result = read_sse_result(resp, 1)
    try:
        resp.close()
    except Exception:
        pass
    if init_result is None:
        return None

    notified_body = {"jsonrpc": "2.0", "method": "notifications/initialized"}
    try:
        nresp = post(url, token, notified_body, session_id)
        try:
            nresp.read()
        except Exception:
            pass
        nresp.close()
    except urllib.error.HTTPError as e:
        try:
            e.close()
        except Exception:
            pass
    except Exception:
        pass

    call_body = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "catch_up", "arguments": {"only_mentions": True}},
    }
    resp = post(url, token, call_body, session_id)
    result = read_sse_result(resp, 2)
    try:
        resp.close()
    except Exception:
        pass
    return result


def coerce_items(structured):
    if structured is None:
        return []
    if isinstance(structured, list):
        return structured
    if isinstance(structured, dict):
        for key in ("items", "unread", "messages", "mentions", "results", "posts"):
            val = structured.get(key)
            if isinstance(val, list):
                return val
        channels = structured.get("channels")
        if isinstance(channels, list):
            collected = []
            for ch in channels:
                if isinstance(ch, dict):
                    for key in ("items", "unread", "messages", "posts"):
                        val = ch.get(key)
                        if isinstance(val, list):
                            collected.extend(val)
            if collected:
                return collected
    return []


def pick(item, keys):
    for key in keys:
        if key in item and item[key] not in (None, ""):
            return item[key]
    return None


def normalize(item):
    if not isinstance(item, dict):
        return None
    channel = pick(item, ("channel", "ch", "channel_name", "channelName", "chan"))
    kind = pick(item, ("kind", "type", "message_kind"))
    frm = pick(item, ("from", "author", "from_handle", "fromHandle", "sender", "agent", "by", "username"))
    summary = pick(item, ("summary", "sum", "text", "body", "title", "content", "message"))
    ident = pick(item, ("id", "msg_id", "message_id", "messageId", "seq"))
    return {
        "channel": channel if channel is not None else "?",
        "kind": kind if kind is not None else "?",
        "from": frm if frm is not None else "?",
        "summary": summary if summary is not None else "",
        "id": ident,
    }


def gather(result):
    if not isinstance(result, dict):
        return []
    inner = result.get("result")
    if not isinstance(inner, dict):
        return []
    structured = inner.get("structuredContent")
    items = coerce_items(structured)
    if not items:
        content = inner.get("content")
        if isinstance(content, list):
            for block in content:
                if isinstance(block, dict) and block.get("type") == "text":
                    try:
                        parsed = json.loads(block.get("text", ""))
                    except Exception:
                        continue
                    items = coerce_items(parsed)
                    if items:
                        break
    out = []
    for it in items:
        norm = normalize(it)
        if norm is not None:
            out.append(norm)
    return out


def main():
    args = sys.argv[1:]
    as_json = "--json" in args
    try:
        token, url = load_config()
        if not token:
            sys.exit(0)
        result = call_catch_up(url, token)
        items = gather(result)
    except Exception:
        sys.exit(0)

    if as_json:
        payload = {
            "count": len(items),
            "items": [
                {
                    "channel": it["channel"],
                    "kind": it["kind"],
                    "from": it["from"],
                    "summary": it["summary"],
                    "id": it["id"],
                }
                for it in items
            ],
        }
        sys.stdout.write(json.dumps(payload))
        sys.exit(0)

    if not items:
        sys.exit(0)

    lines = []
    for it in items[:MAX_LINES]:
        summary = str(it["summary"]).replace("\n", " ").strip()
        lines.append(
            "{} · {} · @{} · {}".format(
                it["channel"], it["kind"], it["from"], summary
            )
        )
    if len(items) > MAX_LINES:
        lines.append("…and {} more".format(len(items) - MAX_LINES))
    sys.stdout.write("\n".join(lines) + "\n")
    sys.exit(0)


if __name__ == "__main__":
    socket.setdefaulttimeout(TIMEOUT)
    main()
