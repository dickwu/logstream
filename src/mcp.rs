//! MCP Server for logstream
//! 
//! Implements the Model Context Protocol for AI integration.
//! Communicates via stdin/stdout using JSON-RPC 2.0.

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncBufReadExt;

#[derive(Clone)]
pub struct McpConfig {
    pub meili_host: String,
    pub meili_key: String,
}

/// MCP JSON-RPC request
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC response
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Run the MCP server on stdio
pub async fn run_mcp_server(cfg: McpConfig) -> anyhow::Result<()> {
    use meilisearch_sdk::client::Client;
    
    let meili = Client::new(&cfg.meili_host, Some(&cfg.meili_key))?;
    let index = meili.index("logs");

    // Read lines from stdin
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await? {
            0 => break,
            _ => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                
                let request: JsonRpcRequest = match serde_json::from_str(line) {
                    Ok(req) => req,
                    Err(e) => {
                        eprintln!("Failed to parse request: {}", e);
                        continue;
                    }
                };

                let response = handle_request(&index, request).await;
                
                let response_json = serde_json::to_string(&response)?;
                println!("{}", response_json);
            }
        }
    }

    Ok(())
}

async fn handle_request(
    index: &meilisearch_sdk::indexes::Index,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "logstream",
                    "version": "0.1.0"
                }
            })),
            error: None,
        },
        
        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(json!({
                "tools": [
                    {
                        "name": "search_logs",
                        "description": "Search logs with full-text search across all projects. Supports filtering by project/level/trace/time.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "query": { "type": "string", "description": "Full-text search query" },
                                "project": { "type": "string", "description": "Filter by project" },
                                "level": { "type": "string", "enum": ["debug", "info", "warn", "error", "fatal"] },
                                "traceId": { "type": "string", "description": "Filter by trace ID" },
                                "since": { "type": "string", "description": "Time range: 5m, 1h, 2d" },
                                "limit": { "type": "number", "description": "Max results (default 20)" }
                            }
                        }
                    },
                    {
                        "name": "get_trace",
                        "description": "Get all log entries for a trace ID, ordered chronologically.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "traceId": { "type": "string" }
                            },
                            "required": ["traceId"]
                        }
                    },
                    {
                        "name": "tail_logs",
                        "description": "Get the most recent N logs, optionally filtered.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "project": { "type": "string" },
                                "level": { "type": "string" },
                                "limit": { "type": "number", "description": "Number of recent logs (default 30)" }
                            }
                        }
                    },
                    {
                        "name": "list_projects",
                        "description": "List all projects with log counts broken down by level.",
                        "inputSchema": { "type": "object", "properties": {} }
                    },
                    {
                        "name": "error_summary",
                        "description": "Get recent errors with full context.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "since": { "type": "string", "description": "Time range (default 1h)" },
                                "project": { "type": "string" }
                            }
                        }
                    },
                    {
                        "name": "find_similar",
                        "description": "Find logs similar to a given message.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": { "type": "string" },
                                "project": { "type": "string" },
                                "limit": { "type": "number" }
                            },
                            "required": ["message"]
                        }
                    }
                ]
            })),
            error: None,
        },
        
        "tools/call" => {
            let params = request.params;
            let tool_name = params.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let arguments = params.get("arguments")
                .and_then(|a| a.as_object())
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();

            let result = match tool_name {
                "search_logs" => {
                    let result = search_logs(index, &arguments).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                "get_trace" => {
                    let trace_id = arguments.get("traceId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let result = get_trace(index, trace_id).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                "tail_logs" => {
                    let result = tail_logs(index, &arguments).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                "list_projects" => {
                    let result = list_projects(index).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                "error_summary" => {
                    let result = error_summary(index, &arguments).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                "find_similar" => {
                    let message = arguments.get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let result = find_similar(index, message, &arguments).await;
                    json!({ "content": [{ "type": "text", "text": result }] })
                }
                _ => json!({ "error": "Unknown tool" }),
            };

            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(result),
                error: None,
            }
        }
        
        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            }),
        },
    }
}

// --- Tool Implementations ---

fn parse_duration(s: &str) -> i64 {
    let s = s.trim();
    if s.len() < 2 { return 3600000; }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().unwrap_or(1);
    match unit {
        "s" => num * 1000,
        "m" => num * 60000,
        "h" => num * 3600000,
        "d" => num * 86400000,
        _ => 3600000,
    }
}

fn build_filter(args: &std::collections::HashMap<String, serde_json::Value>) -> Option<String> {
    let mut parts = Vec::new();
    
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        parts.push(format!("project = \"{}\"", p));
    }
    if let Some(l) = args.get("level").and_then(|v| v.as_str()) {
        parts.push(format!("level = \"{}\"", l));
    }
    if let Some(t) = args.get("traceId").and_then(|v| v.as_str()) {
        parts.push(format!("traceId = \"{}\"", t));
    }
    if let Some(s) = args.get("since").and_then(|v| v.as_str()) {
        let cutoff = chrono::Utc::now().timestamp_millis() - parse_duration(s);
        parts.push(format!("timestampMs > {}", cutoff));
    }
    
    if parts.is_empty() { None } else { Some(parts.join(" AND ")) }
}

async fn search_logs(index: &meilisearch_sdk::indexes::Index, args: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let filter = build_filter(args);

    let mut search = index.search();
    search.with_query(query);
    search.with_limit(limit);
    search.with_sort(&["timestamp:desc"]);
    search.with_facets(meilisearch_sdk::search::Selectors::Some(&["project", "level"]));
    
    if let Some(ref f) = filter {
        search.with_filter(f.as_str());
    }

    match search.execute::<serde_json::Value>().await {
        Ok(r) => {
            let hits: Vec<serde_json::Value> = r.hits.into_iter().map(|h| h.result).collect();
            serde_json::to_string_pretty(&json!({
                "totalHits": r.estimated_total_hits,
                "facets": r.facet_distribution,
                "hits": hits
            })).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}

async fn get_trace(index: &meilisearch_sdk::indexes::Index, trace_id: &str) -> String {
    if trace_id.is_empty() {
        return "Error: traceId is required".to_string();
    }
    
    let filter = format!("traceId = \"{}\"", trace_id);
    let mut search = index.search();
    search.with_filter(&filter);
    search.with_sort(&["timestamp:asc"]);
    search.with_limit(500);

    match search.execute::<serde_json::Value>().await {
        Ok(r) => {
            let hits: Vec<serde_json::Value> = r.hits.into_iter().map(|h| h.result).collect();
            let projects: Vec<&str> = hits.iter()
                .filter_map(|h| h.get("project").and_then(|p| p.as_str()))
                .collect();
            
            serde_json::to_string_pretty(&json!({
                "traceId": trace_id,
                "eventCount": hits.len(),
                "projects": projects,
                "timeline": hits
            })).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}

async fn tail_logs(index: &meilisearch_sdk::indexes::Index, args: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
    let filter = build_filter(args);

    let mut search = index.search();
    search.with_sort(&["timestamp:desc"]);
    search.with_limit(limit);
    
    if let Some(ref f) = filter {
        search.with_filter(f.as_str());
    }

    match search.execute::<serde_json::Value>().await {
        Ok(r) => {
            let mut hits: Vec<serde_json::Value> = r.hits.into_iter().map(|h| h.result).collect();
            hits.reverse();
            serde_json::to_string_pretty(&json!({
                "count": hits.len(),
                "logs": hits
            })).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}

async fn list_projects(index: &meilisearch_sdk::indexes::Index) -> String {
    let mut search = index.search();
    search.with_facets(meilisearch_sdk::search::Selectors::Some(&["project", "level", "environment"]));
    search.with_limit(0);

    match search.execute::<serde_json::Value>().await {
        Ok(r) => serde_json::to_string_pretty(&json!({
            "totalLogs": r.estimated_total_hits,
            "byProject": r.facet_distribution.as_ref().and_then(|f| f.get("project")),
            "byLevel": r.facet_distribution.as_ref().and_then(|f| f.get("level")),
            "byEnvironment": r.facet_distribution.as_ref().and_then(|f| f.get("environment"))
        })).unwrap_or_default(),
        Err(e) => format!("Error: {}", e),
    }
}

async fn error_summary(index: &meilisearch_sdk::indexes::Index, args: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let since = args.get("since").and_then(|v| v.as_str()).unwrap_or("1h");
    let project = args.get("project").and_then(|v| v.as_str());
    
    let mut parts = vec!["level = \"error\" OR level = \"fatal\"".to_string()];
    
    if let Some(p) = project {
        parts.push(format!("project = \"{}\"", p));
    }
    
    let cutoff = chrono::Utc::now().timestamp_millis() - parse_duration(since);
    parts.push(format!("timestampMs > {}", cutoff));
    
    let filter = parts.join(" AND ");
    
    let mut search = index.search();
    search.with_filter(&filter);
    search.with_sort(&["timestamp:desc"]);
    search.with_limit(30);
    search.with_facets(meilisearch_sdk::search::Selectors::Some(&["project"]));

    match search.execute::<serde_json::Value>().await {
        Ok(r) => {
            let errors: Vec<serde_json::Value> = r.hits.into_iter().map(|h| h.result).collect();
            serde_json::to_string_pretty(&json!({
                "totalErrors": r.estimated_total_hits,
                "byProject": r.facet_distribution.as_ref().and_then(|f| f.get("project")),
                "recentErrors": errors
            })).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}

async fn find_similar(index: &meilisearch_sdk::indexes::Index, message: &str, args: &std::collections::HashMap<String, serde_json::Value>) -> String {
    if message.is_empty() {
        return "Error: message is required".to_string();
    }
    
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let filter = build_filter(args);

    let mut search = index.search();
    search.with_query(message);
    search.with_limit(limit);
    
    if let Some(ref f) = filter {
        search.with_filter(f.as_str());
    }

    match search.execute::<serde_json::Value>().await {
        Ok(r) => {
            let matches: Vec<serde_json::Value> = r.hits.into_iter().map(|h| h.result).collect();
            serde_json::to_string_pretty(&json!({
                "query": message,
                "matches": matches
            })).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}
