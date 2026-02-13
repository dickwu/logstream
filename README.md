# Logstream

Real-time multi-project log collector with full-text search, tracing, and AI integration.

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│  Frontend    │  │  Backend    │  │  Service N   │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │ HTTP POST        │ WebSocket       │ WebSocket
       └─────────────────┬┘────────────────┘
                         ▼
              ┌─────────────────────┐
              │  Logstream Server   │  ← Rust (axum + tokio)
              │  :4800              │
              └────┬───────┬────────┘
                   │       │
          ┌────────▼──┐ ┌──▼──────────┐
          │Meilisearch │ │ Live WS Feed│
          │ :7700      │ │             │
          └────┬───────┘ └─────────────┘
               │
        ┌──────┴───────┐
        │ MCP / CLI    │
        └──────────────┘
```

**Stack:**
- **Server:** Rust (axum, tokio) — high throughput, low latency
- **Storage:** Meilisearch — full-text search, faceting, typo-tolerant
- **SDKs:** TypeScript (Node.js + Browser)
- **AI:** MCP server for Claude/Cursor integration
- **Protocol:** WebSocket (real-time) + HTTP POST (fallback)

## Quick Start

### 1. Start Meilisearch

```bash
docker compose up -d
```

### 2. Initialize the index

```bash
# Build the server
cargo build --release

# Initialize Meilisearch index with correct settings
MEILI_KEY=dev-master-key ./target/release/logstream init
```

### 3. Start the collector

```bash
MEILI_KEY=dev-master-key ./target/release/logstream serve
```

The server starts on `:4800` with these endpoints:

| Endpoint | Method | Description |
|---|---|---|
| `/ingest` | POST | HTTP log ingestion (single or batch) |
| `/ws` | GET (WS) | WebSocket — ingest or subscribe mode |
| `/search` | GET | Query logs (full-text + filters) |
| `/projects` | GET | Project/level/environment facet breakdown |
| `/trace/:id` | GET | Full trace timeline |
| `/errors` | GET | Error summary with facets |
| `/health` | GET | Health check |

## Ingesting Logs

### HTTP POST

```bash
# Single log
curl -X POST http://localhost:4800/ingest \
  -H 'Content-Type: application/json' \
  -d '{
    "project": "api-server",
    "level": "info",
    "message": "Server started on :3000",
    "meta": {"port": 3000}
  }'

# Batch
curl -X POST http://localhost:4800/ingest \
  -H 'Content-Type: application/json' \
  -d '[
    {"project": "api-server", "level": "info", "message": "Request received", "traceId": "abc-123"},
    {"project": "api-server", "level": "error", "message": "DB timeout", "traceId": "abc-123"}
  ]'
```

### WebSocket (ingest mode)

Connect to `ws://localhost:4800/ws` and send JSON messages:

```javascript
const ws = new WebSocket("ws://localhost:4800/ws");
ws.send(JSON.stringify({
  project: "frontend",
  level: "error",
  message: "Uncaught TypeError: x is not a function",
  meta: { component: "Dashboard", url: "/app/dashboard" }
}));
```

### WebSocket (subscribe mode)

Connect with `?mode=subscribe` to receive logs in real time:

```javascript
// All logs
const ws = new WebSocket("ws://localhost:4800/ws?mode=subscribe");

// Only frontend errors
const ws = new WebSocket("ws://localhost:4800/ws?mode=subscribe&projects=frontend&levels=error");

// Follow a specific trace
const ws = new WebSocket("ws://localhost:4800/ws?mode=subscribe&traceId=abc-123");
```

## Querying Logs

### Search

```bash
# Full-text search
curl "http://localhost:4800/search?q=database+timeout"

# Filter by project + level
curl "http://localhost:4800/search?project=api-server&level=error"

# Recent errors (last hour)
curl "http://localhost:4800/search?level=error&since=1h"

# Combined
curl "http://localhost:4800/search?q=timeout&project=api-server&since=2h&limit=50"
```

Response includes faceted counts:
```json
{
  "totalHits": 42,
  "facets": {
    "project": { "api-server": 30, "auth-service": 12 },
    "level": { "error": 35, "warn": 7 }
  },
  "hits": [...]
}
```

### Trace Timeline

```bash
curl "http://localhost:4800/trace/abc-123-def"
```

### Error Summary

```bash
curl "http://localhost:4800/errors?since=1h&project=api-server"
```

## Node.js SDK

```typescript
import { createLogger } from "./sdk/node";

const logger = createLogger({
  serverUrl: "ws://localhost:4800/ws",
  project: "api-server",
  environment: "dev",
});

// Simple logging
logger.info("Server started");
logger.error("DB connection failed", {
  meta: { host: "db.local", code: "ECONNREFUSED" },
});

// Distributed tracing
const trace = logger.startTrace("POST /api/checkout");

const dbSpan = trace.span("db.getCart");
// ... do work ...
dbSpan.end();

const paymentSpan = trace.span("payment.charge");
// Pass trace context to downstream service
fetch("http://payment-service/charge", {
  headers: trace.headers(),
});
paymentSpan.end();

trace.end();
```

### Express middleware

```typescript
import { createLogger } from "./sdk/node";

const logger = createLogger({
  serverUrl: "ws://localhost:4800/ws",
  project: "api-server",
});

app.use((req, res, next) => {
  // Continue trace from upstream, or start new one
  const trace = req.headers["x-trace-id"]
    ? logger.continueTrace(req.headers as any, `${req.method} ${req.path}`)
    : logger.startTrace(`${req.method} ${req.path}`);

  req.trace = trace;
  res.setHeader("x-trace-id", trace.traceId);

  res.on("finish", () => {
    logger.info(`${req.method} ${req.path} ${res.statusCode}`, {
      traceId: trace.traceId,
      meta: { method: req.method, path: req.path, status: res.statusCode },
    });
    trace.end();
  });

  next();
});
```

## Browser SDK

```typescript
import { createBrowserLogger } from "./sdk/browser";

const logger = createBrowserLogger({
  endpoint: "http://localhost:4800/ingest",
  project: "frontend",
  captureErrors: true,   // auto-catch window.onerror + unhandledrejection
  captureConsole: false,  // optionally capture console.error
});

logger.info("App loaded", { route: "/dashboard" });
logger.error("API call failed", { endpoint: "/api/users", status: 500 });
```

## MCP Server (AI Integration)

The MCP server lets Claude, Cursor, or any MCP-compatible AI query your logs.

### Setup

```bash
cd mcp/
npm install
```

### Claude Desktop / Cursor config

```json
{
  "mcpServers": {
    "logstream": {
      "command": "npx",
      "args": ["tsx", "/path/to/logstream/mcp/log-mcp-server.ts"],
      "env": {
        "MEILI_HOST": "http://localhost:7700",
        "MEILI_KEY": "dev-master-key"
      }
    }
  }
}
```

### Available MCP Tools

| Tool | Description |
|---|---|
| `search_logs` | Full-text search with project/level/trace/time filters + facets |
| `get_trace` | Full trace timeline across services |
| `tail_logs` | Most recent N logs (like `tail -f` snapshot) |
| `list_projects` | All projects with level/environment breakdown |
| `error_summary` | Recent errors grouped by project |
| `find_similar` | Find logs with similar messages (powered by Meili's relevance) |

### Example AI Queries

> "Show me all errors in the last hour"
> → `error_summary(since: "1h")`

> "What happened during trace abc-123?"
> → `get_trace(traceId: "abc-123")`

> "Find logs similar to 'connection timeout on payment service'"
> → `find_similar(message: "connection timeout on payment service")`

## Log Entry Schema

```typescript
{
  id: string;              // ULID (auto-generated)
  timestamp: string;       // ISO 8601
  timestampMs: number;     // Unix ms (for range queries)
  project: string;         // "frontend", "api-server", etc.
  level: "debug" | "info" | "warn" | "error" | "fatal";
  message: string;

  // Tracing (optional)
  traceId?: string;        // Correlation across services
  spanId?: string;         // Specific operation
  parentSpanId?: string;   // Parent operation

  // Context (optional)
  meta?: object;           // Arbitrary metadata
  source?: string;         // File/component
  environment?: string;    // dev, staging, prod
}
```

## Project Structure

```
logstream/
├── src/
│   ├── main.rs           # CLI entry point (serve / init)
│   ├── config.rs         # Configuration
│   ├── models.rs         # Data types (LogEntry, etc.)
│   ├── collector.rs      # Server startup & wiring
│   ├── routes.rs         # HTTP + WebSocket handlers
│   ├── meili.rs          # Meilisearch client, batcher, index setup
│   └── subscribers.rs    # Live WebSocket subscriber management
├── sdk/
│   ├── node/index.ts     # Node.js SDK (WebSocket)
│   └── browser/index.ts  # Browser SDK (HTTP POST + sendBeacon)
├── mcp/
│   ├── log-mcp-server.ts # MCP server for AI integration
│   └── package.json
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml    # Meilisearch
└── README.md
```

## Configuration

All config via CLI flags or environment variables:

| Flag | Env | Default | Description |
|---|---|---|---|
| `--port` | — | 4800 | Server port |
| `--meili-host` | `MEILI_HOST` | http://localhost:7700 | Meilisearch URL |
| `--meili-key` | `MEILI_KEY` | — | Meilisearch API key |

## Performance Notes

- **Ingestion:** Logs are batched (200 docs or 250ms, whichever first) before flushing to Meilisearch
- **Broadcasting:** Live subscribers get logs immediately (before Meili flush) — zero-delay streaming
- **Search:** Sub-50ms for most queries thanks to Meilisearch's in-memory indexes
- **Memory:** ~100-200MB for Meilisearch with ~1M documents
- **Concurrency:** Rust async (tokio) handles thousands of concurrent connections

## Retention

Meilisearch has no built-in TTL. Run a cleanup cron:

```bash
# Delete logs older than 30 days
curl -X POST http://localhost:7700/indexes/logs/documents/delete \
  -H 'Authorization: Bearer dev-master-key' \
  -H 'Content-Type: application/json' \
  -d '{ "filter": "timestampMs < CUTOFF_MS" }'
```

## License

MIT
