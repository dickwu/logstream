#!/usr/bin/env npx tsx
/**
 * Logstream MCP Server
 *
 * Exposes log search, tracing, and error analysis tools to AI systems.
 * Talks directly to Meilisearch — no collector server needed for reads.
 *
 * Usage:
 *   MEILI_HOST=http://localhost:7700 MEILI_KEY=your-key npx tsx log-mcp-server.ts
 *
 * MCP config (Claude Desktop / Cursor):
 *   {
 *     "mcpServers": {
 *       "logstream": {
 *         "command": "npx",
 *         "args": ["tsx", "/path/to/mcp/log-mcp-server.ts"],
 *         "env": {
 *           "MEILI_HOST": "http://localhost:7700",
 *           "MEILI_KEY": "your-key"
 *         }
 *       }
 *     }
 *   }
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { MeiliSearch } from "meilisearch";

const MEILI_HOST = process.env.MEILI_HOST || "http://localhost:7700";
const MEILI_KEY = process.env.MEILI_KEY || "";
const INDEX = "logs";

const meili = new MeiliSearch({ host: MEILI_HOST, apiKey: MEILI_KEY });
const logs = meili.index(INDEX);

// ─── Helper ───

function parseDuration(s: string): number {
  const m = s.match(/^(\d+)(s|m|h|d)$/);
  if (!m) return 3600000; // default 1h
  const n = parseInt(m[1]);
  const units: Record<string, number> = { s: 1000, m: 60000, h: 3600000, d: 86400000 };
  return n * (units[m[2]] || 0);
}

function buildFilter(opts: {
  project?: string;
  level?: string;
  traceId?: string;
  environment?: string;
  since?: string;
  extraFilter?: string;
}): string | undefined {
  const parts: string[] = [];
  if (opts.project) parts.push(`project = "${opts.project}"`);
  if (opts.level) parts.push(`level = "${opts.level}"`);
  if (opts.traceId) parts.push(`traceId = "${opts.traceId}"`);
  if (opts.environment) parts.push(`environment = "${opts.environment}"`);
  if (opts.since) {
    const cutoff = Date.now() - parseDuration(opts.since);
    parts.push(`timestampMs > ${cutoff}`);
  }
  if (opts.extraFilter) parts.push(opts.extraFilter);
  return parts.length ? parts.join(" AND ") : undefined;
}

// ─── Server ───

const server = new Server(
  { name: "logstream", version: "1.0.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler("tools/list", async () => ({
  tools: [
    {
      name: "search_logs",
      description:
        "Search logs with full-text search across all projects. Supports typo-tolerant text search, " +
        "filtering by project/level/trace/environment/time, and returns facet counts.",
      inputSchema: {
        type: "object" as const,
        properties: {
          query: { type: "string", description: "Full-text search (message, source, meta). Empty = filter only." },
          project: { type: "string", description: "Filter by project name" },
          level: { type: "string", enum: ["debug", "info", "warn", "error", "fatal"] },
          traceId: { type: "string", description: "Filter by trace ID" },
          environment: { type: "string", description: "Filter by environment" },
          since: { type: "string", description: "Time range: 5m, 1h, 2d" },
          limit: { type: "number", description: "Max results (default 20, max 100)" },
        },
      },
    },
    {
      name: "get_trace",
      description:
        "Get all log entries for a trace ID, ordered chronologically. " +
        "Shows the full request flow across services.",
      inputSchema: {
        type: "object" as const,
        properties: {
          traceId: { type: "string", description: "Trace ID to look up" },
        },
        required: ["traceId"],
      },
    },
    {
      name: "tail_logs",
      description:
        "Get the most recent N logs, optionally filtered by project/level. " +
        "Like `tail -f` but returns a snapshot.",
      inputSchema: {
        type: "object" as const,
        properties: {
          project: { type: "string" },
          level: { type: "string" },
          limit: { type: "number", description: "Number of recent logs (default 30)" },
        },
      },
    },
    {
      name: "list_projects",
      description:
        "List all projects that have sent logs, with counts broken down by level and environment.",
      inputSchema: { type: "object" as const, properties: {} },
    },
    {
      name: "error_summary",
      description:
        "Get recent errors and fatals with full context. Returns facet breakdown by project.",
      inputSchema: {
        type: "object" as const,
        properties: {
          since: { type: "string", description: "Time range (default 1h)" },
          project: { type: "string", description: "Limit to one project" },
          query: { type: "string", description: "Search within errors" },
        },
      },
    },
    {
      name: "find_similar",
      description:
        "Find logs with messages similar to the given text. " +
        "Useful for finding related errors, recurring patterns, or past occurrences.",
      inputSchema: {
        type: "object" as const,
        properties: {
          message: { type: "string", description: "Log message to find similar entries for" },
          project: { type: "string" },
          limit: { type: "number", description: "Max results (default 10)" },
        },
        required: ["message"],
      },
    },
  ],
}));

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;

  try {
    switch (name) {
      case "search_logs": {
        const filter = buildFilter({
          project: args.project,
          level: args.level,
          traceId: args.traceId,
          environment: args.environment,
          since: args.since,
        });

        const results = await logs.search(args.query || "", {
          filter,
          sort: ["timestamp:desc"],
          limit: Math.min(args.limit || 20, 100),
          facets: ["project", "level"],
          attributesToRetrieve: [
            "id", "timestamp", "project", "level", "message",
            "traceId", "spanId", "source", "meta", "environment",
          ],
        });

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              totalHits: results.estimatedTotalHits,
              facets: results.facetDistribution,
              hits: results.hits,
            }, null, 2),
          }],
        };
      }

      case "get_trace": {
        const results = await logs.search("", {
          filter: `traceId = "${args.traceId}"`,
          sort: ["timestamp:asc"],
          limit: 500,
        });

        const hits = results.hits;
        const projects = [...new Set(hits.map((h: any) => h.project))];

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              traceId: args.traceId,
              eventCount: hits.length,
              projects,
              timeline: hits.map((h: any) => ({
                timestamp: h.timestamp,
                project: h.project,
                level: h.level,
                message: h.message,
                spanId: h.spanId,
                parentSpanId: h.parentSpanId,
                source: h.source,
              })),
            }, null, 2),
          }],
        };
      }

      case "tail_logs": {
        const filter = buildFilter({
          project: args.project,
          level: args.level,
        });

        const results = await logs.search("", {
          filter,
          sort: ["timestamp:desc"],
          limit: Math.min(args.limit || 30, 100),
        });

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              count: results.hits.length,
              logs: results.hits.reverse(),
            }, null, 2),
          }],
        };
      }

      case "list_projects": {
        const results = await logs.search("", {
          facets: ["project", "level", "environment"],
          limit: 0,
        });

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              totalLogs: results.estimatedTotalHits,
              byProject: results.facetDistribution?.project,
              byLevel: results.facetDistribution?.level,
              byEnvironment: results.facetDistribution?.environment,
            }, null, 2),
          }],
        };
      }

      case "error_summary": {
        const filter = buildFilter({
          project: args.project,
          since: args.since || "1h",
          extraFilter: '(level = "error" OR level = "fatal")',
        });

        const results = await logs.search(args.query || "", {
          filter,
          sort: ["timestamp:desc"],
          limit: 30,
          facets: ["project"],
        });

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              totalErrors: results.estimatedTotalHits,
              byProject: results.facetDistribution?.project,
              recentErrors: results.hits.map((h: any) => ({
                timestamp: h.timestamp,
                project: h.project,
                message: h.message,
                source: h.source,
                traceId: h.traceId,
                meta: h.meta,
              })),
            }, null, 2),
          }],
        };
      }

      case "find_similar": {
        const filter = buildFilter({ project: args.project });

        const results = await logs.search(args.message, {
          filter,
          limit: args.limit || 10,
        });

        return {
          content: [{
            type: "text",
            text: JSON.stringify({
              query: args.message,
              matches: results.hits,
            }, null, 2),
          }],
        };
      }

      default:
        return {
          content: [{ type: "text", text: `Unknown tool: ${name}` }],
          isError: true,
        };
    }
  } catch (err: any) {
    return {
      content: [{ type: "text", text: `Error: ${err.message}` }],
      isError: true,
    };
  }
});

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error(`Logstream MCP server running (Meili: ${MEILI_HOST})`);
}

main();
