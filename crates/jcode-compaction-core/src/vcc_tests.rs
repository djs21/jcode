#[cfg(test)]
mod tests {
    use crate::vcc::*;
    use jcode_message_types::{ContentBlock, Message, Role};

    #[test]
    fn test_vcc_summary_extraction() {
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Please fix the authentication bug in login page".into(),
                    cache_control: None,
                }],
                timestamp: None,
                tool_duration_ms: None,
            },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Text {
                        text: "I will check the auth files first.".into(),
                        cache_control: None,
                    },
                    ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "read".into(),
                        input: serde_json::json!({ "file_path": "src/auth/session.rs" }),
                        thought_signature: None,
                    },
                ],
                timestamp: None,
                tool_duration_ms: None,
            },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::ToolUse {
                        id: "call_2".into(),
                        name: "edit".into(),
                        input: serde_json::json!({ "file_path": "src/auth/session.rs" }),
                        thought_signature: None,
                    },
                    ContentBlock::ToolUse {
                        id: "call_3".into(),
                        name: "bash".into(),
                        input: serde_json::json!({ "command": "git commit -m \"fix: auth refresh bug\"" }),
                        thought_signature: None,
                    },
                ],
                timestamp: None,
                tool_duration_ms: None,
            },
        ];

        let summary = compile_vcc_summary(&messages, None, &[]);
        assert!(summary.contains("[Early Session Requests]"));
        assert!(summary.contains("Please fix the authentication bug in login page"));
        assert!(summary.contains("[Files And Changes]"));
        assert!(summary.contains("src/auth/session.rs"));
        assert!(summary.contains("[Commits]"));
        assert!(summary.contains("fix: auth refresh bug"));
        assert!(summary.contains("Do not redo work already completed"));
    }

    #[test]
    fn test_vcc_incremental_merge() {
        let messages_turn_1 = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Initial user instruction".into(),
                    cache_control: None,
                }],
                timestamp: None,
                tool_duration_ms: None,
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "write".into(),
                    input: serde_json::json!({ "file_path": "src/main.rs" }),
                    thought_signature: None,
                }],
                timestamp: None,
                tool_duration_ms: None,
            },
        ];

        let summary_1 = compile_vcc_summary(&messages_turn_1, None, &[]);
        assert!(summary_1.contains("Initial user instruction"));
        assert!(summary_1.contains("src/main.rs"));

        let messages_turn_2 = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Second request: add unit tests".into(),
                    cache_control: None,
                }],
                timestamp: None,
                tool_duration_ms: None,
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "call_2".into(),
                    name: "write".into(),
                    input: serde_json::json!({ "file_path": "tests/test.rs" }),
                    thought_signature: None,
                }],
                timestamp: None,
                tool_duration_ms: None,
            },
        ];

        let summary_2 = compile_vcc_summary(&messages_turn_2, Some(&summary_1), &[]);
        // Early session request must remain the first one
        assert!(summary_2.contains("[Early Session Requests]"));
        assert!(summary_2.contains("Initial user instruction"));
        // User messages section should have the second request
        assert!(summary_2.contains("[User Messages]"));
        assert!(summary_2.contains("Second request: add unit tests"));
        // Both files should be recorded
        assert!(summary_2.contains("src/main.rs"));
        assert!(summary_2.contains("tests/test.rs"));
    }
}
