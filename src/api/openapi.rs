/// OpenAPI 3.1 specification for the local HTTP API.
pub fn openapi_json(node_id: &str) -> String {
    let spec = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "DSearch Local API",
            "version": "0.1.0",
            "description": "Local HTTP API for DSearch node — all CLI and UI operations proxy through this surface."
        },
        "servers": [
            { "url": "http://127.0.0.1:7743", "description": "Local node API" }
        ],
        "paths": {
            "/health": {
                "get": {
                    "summary": "Health check",
                    "responses": {
                        "200": { "description": "Node is healthy", "content": { "application/json": { "schema": { "type": "object", "properties": { "status": { "type": "string" }, "node_id": { "type": "string" }, "uptime_secs": { "type": "number" } } } } } }
                    }
                }
            },
            "/node": {
                "get": {
                    "summary": "Node info",
                    "responses": {
                        "200": { "description": "Node identity and status", "content": { "application/json": { "schema": { "type": "object", "properties": { "node_id": { "type": "string" }, "role": { "type": "string" }, "protocol_version": { "type": "integer" }, "peers": { "type": "integer" }, "records": { "type": "integer" } } } } } }
                    }
                }
            },
            "/search": {
                "get": {
                    "summary": "Search records",
                    "parameters": [
                        { "name": "q", "in": "query", "required": true, "schema": { "type": "string" }, "description": "Search query" },
                        { "name": "schema", "in": "query", "schema": { "type": "string" }, "description": "Schema filter" },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 20 }, "description": "Max results" },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 }, "description": "Result offset" }
                    ],
                    "responses": {
                        "200": { "description": "Search results", "headers": { "X-Record-Count": { "schema": { "type": "integer" } } } }
                    }
                }
            },
            "/record/{id}": {
                "get": {
                    "summary": "Get a record by ID",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Record found" },
                        "404": { "description": "Record not found" }
                    }
                }
            },
            "/records": {
                "get": {
                    "summary": "List records",
                    "parameters": [
                        { "name": "schema", "in": "query", "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50 } },
                        { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
                    ],
                    "responses": {
                        "200": { "description": "Record list" }
                    }
                }
            },
            "/schema": {
                "get": {
                    "summary": "List known schemas",
                    "responses": { "200": { "description": "Schema list" } }
                }
            },
            "/schema/{id}": {
                "get": {
                    "summary": "Get schema details",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "Schema details" }, "404": { "description": "Unknown schema" } }
                }
            },
            "/peers": {
                "get": {
                    "summary": "List known peers",
                    "responses": { "200": { "description": "Peer list" } }
                }
            },
            "/peers/add": {
                "post": {
                    "summary": "Add a peer",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "addr": { "type": "string" } }, "required": ["addr"] } } } },
                    "responses": { "200": { "description": "Peer added" } }
                }
            },
            "/scraper": {
                "get": {
                    "summary": "List scraper jobs",
                    "responses": { "200": { "description": "Scraper job list" } }
                }
            },
            "/scraper/run": {
                "post": {
                    "summary": "Run a scraper job",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] } } } },
                    "responses": { "200": { "description": "Job result" } }
                }
            },
            "/storage": {
                "get": {
                    "summary": "Storage info",
                    "responses": { "200": { "description": "Storage statistics" } }
                }
            },
            "/storage/vacuum": {
                "post": {
                    "summary": "Vacuum storage",
                    "responses": { "200": { "description": "Vacuum complete" } }
                }
            },
            "/config": {
                "get": {
                    "summary": "Get current config",
                    "responses": { "200": { "description": "Config object" } }
                }
            },
            "/config/set": {
                "post": {
                    "summary": "Set a config key",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "key": { "type": "string" }, "value": { "type": "string" } }, "required": ["key", "value"] } } } },
                    "responses": { "200": { "description": "Config updated" } }
                }
            },
            "/identity": {
                "get": {
                    "summary": "Show node identity",
                    "responses": { "200": { "description": "Identity info" } }
                }
            },
            "/bootstrap": {
                "get": {
                    "summary": "List bootstrap peers",
                    "responses": { "200": { "description": "Bootstrap peer list" } }
                }
            },
            "/openapi.json": {
                "get": {
                    "summary": "OpenAPI specification",
                    "responses": { "200": { "description": "OpenAPI 3.1 JSON" } }
                }
            }
        },
        "components": {
            "schemas": {
                "error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" },
                        "code": { "type": "integer" }
                    }
                }
            }
        },
        "x-node-id": node_id
    });
    serde_json::to_string_pretty(&spec).unwrap_or_default()
}
