//! Static tiering for builtin tools (Opsi A, see docs/LAZY_TOOL_DEFINITIONS_SPEC.md).
//!
//! Tier 1 tools stay eager in the cached prompt prefix; Tier 2 tools are
//! lazy-loaded on demand via the `tools` meta-tool (search/call) or via
//! provider-native deferred loading (`defer_loading`).

use std::collections::HashSet;

/// Tier of a builtin tool under static tiering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Eager,
    Lazy,
}

/// Tools that stay eager: daily coding workflow.
pub const TIER1_EAGER_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "apply_patch",
    "bash",
    "todo",
    "swarm",
    "batch",
    "bg",
    "agentgrep",
    "skill_manage",
];

/// Tools that are lazy in static tiering.
pub const TIER2_LAZY_TOOLS: &[&str] = &[
    "browser",
    "calendar",
    "codemode",
    "compile_remote",
    "conversation_search",
    "debug_socket",
    "desktop_selfdev",
    "gmail",
    "integration_tools",
    "jcode_docs",
    "maintainer_feedback",
    "memory",
    "open",
    "panel",
    "schedule",
    "selfdev",
    "session_search",
    "side_panel",
    "webfetch",
    "websearch",
];

/// Determine the tier for a tool name.
///
/// MCP tools (`mcp__*`, `mcp_search`, `mcp_call`) and the `tools` meta-tool
/// itself are never tiered here; they follow the MCP exposure path.
pub fn tier_for_tool(name: &str) -> Tier {
    if name.starts_with("mcp__") || matches!(name, "mcp_search" | "mcp_call" | "tools") {
        return Tier::Eager;
    }
    if TIER1_EAGER_TOOLS.contains(&name) {
        Tier::Eager
    } else if TIER2_LAZY_TOOLS.contains(&name) {
        Tier::Lazy
    } else {
        // Unknown / custom tools default eager to avoid breaking them.
        Tier::Eager
    }
}

/// Apply tiering to a definitions list, honoring config overrides.
///
/// `eager_tools` forces a tool eager; `lazy_tools` forces it lazy.
/// Explicit overrides win over the static table.
pub fn apply_tiering(
    tools: &mut [jcode_message_types::ToolDefinition],
    eager_overrides: &[String],
    lazy_overrides: &[String],
) {
    let eager_set: HashSet<&str> = eager_overrides.iter().map(String::as_str).collect();
    let lazy_set: HashSet<&str> = lazy_overrides.iter().map(String::as_str).collect();
    for tool in tools.iter_mut() {
        let tier = if eager_set.contains(tool.name.as_str()) {
            Tier::Eager
        } else if lazy_set.contains(tool.name.as_str()) {
            Tier::Lazy
        } else {
            tier_for_tool(&tool.name)
        };
        tool.defer_loading = tier == Tier::Lazy;
    }
}

/// Names of Tier 2 tools, for the system-prompt note.
pub fn lazy_tool_names() -> Vec<&'static str> {
    TIER2_LAZY_TOOLS.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier1_is_eager() {
        for name in TIER1_EAGER_TOOLS {
            assert_eq!(tier_for_tool(name), Tier::Eager, "{name}");
        }
    }

    #[test]
    fn tier2_is_lazy() {
        for name in TIER2_LAZY_TOOLS {
            assert_eq!(tier_for_tool(name), Tier::Lazy, "{name}");
        }
    }

    #[test]
    fn mcp_and_meta_are_never_lazy_here() {
        assert_eq!(tier_for_tool("mcp"), Tier::Eager);
        assert_eq!(tier_for_tool("mcp__srv__tool"), Tier::Eager);
        assert_eq!(tier_for_tool("mcp_search"), Tier::Eager);
        assert_eq!(tier_for_tool("tools"), Tier::Eager);
    }

    #[test]
    fn unknown_defaults_eager() {
        assert_eq!(tier_for_tool("some_custom_tool"), Tier::Eager);
    }

    #[test]
    fn overrides_win() {
        let mut tools = vec![
            jcode_message_types::ToolDefinition {
                name: "browser".to_string(),
                description: "b".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
            jcode_message_types::ToolDefinition {
                name: "read".to_string(),
                description: "r".to_string(),
                input_schema: serde_json::json!({}),
                defer_loading: false,
            },
        ];
        apply_tiering(&mut tools, &["browser".to_string()], &["read".to_string()]);
        assert!(!tools[0].defer_loading);
        assert!(tools[1].defer_loading);
    }
}
