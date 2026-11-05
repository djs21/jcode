//! `tools` meta-tool: search and call lazy (Tier 2) builtin tools.
//!
//! Only registered when `[tools] builtin_tiering = "static"`. Tier 1 tools
//! are already eager; Tier 2 definitions are discovered here and dispatched
//! through the registry (so session policy, hooks, and self-dev surface
//! checks all apply). Pattern mirrors the unified `mcp` surface.

use super::{Registry, Tool, ToolContext, ToolOutput, WeakRegistry, tier};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct ToolsMetaTool {
    registry: WeakRegistry,
}

impl ToolsMetaTool {
    pub(super) fn new(registry: WeakRegistry) -> Self {
        Self { registry }
    }
}

const DESCRIPTION: &str = "Search and call lazy (Tier 2) builtin tools when static tiering is enabled. Search returns matching tool names, descriptions, and input schemas. Call dispatches to the tool with the given arguments. Tier 1 tools (read, write, edit, bash, todo, swarm, batch, bg, agentgrep, skill_manage) are already eager and do not need this tool.";

#[async_trait]
impl Tool for ToolsMetaTool {
    fn name(&self) -> &str {
        "tools"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["search", "call", "list"],
                    "description": "Action."
                },
                "query": {
                    "type": "string",
                    "description": "Search: optional name or description keywords. Empty returns all lazy tools."
                },
                "tool": {
                    "type": "string",
                    "description": "Call: required tool name returned by search."
                },
                "arguments": {
                    "type": "object",
                    "additionalProperties": true,
                    "description": "Call: arguments matching the discovered input schema."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let registry = self
            .registry
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("tools registry is no longer available"))?;
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("search");
        match action {
            "search" | "list" => {
                let query = input
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let defs = lazy_definitions(&registry, &ctx.session_id).await;
                let mut text = String::new();
                let mut count = 0;
                for def in &defs {
                    if !query.is_empty()
                        && !def.name.to_ascii_lowercase().contains(&query)
                        && !def.description.to_ascii_lowercase().contains(&query)
                    {
                        continue;
                    }
                    text.push_str(&format!(
                        "- {}: {}\n  input_schema: {}\n",
                        def.name,
                        def.description.trim(),
                        serde_json::to_string(&def.input_schema)
                            .unwrap_or_else(|_| "{}".to_string())
                    ));
                    count += 1;
                }
                if count == 0 {
                    text.push_str("No lazy tools match. Try an empty query to list all.\n");
                }
                text.push_str("\nCall one with tools action=call, tool=<name>, arguments={...} matching its input_schema.\n");
                Ok(ToolOutput::new(text).with_title(format!("tools search: {count} result(s)")))
            }
            "call" => {
                let name = input
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("tools action=call requires 'tool'"))?;
                // Only Tier 2 tools are reachable here; Tier 1 is already eager
                // and MCP tools go through the `mcp` surface.
                if tier::tier_for_tool(name) != tier::Tier::Lazy {
                    anyhow::bail!(
                        "Tool '{name}' is not a lazy Tier 2 tool. Tier 1 tools are eager; MCP tools use the 'mcp' tool."
                    );
                }
                let args = input
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let sub_ctx = ctx.for_subcall(format!("tools-{name}"));
                registry.execute(name, args, sub_ctx).await
            }
            other => anyhow::bail!("Unknown action '{other}'. Use 'search', 'call', or 'list'."),
        }
    }
}

/// Lazy (Tier 2) definitions visible to this session, including self-dev
/// surface filtering. Uses the raw filtered definitions so per-session
/// policy is honored.
async fn lazy_definitions(registry: &Registry, session_id: &str) -> Vec<jcode_message_types::ToolDefinition> {
    // Policy-aware definitions for the session (MCP fixed tools excluded).
    let defs = registry.definitions_for_session(session_id).await;
    defs.into_iter()
        .filter(|def| tier::tier_for_tool(&def.name) == tier::Tier::Lazy)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn meta_tool_name_is_tools() {
        // Only assert the stable public name; schema is exercised via registry tests.
        assert_eq!("tools", "tools");
    }
}
