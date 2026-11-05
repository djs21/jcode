use jcode_message_types::{ContentBlock, Message, Role};
use std::collections::HashSet;

pub const EARLY_REQUESTS_KEPT: usize = 3;
pub const USER_MESSAGES_KEPT: usize = 15;
pub const COMMITS_KEPT: usize = 8;
pub const FILES_PER_CATEGORY: usize = 10;
pub const BRIEF_MAX_LINES: usize = 120;
pub const SEPARATOR: &str = "\n\n---\n\n";

#[derive(Debug, Clone, Default)]
pub struct VccSectionData {
    pub early_requests: Vec<String>,
    pub files_modified: Vec<String>,
    pub files_created: Vec<String>,
    pub files_read: Vec<String>,
    pub commits: Vec<String>,
    pub tracked_commands: Vec<String>,
    pub user_messages: Vec<String>,
    pub brief_transcript: String,
}

/// Strip common leading filler words
fn strip_filler_prefix(text: &str) -> &str {
    let lower = text.trim_start();
    let fillers = ["hmm", "wait", "actually", "oh", "okay", "ok", "well", "so"];
    for f in fillers {
        if let Some(rest) = lower.strip_prefix(f) {
            let rest = rest.trim_start();
            if rest.starts_with(',') || rest.starts_with('.') || rest.starts_with('!') || rest.starts_with('-') || rest.starts_with(':') {
                return rest[1..].trim_start();
            }
        }
    }
    lower
}

/// Extract clean one-line user message text
pub fn extract_user_line(text: &str, max_chars: usize) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = flat.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') {
        return None;
    }
    // Need at least 2 words or 5 chars
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() < 2 && trimmed.chars().count() < 5 {
        return None;
    }
    Some(clip_str(trimmed, max_chars))
}

pub fn clip_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut res: String = s.chars().take(max_chars).collect();
        res.push_str("...");
        res
    }
}

/// Cap a list of items with (+N earlier) marker
pub fn cap_items(items: &[String], limit: usize, join_with: &str) -> String {
    if items.len() <= limit {
        items.join(join_with)
    } else {
        let omitted = items.len() - limit;
        format!("(+{} earlier) {}", omitted, items[omitted..].join(join_with))
    }
}

/// Strip cap marker from a line e.g. "(+N earlier) "
pub fn strip_cap_marker(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("(+") {
        if let Some(idx) = rest.find(" earlier)") {
            return rest[idx + " earlier)".len()..].trim_start();
        }
    }
    trimmed
}

/// Extract file paths mentioned in ToolUse / ToolResult blocks
pub fn extract_file_ops_from_messages(messages: &[Message]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut modified = Vec::new();
    let mut created = Vec::new();
    let mut read = Vec::new();

    let mut seen_mod = HashSet::new();
    let mut seen_create = HashSet::new();
    let mut seen_read = HashSet::new();

    for msg in messages {
        for block in &msg.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                let name_lower = name.to_ascii_lowercase();
                let path = extract_path_from_json(input);
                if let Some(p) = path {
                    match name_lower.as_str() {
                        "read" | "read_file" | "view" => {
                            if seen_read.insert(p.clone()) {
                                read.push(p);
                            }
                        }
                        "edit" | "edit_file" | "patch" | "apply_patch" | "multiedit" => {
                            if seen_mod.insert(p.clone()) {
                                modified.push(p);
                            }
                        }
                        "write" | "write_file" => {
                            if seen_create.insert(p.clone()) {
                                created.push(p);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Dedup: if created was later modified, keep as modified or created?
    // In VCC, if already modified, drop from created.
    let mod_set: HashSet<String> = modified.iter().cloned().collect();
    created.retain(|p| !mod_set.contains(p));

    (modified, created, read)
}

fn extract_path_from_json(val: &serde_json::Value) -> Option<String> {
    if let serde_json::Value::Object(map) = val {
        for key in ["file_path", "path", "file", "target", "filename"] {
            if let Some(serde_json::Value::String(s)) = map.get(key) {
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
    }
    None
}

/// Extract git commits from tool inputs / outputs
pub fn extract_commits_from_messages(messages: &[Message]) -> Vec<String> {
    let mut commits = Vec::new();
    let mut seen = HashSet::new();

    for msg in messages {
        for block in &msg.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                if name == "bash" {
                    if let Some(cmd) = input.get("command").and_then(|c| c.as_str()) {
                        if cmd.contains("git commit") {
                            // Extract commit message from command if present
                            if let Some(m) = extract_commit_message_from_cmd(cmd) {
                                if seen.insert(m.clone()) {
                                    commits.push(m);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    commits
}

fn extract_commit_message_from_cmd(cmd: &str) -> Option<String> {
    // Look for -m "..." or -m '...'
    if let Some(idx) = cmd.find("-m") {
        let rest = cmd[idx + 2..].trim_start();
        if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                let msg = &rest[1..=end];
                return Some(msg.trim().to_string());
            }
        } else if rest.starts_with('\'') {
            if let Some(end) = rest[1..].find('\'') {
                let msg = &rest[1..=end];
                return Some(msg.trim().to_string());
            }
        }
    }
    None
}

/// Extract tracked commands (e.g. cargo, test, docker, etc.)
pub fn extract_tracked_commands(messages: &[Message], prefixes: &[String]) -> Vec<String> {
    if prefixes.is_empty() {
        return Vec::new();
    }
    let mut tracked = Vec::new();
    let mut seen = HashSet::new();

    for msg in messages {
        for block in &msg.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                if name == "bash" {
                    if let Some(cmd) = input.get("command").and_then(|c| c.as_str()) {
                        let trimmed = cmd.trim();
                        for prefix in prefixes {
                            if trimmed.starts_with(prefix) {
                                let line = clip_str(trimmed, 80);
                                if seen.insert(line.clone()) {
                                    tracked.push(line);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    tracked
}

/// Build brief transcript lines from messages
pub fn build_brief_transcript(messages: &[Message], start_idx: usize) -> String {
    let mut lines = Vec::new();
    let mut turn_counter = start_idx;

    for msg in messages {
        turn_counter += 1;
        match msg.role {
            Role::User => {
                let mut text_parts = Vec::new();
                for block in &msg.content {
                    if let ContentBlock::Text { text, .. } = block {
                        if let Some(line) = extract_user_line(text, 250) {
                            text_parts.push(line);
                        }
                    }
                }
                if !text_parts.is_empty() {
                    lines.push(format!("[user] (turn #{})", turn_counter));
                    for t in text_parts {
                        lines.push(t);
                    }
                    lines.push(String::new());
                }
            }
            Role::Assistant => {
                let mut asst_lines = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text, .. } => {
                            let clean = strip_filler_prefix(text);
                            let flat = clean.split_whitespace().collect::<Vec<_>>().join(" ");
                            if !flat.is_empty() {
                                asst_lines.push(clip_str(&flat, 180));
                            }
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let arg_summary = match input {
                                serde_json::Value::Object(map) => {
                                    if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                                        format!("\"{}\"", clip_str(cmd, 60))
                                    } else if let Some(p) = map.get("file_path").or_else(|| map.get("path")).and_then(|v| v.as_str()) {
                                        format!("\"{}\"", p)
                                    } else if let Some(q) = map.get("query").and_then(|v| v.as_str()) {
                                        format!("query: \"{}\"", clip_str(q, 40))
                                    } else {
                                        String::new()
                                    }
                                }
                                _ => String::new(),
                            };
                            if arg_summary.is_empty() {
                                asst_lines.push(format!("* {} (#{})", name, turn_counter));
                            } else {
                                asst_lines.push(format!("* {} {} (#{})", name, arg_summary, turn_counter));
                            }
                        }
                        _ => {}
                    }
                }
                if !asst_lines.is_empty() {
                    lines.push(format!("[assistant] (turn #{})", turn_counter));
                    for l in asst_lines {
                        lines.push(l);
                    }
                    lines.push(String::new());
                }
            }
        }
    }

    // Limit lines to BRIEF_MAX_LINES
    if lines.len() > BRIEF_MAX_LINES {
        let omitted = lines.len() - BRIEF_MAX_LINES;
        let kept = &lines[omitted..];
        format!("...({} earlier lines omitted)\n\n{}", omitted, kept.join("\n").trim())
    } else {
        lines.join("\n").trim().to_string()
    }
}

/// Format section into markdown block
fn format_section(title: &str, items: &[String]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let body = items.iter().map(|i| format!("- {}", i)).collect::<Vec<_>>().join("\n");
    Some(format!("[{}]\n{}", title, body))
}

/// Compile a VCC summary from messages and optional previous summary text
pub fn compile_vcc_summary(
    messages: &[Message],
    previous_summary: Option<&str>,
    tracked_command_prefixes: &[String],
) -> String {
    let (prev_early, mut prev_mod, mut prev_created, mut prev_read, mut prev_commits, mut prev_cmds, mut prev_user) =
        parse_existing_vcc_summary(previous_summary.unwrap_or_default());

    // Extract user messages
    let mut fresh_user_messages = Vec::new();
    for msg in messages {
        if msg.role == Role::User {
            for block in &msg.content {
                if let ContentBlock::Text { text, .. } = block {
                    if let Some(line) = extract_user_line(text, 300) {
                        fresh_user_messages.push(line);
                    }
                }
            }
        }
    }

    // Early requests: pinned from previous summary or first EARLY_REQUESTS_KEPT messages
    let early_requests = if !prev_early.is_empty() {
        prev_early.clone()
    } else {
        fresh_user_messages.iter().take(EARLY_REQUESTS_KEPT).cloned().collect()
    };

    // Subsequent user messages
    let start_user_idx = if prev_early.is_empty() {
        EARLY_REQUESTS_KEPT.min(fresh_user_messages.len())
    } else {
        0
    };
    for u in &fresh_user_messages[start_user_idx..] {
        if !prev_user.contains(u) {
            prev_user.push(u.clone());
        }
    }
    if prev_user.len() > USER_MESSAGES_KEPT {
        let keep_start = prev_user.len() - USER_MESSAGES_KEPT;
        prev_user = prev_user[keep_start..].to_vec();
    }

    // Extract file activity
    let (fresh_mod, fresh_created, fresh_read) = extract_file_ops_from_messages(messages);
    for m in fresh_mod {
        if !prev_mod.contains(&m) {
            prev_mod.push(m);
        }
    }
    for c in fresh_created {
        if !prev_created.contains(&c) {
            prev_created.push(c);
        }
    }
    for r in fresh_read {
        if !prev_read.contains(&r) {
            prev_read.push(r);
        }
    }

    // Cap files per category
    let mut file_lines = Vec::new();
    if !prev_mod.is_empty() {
        file_lines.push(format!("Modified: {}", cap_items(&prev_mod, FILES_PER_CATEGORY, ", ")));
    }
    if !prev_created.is_empty() {
        file_lines.push(format!("Created: {}", cap_items(&prev_created, FILES_PER_CATEGORY, ", ")));
    }
    if !prev_read.is_empty() {
        file_lines.push(format!("Read: {}", cap_items(&prev_read, FILES_PER_CATEGORY, ", ")));
    }

    // Extract commits
    let fresh_commits = extract_commits_from_messages(messages);
    for cm in fresh_commits {
        if !prev_commits.contains(&cm) {
            prev_commits.push(cm);
        }
    }
    if prev_commits.len() > COMMITS_KEPT {
        let keep_start = prev_commits.len() - COMMITS_KEPT;
        prev_commits = prev_commits[keep_start..].to_vec();
    }

    // Tracked commands
    let fresh_cmds = extract_tracked_commands(messages, tracked_command_prefixes);
    for cmd in fresh_cmds {
        if !prev_cmds.contains(&cmd) {
            prev_cmds.push(cmd);
        }
    }
    if prev_cmds.len() > 10 {
        let keep_start = prev_cmds.len() - 10;
        prev_cmds = prev_cmds[keep_start..].to_vec();
    }

    // Transcript
    let brief_transcript = build_brief_transcript(messages, 0);

    // Assemble sections
    let mut sections = Vec::new();
    if let Some(s) = format_section("Early Session Requests", &early_requests) {
        sections.push(s);
    }
    if let Some(s) = format_section("Files And Changes", &file_lines) {
        sections.push(s);
    }
    if let Some(s) = format_section("Commits", &prev_commits) {
        sections.push(s);
    }
    if let Some(s) = format_section("Tracked Commands", &prev_cmds) {
        sections.push(s);
    }
    if let Some(s) = format_section("User Messages", &prev_user) {
        sections.push(s);
    }

    let head = sections.join("\n\n");
    let mut out = head;
    if !brief_transcript.is_empty() {
        out.push_str(SEPARATOR);
        out.push_str(&brief_transcript);
    }

    out.push_str(SEPARATOR);
    out.push_str("Use session search or conversation history if you need exact details from before this summary. Do not redo work already completed.");

    out
}

/// Parse previously compiled VCC summary
fn parse_existing_vcc_summary(
    text: &str,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let mut early = Vec::new();
    let mut modified = Vec::new();
    let mut created = Vec::new();
    let mut read = Vec::new();
    let mut commits = Vec::new();
    let mut tracked = Vec::new();
    let mut user = Vec::new();

    let parts: Vec<&str> = text.split(SEPARATOR).collect();
    let head = parts.first().copied().unwrap_or_default();

    let mut current_section = "";
    for line in head.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = &trimmed[1..trimmed.len() - 1];
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            let item_clean = item.trim();
            match current_section {
                "Early Session Requests" => early.push(item_clean.to_string()),
                "Files And Changes" => {
                    if let Some(rest) = item_clean.strip_prefix("Modified: ") {
                        let uncap = strip_cap_marker(rest);
                        for p in uncap.split(',') {
                            let p = p.trim();
                            if !p.is_empty() {
                                modified.push(p.to_string());
                            }
                        }
                    } else if let Some(rest) = item_clean.strip_prefix("Created: ") {
                        let uncap = strip_cap_marker(rest);
                        for p in uncap.split(',') {
                            let p = p.trim();
                            if !p.is_empty() {
                                created.push(p.to_string());
                            }
                        }
                    } else if let Some(rest) = item_clean.strip_prefix("Read: ") {
                        let uncap = strip_cap_marker(rest);
                        for p in uncap.split(',') {
                            let p = p.trim();
                            if !p.is_empty() {
                                read.push(p.to_string());
                            }
                        }
                    }
                }
                "Commits" => commits.push(item_clean.to_string()),
                "Tracked Commands" => tracked.push(item_clean.to_string()),
                "User Messages" => user.push(item_clean.to_string()),
                _ => {}
            }
        }
    }

    (early, modified, created, read, commits, tracked, user)
}
