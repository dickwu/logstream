import WebSocket from "ws";
import { randomUUID } from "crypto";

// ─────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────

export interface LogstreamOptions {
  /** WebSocket URL of the logstream server (e.g. ws://localhost:4800/ws) */
  serverUrl: string;
  /** Project name (e.g. "api-server", "frontend") */
  project: string;
  /** Environment (default: "dev") */
  environment?: string;
  /** Batch flush interval in ms (default: 100) */
  batchMs?: number;
  /** Max batch size before forced flush (default: 50) */
  batchSize?: number;
}

export interface LogMeta {
  traceId?: string;
  spanId?: string;
  parentSpanId?: string;
  meta?: Record<string, any>;
  source?: string;
}

export interface TraceContext {
  traceId: string;
  spanId: string;
  /** Create a child span */
  span(name: string): SpanContext;
  /** End the trace */
  end(): void;
  /** Get headers for propagation to downstream services */
  headers(): Record<string, string>;
}

export interface SpanContext {
  traceId: string;
  spanId: string;
  parentSpanId: string;
  /** End the span */
  end(): void;
  /** Get headers for propagation */
  headers(): Record<string, string>;
}

// ─────────────────────────────────────────────
// Logger
// ─────────────────────────────────────────────

export function createLogger(opts: LogstreamOptions) {
  let ws: WebSocket;
  let connected = false;
  let queue: any[] = [];
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  const batchMs = opts.batchMs ?? 100;
  const batchSize = opts.batchSize ?? 50;

  function connect() {
    ws = new WebSocket(opts.serverUrl);
    ws.on("open", () => {
      connected = true;
      flush();
    });
    ws.on("close", () => {
      connected = false;
      reconnectTimer = setTimeout(connect, 2000);
    });
    ws.on("error", () => {});
  }
  connect();

  function flush() {
    if (!connected || !queue.length) return;
    ws.send(JSON.stringify(queue));
    queue = [];
  }

  const flushInterval = setInterval(flush, batchMs);

  function log(level: string, message: string, extra?: LogMeta) {
    const entry: any = {
      timestamp: new Date().toISOString(),
      project: opts.project,
      environment: opts.environment || "dev",
      level,
      message,
    };
    if (extra?.traceId) entry.traceId = extra.traceId;
    if (extra?.spanId) entry.spanId = extra.spanId;
    if (extra?.parentSpanId) entry.parentSpanId = extra.parentSpanId;
    if (extra?.meta) entry.meta = extra.meta;
    if (extra?.source) entry.source = extra.source;

    queue.push(entry);
    if (queue.length >= batchSize) flush();
  }

  return {
    debug: (msg: string, extra?: LogMeta) => log("debug", msg, extra),
    info: (msg: string, extra?: LogMeta) => log("info", msg, extra),
    warn: (msg: string, extra?: LogMeta) => log("warn", msg, extra),
    error: (msg: string, extra?: LogMeta) => log("error", msg, extra),
    fatal: (msg: string, extra?: LogMeta) => log("fatal", msg, extra),

    /** Flush buffered logs immediately */
    flush,

    /** Close the connection */
    close() {
      clearInterval(flushInterval);
      if (reconnectTimer) clearTimeout(reconnectTimer);
      flush();
      ws?.close();
    },

    /**
     * Start a new trace. Pass the returned context to child operations.
     * Propagate to other services via trace.headers().
     */
    startTrace(name: string): TraceContext {
      const traceId = randomUUID();
      const spanId = randomUUID();

      log("info", `trace:start ${name}`, {
        traceId,
        spanId,
        meta: { operation: name },
      });

      const trace: TraceContext = {
        traceId,
        spanId,

        span(childName: string): SpanContext {
          const childSpanId = randomUUID();
          log("info", `span:start ${childName}`, {
            traceId,
            spanId: childSpanId,
            parentSpanId: spanId,
            meta: { operation: childName },
          });
          return {
            traceId,
            spanId: childSpanId,
            parentSpanId: spanId,
            end() {
              log("info", `span:end ${childName}`, {
                traceId,
                spanId: childSpanId,
                parentSpanId: spanId,
              });
            },
            headers() {
              return {
                "x-trace-id": traceId,
                "x-span-id": childSpanId,
                "x-parent-span-id": spanId,
              };
            },
          };
        },

        end() {
          log("info", `trace:end ${name}`, { traceId, spanId });
        },

        headers() {
          return {
            "x-trace-id": traceId,
            "x-span-id": spanId,
          };
        },
      };

      return trace;
    },

    /**
     * Continue a trace from incoming request headers.
     * Use this on the receiving side of a cross-service call.
     */
    continueTrace(headers: Record<string, string | undefined>, name: string): TraceContext {
      const traceId = headers["x-trace-id"] || randomUUID();
      const parentSpanId = headers["x-span-id"];
      const spanId = randomUUID();

      log("info", `span:start ${name}`, {
        traceId,
        spanId,
        parentSpanId,
        meta: { operation: name },
      });

      return {
        traceId,
        spanId,
        span(childName: string): SpanContext {
          const childSpanId = randomUUID();
          log("info", `span:start ${childName}`, {
            traceId,
            spanId: childSpanId,
            parentSpanId: spanId,
            meta: { operation: childName },
          });
          return {
            traceId,
            spanId: childSpanId,
            parentSpanId: spanId,
            end() {
              log("info", `span:end ${childName}`, {
                traceId,
                spanId: childSpanId,
              });
            },
            headers() {
              return {
                "x-trace-id": traceId,
                "x-span-id": childSpanId,
                "x-parent-span-id": spanId,
              };
            },
          };
        },
        end() {
          log("info", `span:end ${name}`, { traceId, spanId });
        },
        headers() {
          return {
            "x-trace-id": traceId,
            "x-span-id": spanId,
            "x-parent-span-id": parentSpanId || "",
          };
        },
      };
    },
  };
}
