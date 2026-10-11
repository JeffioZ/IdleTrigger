//! Explorer context-menu rule model, validation, and pure helpers.
//!
//! Menu entries themselves live in the registry (static verbs owned by the
//! tray app); this module owns the `[[ctx_rules]]` document section and the
//! pure translations the registry sync consumes: pattern lists to
//! `AppliesTo` filters, scopes to verb roots, and copy-path formatting.

use crate::toml_rules::{self, Entry, Output};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use toml_edit::ArrayOfTables;
use toml_edit::{DocumentMut, Item, Table};

/// Registry verb names are prefixed so the sync engine can find and clean
/// every key it owns by prefix alone.
pub const VERB_PREFIX: &str = "IdleTrigger";

pub const MAX_CTX_RULES: usize = 32;
/// Registry key-name fragment cap for rule ids.
const MAX_ID_LEN: usize = 40;

pub const SCOPE_FILE: &str = "file";
pub const SCOPE_DIR: &str = "dir";
pub const SCOPE_FILE_DIR: &str = "file_dir";
pub const SCOPE_BACKGROUND: &str = "background";
pub const SCOPE_ALL: &str = "all";

/// Missing fields keep the historical one-invocation-per-item behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    Single,
    #[default]
    Each,
    Together,
}

pub const QUOTE_AUTO: &str = "auto";
pub const QUOTE_ALWAYS: &str = "always";
pub const QUOTE_NONE: &str = "none";

pub const SEPARATOR_NEWLINE: &str = "newline";
pub const SEPARATOR_SPACE: &str = "space";

pub const COPY_FORMAT_FULL: &str = "full";
pub const COPY_FORMAT_NAME: &str = "name";
pub const COPY_FORMAT_DIR: &str = "dir";
/// File name without its extension ("report" for "report.pdf").
pub const COPY_FORMAT_STEM: &str = "stem";
/// Extension with the leading dot (".pdf").
pub const COPY_FORMAT_EXT: &str = "ext";
/// For .lnk selections: the resolved target path (falls back to the full
/// path for regular files — static verbs cannot hide items per type).
pub const COPY_FORMAT_TARGET: &str = "target";

/// One custom "open with" menu rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CtxRule {
    pub id: String,
    pub title: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    /// Extension patterns such as `*.exe`; empty = every item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    pub command: String,
    /// Argument template; `%1`/`%L`/`%V` substitute the selected path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    /// Custom icon file; empty = derive from the target program.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden_console: bool,
    /// Separator row (only renders inside the rules submenu; top-level
    /// static verbs cannot draw separator lines).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub separator: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub working_dir: String,
    #[serde(default)]
    pub selection_mode: SelectionMode,
}

fn default_true() -> bool {
    true
}

impl Default for CtxRule {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            enabled: true,
            scope: SCOPE_FILE.to_string(),
            patterns: Vec::new(),
            command: String::new(),
            args: String::new(),
            icon: String::new(),
            hidden_console: false,
            separator: false,
            working_dir: String::new(),
            selection_mode: SelectionMode::Each,
        }
    }
}

/// Why one configured rule cannot be registered as-is. `code` drives the
/// UI's localized line; `message` stays English for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct CtxIssue {
    pub index: usize,
    pub code: IssueCode,
    pub message: String,
}

/// Stable classification so the manager can translate issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCode {
    TitleRequired,
    CommandRequired,
    PatternInvalid,
    ArgumentsInvalid,
    TooMany,
    /// A malformed [[ctx_rules]] entry (the message carries the decode error).
    Parse,
}

pub fn valid_scope(scope: &str) -> bool {
    matches!(
        scope,
        SCOPE_FILE | SCOPE_DIR | SCOPE_FILE_DIR | SCOPE_BACKGROUND | SCOPE_ALL
    )
}

/// HKCR roots (minus the `Software\Classes` prefix) a scope registers under.
pub fn scope_roots(scope: &str) -> &'static [&'static str] {
    match scope {
        SCOPE_DIR => &["Directory"],
        SCOPE_FILE_DIR => &["*", "Directory"],
        SCOPE_BACKGROUND => &["Directory\\Background"],
        SCOPE_ALL => &["*", "Directory", "Directory\\Background"],
        // Unknown scopes were sanitized to file-only before this point.
        _ => &["*"],
    }
}

/// Splits a user-typed pattern field ("*.exe; *.dll") into trimmed tokens.
pub fn patterns_from_text(text: &str) -> Vec<String> {
    text.split([';', ',', ' ', '\t'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// `*.exe`/`.exe`/`exe` → normalized extension `exe`; anything the property
/// system cannot express (other wildcards, path fragments) yields `None`.
fn extension_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("*") || token.eq_ignore_ascii_case("*.*") {
        // "Everything" collapses the whole filter to "always visible".
        return Some(String::new());
    }
    let stripped = token
        .strip_prefix('*')
        .map(|rest| rest.strip_prefix('.').unwrap_or(rest))
        .or_else(|| token.strip_prefix('.'))
        .unwrap_or(token);
    if stripped.is_empty()
        || !stripped
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(stripped.to_ascii_lowercase())
}

/// Translates pattern tokens into an `AppliesTo` AQS expression. `None`
/// means "show the verb unfiltered" for an empty list or `*`. Unsupported
/// tokens are rejected by validation and fail closed at invocation time.
pub fn applies_to_aqs(patterns: &[String]) -> Option<String> {
    let mut extensions: Vec<String> = Vec::new();
    for token in patterns {
        match extension_token(token) {
            Some(ext) if ext.is_empty() => return None,
            Some(ext) => {
                if !extensions.contains(&ext) {
                    extensions.push(ext);
                }
            }
            None => return None,
        }
    }
    if extensions.is_empty() {
        return None;
    }
    Some(
        extensions
            .iter()
            .map(|ext| format!("System.FileExtension:=\".{ext}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// Invoke-time check mirroring [`applies_to_aqs`]: does the selection match
/// the rule's extension patterns? Empty patterns match everything.
pub fn extension_matches(patterns: &[String], path: &std::path::Path) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let actual = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    patterns.iter().any(|token| match extension_token(token) {
        Some(ext) => ext.is_empty() || ext == actual,
        None => false,
    })
}

/// Wraps one path per the copy-path quote policy.
fn quoted(path: &str, quote: &str) -> String {
    match quote {
        QUOTE_ALWAYS => format!("\"{path}\""),
        QUOTE_NONE => path.to_string(),
        // auto: quotes only where a shell would need them.
        _ => {
            if path.contains([' ', '\t']) {
                format!("\"{path}\"")
            } else {
                path.to_string()
            }
        }
    }
}

/// Joins aggregated multi-selection paths for one clipboard write.
pub fn format_paths(paths: &[String], quote: &str, separator: &str) -> String {
    let joiner = if separator == SEPARATOR_SPACE {
        " "
    } else {
        "\r\n"
    };
    paths
        .iter()
        .map(|path| quoted(path, quote))
        .collect::<Vec<_>>()
        .join(joiner)
}

/// Replaces `%1`/`%L`/`%V` in an argument template with the raw selected
/// path; the template supplies its own quoting (`"%1"`).
pub fn substitute_args(args: &str, path: &str) -> String {
    let mut output = String::new();
    let mut chars = args.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' && matches!(chars.peek(), Some('1' | 'L' | 'V')) {
            chars.next();
            output.push_str(path);
            // A quoted drive root must not escape its closing quote in
            // CommandLineToArgvW / the Microsoft CRT argument parser.
            if chars.peek() == Some(&'"') {
                output.extend(path.chars().rev().take_while(|c| *c == '\\'));
            }
        } else {
            output.push(ch);
        }
    }
    output
}

/// Expand a complete selection once, without interpreting tokens in paths.
/// `%*` must occupy a whole argument; optional surrounding quotes are accepted.
pub fn selection_args(args: &str, paths: &[String]) -> Option<String> {
    let first = paths.first()?;
    let all = paths
        .iter()
        .map(|path| {
            let mut quoted = String::from("\"");
            let mut slashes = 0;
            for ch in path.chars() {
                if ch == '\\' {
                    slashes += 1;
                    continue;
                }
                quoted.extend(std::iter::repeat_n(
                    '\\',
                    if ch == '"' { slashes * 2 + 1 } else { slashes },
                ));
                quoted.push(ch);
                slashes = 0;
            }
            quoted.extend(std::iter::repeat_n('\\', slashes * 2));
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut output = String::new();
    let mut rest = args;
    let mut boundary = true;
    let mut in_quotes = false;
    let mut slashes = 0;
    while !rest.is_empty() {
        let token_len = if rest.starts_with("\"%*\"") {
            4
        } else if rest.starts_with("%*") {
            2
        } else {
            0
        };
        if token_len > 0 {
            let tail = &rest[token_len..];
            if in_quotes || !boundary || tail.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
                return None;
            }
            output.push_str(&all);
            rest = tail;
            boundary = false;
            slashes = 0;
        } else if ["%1", "%L", "%V"]
            .iter()
            .any(|token| rest.starts_with(token))
        {
            output.push_str(first);
            rest = &rest[2..];
            if rest.starts_with('"') {
                output.extend(first.chars().rev().take_while(|ch| *ch == '\\'));
            }
            boundary = false;
            slashes = 0;
        } else {
            let ch = rest.chars().next()?;
            output.push(ch);
            rest = &rest[ch.len_utf8()..];
            if ch == '"' && slashes % 2 == 0 {
                in_quotes = !in_quotes;
            }
            slashes = if ch == '\\' { slashes + 1 } else { 0 };
            boundary = !in_quotes && ch.is_whitespace();
        }
    }
    Some(output)
}

/// Use the same selection contract for validation, preview and execution.
pub fn rule_selection_args(rule: &CtxRule, paths: &[String]) -> Option<String> {
    if rule.selection_mode == SelectionMode::Together
        && rule.scope != SCOPE_BACKGROUND
        && !rule.args.contains("%*")
    {
        return None;
    }
    selection_args(&rule.args, paths)
}

/// Slugs a rule id into a registry-key-safe fragment.
fn slug(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        String::new()
    } else {
        cleaned.chars().take(MAX_ID_LEN).collect()
    }
}

/// Trims fields, repairs scopes and ids, and enforces the rule cap and
/// uniqueness. Returns the rules plus issues for entries missing a title or
/// command (the UI blocks saving those).
pub fn prepare_rules(rules: Vec<CtxRule>) -> (Vec<CtxRule>, Vec<CtxIssue>) {
    let mut issues = Vec::new();
    let mut prepared = Vec::with_capacity(rules.len());
    let mut used_ids: Vec<String> = Vec::new();
    for (index, mut rule) in rules.into_iter().enumerate() {
        rule.title = rule.title.trim().to_string();
        rule.command = rule.command.trim().to_string();
        rule.args = rule.args.trim().to_string();
        rule.icon = rule.icon.trim().to_string();
        rule.working_dir = rule.working_dir.trim().to_string();
        rule.patterns = patterns_from_text(&rule.patterns.join(";"));
        if !valid_scope(&rule.scope) {
            rule.scope = SCOPE_FILE.to_string();
        }
        let base = slug(&rule.id);
        let mut id = if base.is_empty() {
            format!("rule{}", index + 1)
        } else {
            base.clone()
        };
        let mut suffix = 2;
        while used_ids.iter().any(|used| used.eq_ignore_ascii_case(&id)) {
            let seed = if base.is_empty() {
                format!("rule{}", index + 1)
            } else {
                base.clone()
            };
            id = format!("{seed}-{suffix}");
            suffix += 1;
        }
        rule.id = id.clone();
        used_ids.push(id);
        if !rule.separator {
            if rule_selection_args(&rule, &[String::from("sample")]).is_none() {
                issues.push(CtxIssue {
                    index,
                    code: IssueCode::ArgumentsInvalid,
                    message: format!("ctx_rules[{index}]: batch arguments require a standalone %*"),
                });
            }
            if rule
                .patterns
                .iter()
                .any(|pattern| extension_token(pattern).is_none())
            {
                issues.push(CtxIssue {
                    index,
                    code: IssueCode::PatternInvalid,
                    message: format!("ctx_rules[{index}]: use simple extensions such as *.txt"),
                });
            }
            if rule.title.is_empty() {
                issues.push(CtxIssue {
                    index,
                    code: IssueCode::TitleRequired,
                    message: format!("ctx_rules[{index}]: title is required"),
                });
            }
            if rule.command.is_empty() {
                issues.push(CtxIssue {
                    index,
                    code: IssueCode::CommandRequired,
                    message: format!("ctx_rules[{index}]: command is required"),
                });
            }
        }
        prepared.push(rule);
    }
    if prepared.len() > MAX_CTX_RULES {
        let overflow = prepared.len() - MAX_CTX_RULES;
        issues.push(CtxIssue {
            index: MAX_CTX_RULES,
            code: IssueCode::TooMany,
            message: format!("ctx_rules: at most {MAX_CTX_RULES} rules ({overflow} extra)"),
        });
    }
    (prepared, issues)
}

// ---- Document layer ------------------------------------------------------

fn decode(tables: &[Entry]) -> (Vec<CtxRule>, Vec<CtxIssue>) {
    let mut parse_errors = Vec::new();
    let raw: Vec<CtxRule> = tables
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let Some(table) = &entry.table else {
                parse_errors.push((index, "entry must be a table".into()));
                return CtxRule::default();
            };
            let mut document = DocumentMut::new();
            *document.as_table_mut() = table.clone();
            match toml_edit::de::from_document(document) {
                Ok(rule) => rule,
                Err(error) => {
                    parse_errors.push((index, error.to_string()));
                    CtxRule {
                        id: table.get("id").and_then(Item::as_str).unwrap_or("").into(),
                        title: table
                            .get("title")
                            .and_then(Item::as_str)
                            .unwrap_or("")
                            .into(),
                        ..Default::default()
                    }
                }
            }
        })
        .collect();
    let (mut rules, mut issues) = prepare_rules(raw);
    // A malformed entry decodes to a disabled placeholder: editing other
    // rules must not silently drop it, and its broken state must show up.
    for (index, error) in parse_errors {
        if let Some(rule) = rules.get_mut(index) {
            rule.enabled = false;
        }
        issues.retain(|issue| issue.index != index);
        issues.push(CtxIssue {
            index,
            code: IssueCode::Parse,
            message: format!("ctx_rules[{index}]: {error}"),
        });
    }
    for issue in &issues {
        if issue.code != IssueCode::TooMany
            && let Some(rule) = rules.get_mut(issue.index)
        {
            rule.enabled = false;
        }
    }
    issues.sort_by_key(|issue| issue.index);
    (rules, issues)
}

/// Reads the `[[ctx_rules]]` array with per-entry fault isolation.
pub fn read_rules(document: &DocumentMut) -> Result<(Vec<CtxRule>, Vec<CtxIssue>), String> {
    Ok(decode(&toml_rules::tables(document, "ctx_rules")?))
}

fn encoded(rule: &CtxRule) -> Result<Table, String> {
    toml_edit::ser::to_document(rule)
        .map(DocumentMut::into_table)
        .map_err(|error| format!("rule serialization failed: {error}"))
}

/// Refuses stale edits and patches only user-changed fields, mirroring
/// `rule_document::update` for the context-menu rule list.
pub fn update_rules(
    document: &mut DocumentMut,
    base: &[CtxRule],
    proposed: &[CtxRule],
) -> Result<(), String> {
    let existing = toml_rules::tables(document, "ctx_rules")?;
    let (current, existing_issues) = decode(&existing);
    if current != base {
        return Err("ctx_rules_changed_external".into());
    }
    let (_, issues) = prepare_rules(proposed.to_vec());
    for issue in issues {
        if issue.code == IssueCode::TooMany {
            // Existing oversized documents remain lossless and can be
            // reduced or edited, but cannot grow further.
            if proposed.len() > current.len() {
                return Err(issue.message);
            }
            continue;
        }
        if !current.iter().any(|old| old == &proposed[issue.index]) {
            return Err(issue.message);
        }
    }
    if current == proposed {
        return Ok(());
    }
    let mut output = Output::new(document, "ctx_rules");
    for rule in proposed {
        if let Some(index) = current.iter().position(|old| old.id == rule.id) {
            if current[index] == *rule {
                output.unchanged(&existing[index]);
                continue;
            }
            let mut table = existing[index].table.clone().unwrap_or_default();
            let previous = encoded(&current[index])?;
            let next = encoded(rule)?;
            toml_rules::patch(
                &mut table,
                &previous,
                &next,
                existing_issues.iter().any(|issue| issue.index == index),
            );
            output.push(table, true);
        } else {
            output.push(encoded(rule)?, false);
        }
    }
    document["ctx_rules"] = output.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_paths_are_not_reparsed_and_quoted_roots_are_escaped() {
        assert_eq!(
            substitute_args("\"%L\"", r"C:\temp\%1.txt"),
            "\"C:\\temp\\%1.txt\""
        );
        assert_eq!(substitute_args("\"%1\"", "C:\\"), "\"C:\\\\\"");
        assert_eq!(substitute_args("%V", "C:\\"), "C:\\");
    }

    #[test]
    fn invalid_patterns_fail_closed_and_ids_match_registry_case_rules() {
        assert!(!extension_matches(
            &["*.tar.gz".into()],
            std::path::Path::new("x.exe")
        ));
        assert!(extension_matches(
            &["*.my_ext".into()],
            std::path::Path::new("x.my_ext")
        ));
        let rule = CtxRule {
            id: "Tool".into(),
            title: "Tool".into(),
            command: "tool.exe".into(),
            ..Default::default()
        };
        let second = CtxRule {
            id: "tool".into(),
            patterns: vec!["*.tar.gz".into()],
            ..rule.clone()
        };
        let (rules, issues) = prepare_rules(vec![rule, second]);
        assert!(!rules[0].id.eq_ignore_ascii_case(&rules[1].id));
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == IssueCode::PatternInvalid)
        );
    }

    #[test]
    fn oversized_documents_are_not_truncated_by_unrelated_edits() {
        let rules: Vec<_> = (0..MAX_CTX_RULES + 2)
            .map(|i| CtxRule {
                id: format!("r{i}"),
                title: format!("Rule {i}"),
                command: "tool.exe".into(),
                ..Default::default()
            })
            .collect();
        let mut doc = DocumentMut::new();
        let mut tables = ArrayOfTables::new();
        for rule in &rules {
            tables.push(encoded(rule).unwrap());
        }
        doc["ctx_rules"] = Item::ArrayOfTables(tables);
        let (base, issues) = read_rules(&doc).unwrap();
        assert_eq!(base.len(), rules.len());
        assert!(issues.iter().any(|issue| issue.code == IssueCode::TooMany));
        let mut next = base.clone();
        next[0].title = "Edited".into();
        update_rules(&mut doc, &base, &next).unwrap();
        assert_eq!(read_rules(&doc).unwrap().0.len(), rules.len());
        let snapshot = doc.to_string();
        next.push(CtxRule {
            id: "extra".into(),
            ..rules[0].clone()
        });
        let current = read_rules(&doc).unwrap().0;
        assert!(update_rules(&mut doc, &current, &next).is_err());
        assert_eq!(doc.to_string(), snapshot);
    }

    #[test]
    fn aqs_translates_extension_lists() {
        assert_eq!(
            applies_to_aqs(&["*.exe".into(), ".dll".into()]),
            Some("System.FileExtension:=\".exe\" OR System.FileExtension:=\".dll\"".into())
        );
        assert_eq!(applies_to_aqs(&[]), None);
        assert_eq!(applies_to_aqs(&["*".into()]), None);
        assert_eq!(applies_to_aqs(&["C:\\tools".into()]), None);
    }

    #[test]
    fn extension_matching_is_case_insensitive_and_dotted() {
        let patterns = vec!["*.exe".to_string(), "ocx".to_string()];
        assert!(extension_matches(
            &patterns,
            std::path::Path::new("C:\\A B\\t.EXE")
        ));
        assert!(extension_matches(&patterns, std::path::Path::new("x.ocx")));
        assert!(!extension_matches(&patterns, std::path::Path::new("x.dll")));
        assert!(!extension_matches(&patterns, std::path::Path::new("noext")));
        assert!(extension_matches(&[], std::path::Path::new("anything")));
    }

    #[test]
    fn quote_policies_and_separators() {
        assert_eq!(
            format_paths(
                &["C:\\a b\\c.txt".into(), "C:\\d.txt".into()],
                QUOTE_AUTO,
                SEPARATOR_NEWLINE
            ),
            "\"C:\\a b\\c.txt\"\r\nC:\\d.txt"
        );
        assert_eq!(
            format_paths(&["C:\\d.txt".into()], QUOTE_ALWAYS, SEPARATOR_SPACE),
            "\"C:\\d.txt\""
        );
        assert_eq!(
            format_paths(&["C:\\a b.txt".into()], QUOTE_NONE, SEPARATOR_NEWLINE),
            "C:\\a b.txt"
        );
    }

    #[test]
    fn args_substitution_covers_all_placeholders() {
        assert_eq!(substitute_args("\"%1\"", "C:\\x y.exe"), "\"C:\\x y.exe\"");
        assert_eq!(substitute_args("%L --tag", "C:\\z"), "C:\\z --tag");
        assert_eq!(substitute_args("%V", "D:\\dir"), "D:\\dir");
    }

    #[test]
    fn prepare_slugifies_ids_and_repairs_scopes() {
        let (rules, issues) = prepare_rules(vec![CtxRule {
            id: "我的 工具!".into(),
            title: "T".into(),
            scope: "weird".into(),
            command: "cmd.exe".into(),
            ..Default::default()
        }]);
        assert!(issues.is_empty());
        // Non-ASCII ids slug to nothing, so a stable fallback is generated.
        assert_eq!(rules[0].id, "rule1");
        assert_eq!(rules[0].scope, SCOPE_FILE);
    }

    #[test]
    fn prepare_flags_missing_title_and_command() {
        let (_, issues) = prepare_rules(vec![CtxRule {
            id: "a".into(),
            ..Default::default()
        }]);
        assert!(issues.iter().any(|issue| issue.message.contains("title")));
        assert!(issues.iter().any(|issue| issue.message.contains("command")));
    }

    #[test]
    fn document_roundtrip_preserves_untouched_entries_and_comments() {
        let mut doc: DocumentMut = r#"custom = 7
[[ctx_rules]]
id = "rh"
title = "Resource Hacker" # keep note
enabled = true
scope = "file"
patterns = ["*.exe", "*.dll"]
command = 'D:\rh.exe'
args = '"%1"'

# broken entry must survive
[[ctx_rules]]
id = "bad"
title = 42
"#
        .parse()
        .unwrap();
        let (base, issues) = read_rules(&doc).unwrap();
        assert_eq!(base.len(), 2);
        assert_eq!(issues.len(), 1);
        assert!(base[0].enabled);
        assert!(!base[1].enabled);
        let mut next = base.clone();
        next[0].title = "RH v2".into();
        update_rules(&mut doc, &base, &next).unwrap();
        let text = doc.to_string();
        assert!(text.contains("title = \"RH v2\" # keep note"));
        assert!(text.contains("id = \"bad\""));
        assert!(text.contains("# broken entry must survive"));
        // Stale base refused.
        assert_eq!(
            update_rules(&mut doc, &base, &next).unwrap_err(),
            "ctx_rules_changed_external"
        );
        let (reloaded, _) = read_rules(&doc).unwrap();
        assert_eq!(reloaded[0].title, "RH v2");
    }

    #[test]
    fn full_field_rule_roundtrips_through_serialization() {
        let rule = CtxRule {
            id: "full".into(),
            title: "full rule".into(),
            enabled: false,
            scope: SCOPE_ALL.into(),
            patterns: vec!["*.exe".into()],
            command: "D:\\tool.exe".into(),
            args: "\"%1\" --flag".into(),
            icon: "D:\\tool.exe,0".into(),
            hidden_console: true,
            separator: true,
            working_dir: "D:\\".into(),
            selection_mode: SelectionMode::Together,
        };
        let mut doc = DocumentMut::new();
        update_rules(&mut doc, &[], std::slice::from_ref(&rule)).unwrap();
        let (expected, issues) = read_rules(&doc).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        let saved: DocumentMut = doc.to_string().parse().unwrap();
        assert_eq!(read_rules(&saved).unwrap().0, expected);
        assert_eq!(expected, vec![rule]);
    }

    #[test]
    fn batch_arguments_quote_roots_and_do_not_expand_tokens_in_paths() {
        let paths = vec!["C:\\".into(), "C:\\中文 空格\\%1 %*.txt".into()];
        let expected = "\"C:\\\\\" \"C:\\中文 空格\\%1 %*.txt\"";
        assert_eq!(selection_args("%*", &paths).as_deref(), Some(expected));
        assert_eq!(selection_args("\"%*\"", &paths).as_deref(), Some(expected));
        assert_eq!(
            selection_args("--open %* --flag", &paths),
            Some(format!("--open {expected} --flag"))
        );
        for invalid in [
            "--files=%*",
            "prefix%*",
            "%*suffix",
            "\"prefix %* suffix\"",
            "\"%*\"tail",
        ] {
            assert_eq!(selection_args(invalid, &paths), None, "{invalid}");
        }
        assert!(selection_args("%*", &[]).is_none());
        assert_eq!(
            selection_args("\"%1\"", &paths),
            Some(substitute_args("\"%1\"", &paths[0]))
        );
    }

    #[test]
    fn selection_mode_defaults_preserve_documents_and_batch_requires_all_paths() {
        let text = "[[ctx_rules]]\nid = 'old'\ntitle = 'Tool'\ncommand = 'tool.exe'\nargs = '\"%1\"' # keep\n";
        let mut doc = text.parse::<DocumentMut>().unwrap();
        let (base, issues) = read_rules(&doc).unwrap();
        assert!(issues.is_empty());
        assert_eq!(base[0].selection_mode, SelectionMode::Each);
        update_rules(&mut doc, &base, &base).unwrap();
        assert_eq!(doc.to_string(), text);
        let mut next = base.clone();
        next[0].selection_mode = SelectionMode::Together;
        assert!(rule_selection_args(&next[0], &["one.txt".into(), "two.txt".into()]).is_none());
        assert!(
            prepare_rules(next.clone())
                .1
                .iter()
                .any(|issue| issue.code == IssueCode::ArgumentsInvalid)
        );
        next[0].args = "%*".into();
        assert_eq!(
            rule_selection_args(&next[0], &["one.txt".into(), "two.txt".into()]),
            Some("\"one.txt\" \"two.txt\"".into())
        );
        update_rules(&mut doc, &base, &next).unwrap();
        assert!(doc.to_string().contains("# keep"));
        assert_eq!(read_rules(&doc).unwrap().0, next);
        let invalid = doc.to_string().replace("together", "unknown");
        assert!(
            read_rules(&invalid.parse().unwrap())
                .unwrap()
                .1
                .iter()
                .any(|issue| issue.code == IssueCode::Parse)
        );
    }
}
