//! Shared, model-independent preservation of TOML rule tables.
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

pub(crate) struct Entry {
    pub table: Option<Table>,
    pub inline: Option<Value>,
}

pub(crate) fn tables(document: &DocumentMut, key: &str) -> Result<Vec<Entry>, String> {
    match document.get(key) {
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
            .ok_or_else(|| format!("{key} must be an array"))?
            .iter()
            .map(|value| Entry {
                table: value
                    .as_inline_table()
                    .cloned()
                    .map(|table| table.into_table()),
                inline: Some(value.clone()),
            })
            .collect()),
    }
}

pub(crate) fn patch(table: &mut Table, previous: &Table, next: &Table, repair: bool) {
    // Retain the old sorted update order without allocating strings/tree nodes.
    let mut keys: Vec<_> = previous
        .iter()
        .chain(next.iter())
        .map(|(key, _)| key)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        if !repair
            && previous.get(key).map(ToString::to_string) == next.get(key).map(ToString::to_string)
        {
            continue;
        }
        if let Some(mut value) = next.get(key).cloned() {
            if let (Some(old), Some(new)) = (
                table.get(key).and_then(Item::as_value),
                value.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            table.insert(key, value);
        } else {
            table.remove(key);
        }
    }
}

pub(crate) struct Output {
    tables: ArrayOfTables,
    inline: Option<Array>,
}

impl Output {
    pub fn new(document: &DocumentMut, key: &str) -> Self {
        let mut inline = document.get(key).and_then(Item::as_array).cloned();
        if let Some(array) = &mut inline {
            array.clear();
        }
        Self {
            tables: ArrayOfTables::new(),
            inline,
        }
    }

    pub fn unchanged(&mut self, entry: &Entry) {
        if let (Some(array), Some(value)) = (&mut self.inline, &entry.inline) {
            array.push_formatted(value.clone());
        } else {
            self.tables.push(entry.table.clone().unwrap_or_default());
        }
    }

    pub fn push(&mut self, table: Table, existing: bool) {
        if let Some(array) = &mut self.inline {
            let value = Value::InlineTable(table.into_inline_table());
            if existing {
                array.push_formatted(value);
            } else {
                array.push(value);
            }
        } else {
            self.tables.push(table);
        }
    }

    pub fn finish(self) -> Item {
        if let Some(array) = self.inline {
            toml_edit::value(array)
        } else if self.tables.is_empty() {
            toml_edit::value(Array::new())
        } else {
            Item::ArrayOfTables(self.tables)
        }
    }
}
