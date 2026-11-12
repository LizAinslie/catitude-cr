use std::fmt;

/// One key/value field produced by a [`crate::CardApp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub value: String,
}

impl Field {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// One line of a (possibly redacted) structural dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpLine {
    pub label: String,
    pub value: String,
}

impl DumpLine {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// Structured result of a single [`crate::CardApp`] read.
///
/// Fields are the high-level facts the app wants to surface. `raw` is a
/// structural dump (ATR bytes, file IDs, redacted TLV). Built-in apps never
/// put a full payment-card PAN, a cardholder name, or a CVV in either place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardInfo {
    /// Short name of the app that produced this result (`"emv"`, `"desfire"`, …).
    pub app: String,
    pub fields: Vec<Field>,
    pub raw: Vec<DumpLine>,
}

impl CardInfo {
    pub fn new(app: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            fields: Vec::new(),
            raw: Vec::new(),
        }
    }

    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.push(Field::new(key, value));
    }

    pub fn push_raw(&mut self, label: impl Into<String>, value: impl Into<String>) {
        self.raw.push(DumpLine::new(label, value));
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
    }
}

impl fmt::Display for CardInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[{}]", self.app)?;
        for field in &self.fields {
            writeln!(f, "  {:<16} {}", field.key, field.value)?;
        }
        if !self.raw.is_empty() {
            writeln!(f, "  --- dump ---")?;
            for line in &self.raw {
                writeln!(f, "  {:<16} {}", line.label, line.value)?;
            }
        }
        Ok(())
    }
}

/// Rows suitable for a simple list view (GPUI, TUI, or anything else).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRow {
    pub label: String,
    pub value: String,
    pub kind: RowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Heading,
    Field,
    Dump,
}

/// Flatten a [`CardInfo`] into labelled rows. Drop this into a GPUI list
/// (`for row in rows { label(&row.label); label(&row.value); }`) or print it.
pub fn view_rows(infos: &[CardInfo]) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    for info in infos {
        rows.push(ViewRow {
            label: info.app.clone(),
            value: String::new(),
            kind: RowKind::Heading,
        });
        for field in &info.fields {
            rows.push(ViewRow {
                label: field.key.clone(),
                value: field.value.clone(),
                kind: RowKind::Field,
            });
        }
        for line in &info.raw {
            rows.push(ViewRow {
                label: line.label.clone(),
                value: line.value.clone(),
                kind: RowKind::Dump,
            });
        }
    }
    rows
}
