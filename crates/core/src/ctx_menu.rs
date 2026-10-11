//! Explorer context-menu rule model, validation, and pure helpers.
//!
//! Menu entries themselves live in the registry (static verbs owned by the
//! tray app); this module owns the `[[ctx_rules]]` document section and the
//! pure translations the registry sync consumes: pattern lists to
//! `AppliesTo` filters, scopes to verb roots, and copy-path formatting.

use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

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
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    Some(stripped.to_ascii_lowercase())
}

/// Translates pattern tokens into an `AppliesTo` AQS expression. `None`
/// means "show the verb unfiltered" (empty list, a `*` token, or a token the
/// filter syntax cannot express; invoke-time matching still applies).
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
        None => true,
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
    args.replace("%L", path)
        .replace("%V", path)
        .replace("%1", path)
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
        while used_ids.contains(&id) {
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
        prepared.truncate(MAX_CTX_RULES);
        issues.push(CtxIssue {
            index: MAX_CTX_RULES,
            code: IssueCode::TooMany,
            message: format!("ctx_rules: at most {MAX_CTX_RULES} rules ({overflow} dropped)"),
        });
    }
    (prepared, issues)
}

// ---- Document layer ------------------------------------------------------

#[derive(Clone)]
struct Entry {
    table: Option<Table>,
    inline: Option<Value>,
}

fn tables(document: &DocumentMut) -> Result<Vec<Entry>, String> {
    match document.get("ctx_rules") {
        None => Ok(Vec::new()),
        Some(Item::ArrayOfTables(tables)) => Ok(tables
            .iter()
            .cloned()
            .map(|table| Entry {
                table: Some(table),
                inline: None,
            })
            .collect()),
        Some(item) => Ok(item
            .as_array()
            .ok_or("ctx_rules must be an array")?
            .iter()
            .map(|v| Entry {
                table: v.as_inline_table().cloned().map(|v| v.into_table()),
                inline: Some(v.clone()),
            })
            .collect()),
    }
}

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
            match toml_edit::de::from_str(&document.to_string()) {
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
    issues.sort_by_key(|issue| issue.index);
    (rules, issues)
}

/// Reads the `[[ctx_rules]]` array with per-entry fault isolation.
pub fn read_rules(document: &DocumentMut) -> Result<(Vec<CtxRule>, Vec<CtxIssue>), String> {
    Ok(decode(&tables(document)?))
}

fn encoded(rule: &CtxRule) -> Result<Table, String> {
    #[derive(Serialize)]
    struct RulesDocument<'a> {
        ctx_rules: &'a [CtxRule],
    }
    let doc = toml_edit::ser::to_document(&RulesDocument {
        ctx_rules: std::slice::from_ref(rule),
    })
    .map_err(|error| format!("rule serialization failed: {error}"))?;
    tables(&doc)
        .map_err(|error| format!("serialized rule lookup failed: {error}"))?
        .into_iter()
        .next()
        .ok_or("serialized rule entry is missing".to_string())?
        .table
        .ok_or("serialized rule entry is not a table".to_string())
}

/// Refuses stale edits and patches only user-changed fields, mirroring
/// `rule_document::update` for the context-menu rule list.
pub fn update_rules(
    document: &mut DocumentMut,
    base: &[CtxRule],
    proposed: &[CtxRule],
) -> Result<(), String> {
    let existing = tables(document)?;
    let (current, existing_issues) = decode(&existing);
    if current != base {
        return Err("ctx_rules_changed_external".into());
    }
    let (_, issues) = prepare_rules(proposed.to_vec());
    for issue in issues {
        if !current.iter().any(|old| old == &proposed[issue.index]) {
            return Err(issue.message);
        }
    }
    if current == proposed {
        return Ok(());
    }
    let mut output = ArrayOfTables::new();
    let mut inline = document.get("ctx_rules").and_then(Item::as_array).cloned();
    if let Some(array) = &mut inline {
        array.clear();
    }
    for rule in proposed {
        if let Some(index) = current.iter().position(|old| old.id == rule.id) {
            if current[index] == *rule {
                // Untouched (including malformed) entries stay verbatim.
                let table = existing[index].table.clone().unwrap_or_default();
                if let (Some(array), Some(value)) = (&mut inline, &existing[index].inline) {
                    array.push_formatted(value.clone());
                } else {
                    output.push(table);
                }
                continue;
            }
            let mut table = existing[index].table.clone().unwrap_or_default();
            let previous = encoded(&current[index])?;
            let next = encoded(rule)?;
            let keys: std::collections::BTreeSet<_> = previous
                .iter()
                .chain(next.iter())
                .map(|(key, _)| key.to_owned())
                .collect();
            for key in keys {
                if !existing_issues.iter().any(|issue| issue.index == index)
                    && previous.get(&key).map(ToString::to_string)
                        == next.get(&key).map(ToString::to_string)
                {
                    continue;
                }
                if let Some(mut value) = next.get(&key).cloned() {
                    if let (Some(old), Some(new)) = (
                        table.get(&key).and_then(Item::as_value),
                        value.as_value_mut(),
                    ) {
                        *new.decor_mut() = old.decor().clone();
                    }
                    table.insert(&key, value);
                } else {
                    table.remove(&key);
                }
            }
            if let Some(array) = &mut inline {
                array.push_formatted(Value::InlineTable(table.into_inline_table()));
            } else {
                output.push(table);
            }
        } else {
            let table = encoded(rule)?;
            if let Some(array) = &mut inline {
                array.push(Value::InlineTable(table.into_inline_table()));
            } else {
                output.push(table);
            }
        }
    }
    document["ctx_rules"] = if let Some(array) = inline {
        toml_edit::value(array)
    } else if output.is_empty() {
        toml_edit::value(toml_edit::Array::new())
    } else {
        Item::ArrayOfTables(output)
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let mut doc = DocumentMut::new();
        update_rules(&mut doc, &[], std::slice::from_ref(&rule)).unwrap();
        let (expected, issues) = read_rules(&doc).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(expected, vec![rule]);
    }
}
