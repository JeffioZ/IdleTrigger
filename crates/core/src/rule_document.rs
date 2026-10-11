//! Rule editing that isolates malformed entries and retains untouched TOML.
use crate::automation::{self as auto, Rule, RuleIssue};
use crate::toml_rules::{self, Entry, Output};
use toml_edit::{DocumentMut, Item, Table};

fn decode(tables: &[Entry]) -> (Vec<Rule>, Vec<RuleIssue>) {
    let mut parse_errors = Vec::new();
    let raw: Vec<Rule> = tables
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let Some(table) = &entry.table else {
                parse_errors.push((index, "entry must be a table".into()));
                return Rule::default();
            };
            let mut document = DocumentMut::new();
            *document.as_table_mut() = table.clone();
            match toml_edit::de::from_document(document) {
                Ok(rule) => rule,
                Err(error) => {
                    parse_errors.push((index, error.to_string()));
                    Rule {
                        id: table.get("id").and_then(Item::as_str).unwrap_or("").into(),
                        name: table
                            .get("name")
                            .and_then(Item::as_str)
                            .unwrap_or("")
                            .into(),
                        ..Default::default()
                    }
                }
            }
        })
        .collect();
    let (normalized, mut issues) = auto::prepare_rules(&raw);
    for (index, error) in parse_errors {
        issues.retain(|i| i.index != index);
        issues.push(RuleIssue {
            index,
            rule_id: normalized[index].id.clone(),
            message: format!("automation_rules[{index}]: {error}"),
        });
    }
    issues.sort_by_key(|i| i.index);
    (auto::runtime_rules(normalized, &issues), issues)
}

pub fn read(document: &DocumentMut) -> Result<(Vec<Rule>, Vec<RuleIssue>), String> {
    Ok(decode(&toml_rules::tables(document, "automation_rules")?))
}

fn encoded(rule: &Rule) -> Result<Table, String> {
    toml_edit::ser::to_document(rule)
        .map(DocumentMut::into_table)
        .map_err(|error| format!("rule serialization failed: {error}"))
}

/// Refuse stale edits, retain invalid untouched entries, and patch only the
/// fields changed by the user. Unrecognized keys belong to the user.
///
/// `proposed` must already be normalized (pass it through `prepare_rules`
/// first, as the editor does): `base` is compared against the normalized
/// decode of the document, so raw proposals would read as a permanent
/// external change.
pub fn update(document: &mut DocumentMut, base: &[Rule], proposed: &[Rule]) -> Result<(), String> {
    let existing = toml_rules::tables(document, "automation_rules")?;
    let (current, existing_issues) = decode(&existing);
    if current != base {
        return Err("automation_changed_external".into());
    }
    let (_, issues) = auto::prepare_rules(proposed);
    for issue in issues {
        if !current.iter().any(|old| old == &proposed[issue.index]) {
            return Err(issue.message);
        }
    }
    if current == proposed {
        return Ok(());
    }
    let mut output = Output::new(document, "automation_rules");
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
    document["automation_rules"] = output.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_entry_does_not_disable_valid_inline_rules_or_disappear_on_save() {
        let mut doc: DocumentMut = r#"automation_rules = [
  { id = "valid", name = "test", enabled = true, action = "lock", trigger = "daily", time = "08:00", warning_seconds = 60 },
  "bad row", # retain this entry
]
"#.parse().unwrap();
        let (base, issues) = read(&doc).unwrap();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(base[0].enabled);
        assert!(!base[1].enabled);
        let mut next = base.clone();
        next[0].name = "edited".into();
        update(&mut doc, &base, &next).unwrap();
        let text = doc.to_string();
        assert!(text.contains("\"bad row\", # retain this entry"));
        let parsed: DocumentMut = text.parse().unwrap();
        assert_eq!(read(&parsed).unwrap().0, next);
    }
    #[test]
    fn malformed_rule_is_isolated_and_unedited_text_survives_save() {
        let mut doc: DocumentMut = r#"custom = 42
[[automation_rules]]
id = "ok"
name = "original" # name note
enabled = true
action = "lock"
trigger = "daily"
time = "08:00"
warning_seconds = 60
custom_rule_key = "keep me"

# Broken entry must survive editing the other rule.
[[automation_rules]]
id = "bad"
name = "bad"
enabled = "not a boolean"
unknown = 123 # keep this
"#
        .parse()
        .unwrap();
        let (base, issues) = read(&doc).unwrap();
        assert_eq!(base.len(), 2);
        assert_eq!(issues.len(), 1);
        assert!(base[0].enabled);
        assert!(!base[1].enabled);
        let original_bad = doc["automation_rules"]
            .as_array_of_tables()
            .unwrap()
            .get(1)
            .unwrap()
            .to_string();
        let mut edits = base.clone();
        edits[0].name = "changed".into();
        update(&mut doc, &base, &edits).unwrap();
        let text = doc.to_string();
        assert!(text.contains("name = \"changed\" # name note"));
        assert!(text.contains("custom_rule_key = \"keep me\""));
        assert_eq!(
            doc["automation_rules"]
                .as_array_of_tables()
                .unwrap()
                .get(1)
                .unwrap()
                .to_string(),
            original_bad
        );
        assert_eq!(doc["custom"].as_integer(), Some(42));
        let saved = doc.to_string();
        assert_eq!(
            update(&mut doc, &base, &edits).unwrap_err(),
            "automation_changed_external"
        );
        assert_eq!(doc.to_string(), saved);
        let (new_base, _) = read(&doc).unwrap();
        let mut repaired = new_base.clone();
        repaired[1] = Rule {
            id: "bad".into(),
            name: "repaired".into(),
            ..repaired[0].clone()
        };
        update(&mut doc, &new_base, &repaired).unwrap();
        let (new_base, issues) = read(&doc).unwrap();
        assert!(issues.is_empty());
        assert!(new_base[1].enabled);
        assert!(doc.to_string().contains("unknown = 123 # keep this"));
        update(&mut doc, &new_base, &[]).unwrap();
        assert!(read(&doc).unwrap().0.is_empty());
    }

    #[test]
    fn full_field_rule_roundtrips_through_serialization() {
        // Every field set non-default: adding a Rule field must extend this
        // rule (compilation fails otherwise) so the serde roundtrip stays
        // proven for the whole model.
        let rule = Rule {
            id: "full".into(),
            name: "full rule".into(),
            enabled: false,
            action: "shutdown".into(),
            trigger: "weekly".into(),
            time: "09:30".into(),
            end_time: "17:45".into(),
            date: "2026-01-02".into(),
            days: vec!["wed".into(), "mon".into()],
            process_logic: "any".into(),
            processes: vec![
                auto::ProcessTarget {
                    kind: "name".into(),
                    executable: "app.exe".into(),
                    path: String::new(),
                },
                auto::ProcessTarget {
                    kind: "path".into(),
                    executable: "tool.exe".into(),
                    path: "C:\\tools\\tool.exe".into(),
                },
            ],
            keep_screen_on: true,
            idle_minutes: 5,
            warning_seconds: 120,
            blocked_policy: "skip".into(),
            max_wait_minutes: 10,
            battery_level: 20,
        };
        let mut doc = DocumentMut::new();
        update(&mut doc, &[], std::slice::from_ref(&rule)).unwrap();
        let (expected, issues) = auto::prepare_rules(std::slice::from_ref(&rule));
        assert!(issues.is_empty(), "{issues:?}");
        let expected = auto::runtime_rules(expected, &issues);
        assert_eq!(read(&doc).unwrap().0, expected);
        let saved: DocumentMut = doc.to_string().parse().unwrap();
        assert_eq!(read(&saved).unwrap().0, expected);
    }
}
