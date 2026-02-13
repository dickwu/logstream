// ─────────────────────────────────────────────
// Browser SDK — uses fetch + sendBeacon (no WS dependency)
// ─────────────────────────────────────────────

export interface BrowserLoggerOptions {
  /** HTTP endpoint (e.g. http://localhost:4800/ingest) */
  endpoint: string;
  /** Project name */
  project: string;
  /** Environment */
  environment?: string;
  /** Flush interval in ms (default: 500) */
  flushMs?: number;
  /** Auto-capture unhandled errors (default: true) */
  captureErrors?: boolean;
  /** Auto-capture console.error (default: false) */
  captureConsole?: boolean;
}

export function createBrowserLogger(opts: BrowserLoggerOptions) {
  let queue: any[] = [];
  const flushMs = opts.flushMs ?? 500;

  // Periodic flush
  const timer = setInterval(() => flush(false), flushMs);

  // Flush on page hide (use sendBeacon for reliability)
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") flush(true);
    });
    // Also flush on beforeunload
    window.addEventListener("beforeunload", () => flush(true));
  }

  function flush(beacon = false) {
    if (!queue.length) return;
    const batch = queue;
    queue = [];
    const body = JSON.stringify(batch);

    if (beacon && typeof navigator?.sendBeacon === "function") {
      navigator.sendBeacon(opts.endpoint, body);
    } else {
      fetch(opts.endpoint, {
        method: "POST",
        body,
        headers: { "Content-Type": "application/json" },
        keepalive: true,
      }).catch(() => {});
    }
  }

  function log(level: string, message: string, meta?: Record<string, any>) {
    queue.push({
      timestamp: new Date().toISOString(),
      project: opts.project,
      environment: opts.environment || "dev",
      level,
      message,
      meta: {
        ...meta,
        url: typeof location !== "undefined" ? location.href : undefined,
      },
    });
  }

  // Auto-capture unhandled errors
  if (opts.captureErrors !== false && typeof window !== "undefined") {
    window.addEventListener("error", (e) => {
      log("error", e.message, {
        source: `${e.filename}:${e.lineno}:${e.colno}`,
        stack: e.error?.stack,
        type: "unhandled_error",
      });
    });

    window.addEventListener("unhandledrejection", (e) => {
      log("error", `Unhandled promise rejection: ${e.reason}`, {
        stack: e.reason?.stack,
        type: "unhandled_rejection",
      });
    });
  }

  // Auto-capture console.error
  if (opts.captureConsole && typeof console !== "undefined") {
    const origError = console.error;
    console.error = (...args: any[]) => {
      log("error", args.map(String).join(" "), { type: "console_error" });
      origError.apply(console, args);
    };
  }

  return {
    debug: (msg: string, meta?: Record<string, any>) => log("debug", msg, meta),
    info: (msg: string, meta?: Record<string, any>) => log("info", msg, meta),
    warn: (msg: string, meta?: Record<string, any>) => log("warn", msg, meta),
    error: (msg: string, meta?: Record<string, any>) => log("error", msg, meta),
    fatal: (msg: string, meta?: Record<string, any>) => log("fatal", msg, meta),
    flush: () => flush(false),
    close: () => {
      clearInterval(timer);
      flush(true);
    },
  };
}
