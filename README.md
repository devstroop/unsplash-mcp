# Unsplash MCP (Rust + chromiumoxide)

MCP server for Unsplash image discovery. Rebuilt in Rust after the original
Node/axios scraper was permanently blocked by Unsplash's bot-wall
(Anubis proof-of-work + BotStopper "Oh noes!" deny page → HTTP 401 on every request).

Pure headless-Chrome approach — **no API key, no setup needed**. The server
fully manages its own browser and every tool just works:

1. Hardened stealth headless Chrome (new-headless mode, anti-detection
   scripts, modern UA, no `--enable-automation`) solves the Anubis challenge
   and reads Unsplash's internal `/napi/*` JSON endpoints.
2. If BotStopper denies headless, the server automatically launches a
   server-managed headed Chrome (a window briefly opens) and retries there.
3. Clearance cookies persist in `~/.cache/unsplash-mcp/chrome-profile`
   (separate `headless`/`headed` subdirs), stale locks from crashed
   predecessors are healed automatically, and each tool call uses a fresh
   tab that is closed afterwards.

## Setup

```bash
cargo build --release
```

Add to your MCP config:

```json
{
  "unsplash": {
    "command": "/abs/path/to/unsplash-mcp/target/release/unsplash-mcp"
  }
}
```

Requires a Chrome/Chromium binary (auto-detected, override with `CHROME_PATH`).
Nothing else to configure — no headed Chrome to launch, no debug ports, no
extra env vars.

## If both browser modes are denied

If Unsplash denies headless *and* headed browsers, the egress IP itself is
flagged by BotStopper (common with datacenter/VPN/proxy networks). That is
network-level and no browser setting can fix it — retry later or from a
different network. The tool error says exactly this.

### Environment variables

| Var | Purpose | Default |
|---|---|---|
| `CHROME_PATH` | Chrome binary override | auto-detected |
| `UNSPLASH_CHROME_PROFILE` | Persistent Chrome profile dir (keeps Anubis clearance) | `~/.cache/unsplash-mcp/chrome-profile` |
| `UNSPLASH_ANUBIS_TIMEOUT_SECS` | Bot-check wait budget | `45` |
| `UNSPLASH_CHROME_LAUNCH_TIMEOUT_SECS` | Browser launch budget | `60` |
| `RUST_LOG` | Log level (`tracing`) | `info` |

Logs always go to **stderr** — stdout is reserved for MCP JSON-RPC.

## Tools

| Tool | Description |
|------|-------------|
| `search_images` | Search by keywords, orientation, color |
| `get_popular_images` | Trending/popular images (`latest`/`oldest`/`popular`) |
| `browse_category` | Browse by topic (e.g. `nature`, `architecture`) |
| `get_user_profile` | Photographer info + recent photos |
| `get_image_details` | Full metadata for one image ID |
| `search_by_color` | Broad search filtered by dominant color |
| `get_collections` | Curated collections |
| `get_collection_photos` | Photos in a collection |
| `get_random_photos` | Random photos, optional query/orientation filter |

## Development

```bash
cargo build          # debug build
cargo test           # fast offline unit tests (encoding, slugs, response shapes, arg defaults)
UNSPLASH_LIVE_TEST=1 cargo test   # also runs the live Chrome clearance + napi test
cargo build --release
```

Smoke-test the protocol handshake:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"p","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | ./target/debug/unsplash-mcp
```

## License

MIT
