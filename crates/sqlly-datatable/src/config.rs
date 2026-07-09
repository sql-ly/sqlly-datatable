//! Grid-wide configuration: per-kind formatting rules, per-column overrides,
//! and key bindings.
//!
//! [`GridConfig`] is cheap to clone. [`GridConfig::resolve`] and
//! [`GridConfig::resolve_all`] turn a column index into a fully-merged
//! [`ResolvedColumnFormat`]; the grid caches the resolved list on its state
//! so this work does not repeat on every paint.

use crate::data::ColumnKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextCase {
    Upper,
    Lower,
    Title,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TruncationBehavior {
    Ellipsis,
    CutOff,
    Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelativeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ReplacementTiming {
    BeforeFormat,
    #[default]
    AfterFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    pub decimals: usize,
    pub show_negative_red: bool,
    pub negative_parentheses: bool,
    pub thousands_separator: bool,
    pub alignment: TextAlignment,
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self {
            decimals: 2,
            show_negative_red: true,
            negative_parentheses: false,
            thousands_separator: true,
            alignment: TextAlignment::Right,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativeDateFormat {
    pub units: Vec<RelativeUnit>,
    pub max_components: usize,
}

impl Default for RelativeDateFormat {
    fn default() -> Self {
        Self {
            units: vec![RelativeUnit::Year, RelativeUnit::Month, RelativeUnit::Day],
            max_components: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateFormat {
    pub format: String,
    pub timezone_offset_minutes: i32,
    pub relative: Option<RelativeDateFormat>,
    pub alignment: TextAlignment,
}

impl Default for DateFormat {
    fn default() -> Self {
        Self {
            format: "%Y-%m-%d".into(),
            timezone_offset_minutes: 0,
            relative: None,
            alignment: TextAlignment::Center,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BooleanFormat {
    pub true_text: String,
    pub false_text: String,
    pub alignment: TextAlignment,
}

impl Default for BooleanFormat {
    fn default() -> Self {
        Self {
            true_text: "true".into(),
            false_text: "false".into(),
            alignment: TextAlignment::Center,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringFormat {
    pub case: TextCase,
    pub max_length: Option<usize>,
    pub truncation: TruncationBehavior,
    pub alignment: TextAlignment,
}

impl Default for StringFormat {
    fn default() -> Self {
        Self {
            case: TextCase::None,
            max_length: None,
            truncation: TruncationBehavior::Ellipsis,
            alignment: TextAlignment::Left,
        }
    }
}

/// How a cell with no value ([`crate::data::CellValue::None`]) is displayed.
/// The grid-wide default is set via [`GridConfig::default_null`]; individual
/// columns can override it through [`ColumnOverride::null`]. Columns without
/// an override use the grid default, and callers that never set
/// `default_null` get the built-in default: italic `NULL` over a distinctive
/// background (`GridTheme::null_bg` / `null_fg`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NullFormat {
    /// Placeholder text shown in the cell.
    pub text: String,
    /// Render the placeholder in italics.
    pub italic: bool,
    /// Fill the cell with the theme's `null_bg` behind the placeholder.
    pub background: bool,
}

impl Default for NullFormat {
    fn default() -> Self {
        Self {
            text: "NULL".into(),
            italic: true,
            background: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacementRule {
    pub find: String,
    pub replace: String,
}

impl ReplacementRule {
    /// Convenience constructor.
    #[must_use]
    pub fn new(find: impl Into<String>, replace: impl Into<String>) -> Self {
        Self {
            find: find.into(),
            replace: replace.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnOverride {
    pub number: Option<NumberFormat>,
    pub date: Option<DateFormat>,
    pub boolean: Option<BooleanFormat>,
    pub string: Option<StringFormat>,
    pub null: Option<NullFormat>,
    pub replacements: Option<Vec<ReplacementRule>>,
    pub replacement_timing: Option<ReplacementTiming>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedColumnFormat {
    pub kind: ColumnKind,
    pub number: NumberFormat,
    pub date: DateFormat,
    pub boolean: BooleanFormat,
    pub string: StringFormat,
    pub null: NullFormat,
    pub replacements: Vec<ReplacementRule>,
    pub replacement_timing: ReplacementTiming,
}

impl ResolvedColumnFormat {
    #[must_use]
    pub fn alignment(&self) -> TextAlignment {
        match self.kind {
            ColumnKind::Integer | ColumnKind::Decimal => self.number.alignment,
            ColumnKind::Date => self.date.alignment,
            ColumnKind::Boolean => self.boolean.alignment,
            ColumnKind::Text => self.string.alignment,
            ColumnKind::None => TextAlignment::Left,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: String,
    pub platform: bool,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

impl KeyBinding {
    /// `true` iff `ks` matches this binding and carries no extra modifiers we
    /// did not declare.
    ///
    /// For example, `Cmd+C` matches `copy`; `Cmd+Alt+C` does not unless `alt`
    /// is `true` on the binding. This avoids the previous footgun where
    /// `Cmd+Alt+C` would still satisfy a binding that only declared `Cmd+C`.
    pub fn matches(&self, ks: &gpui::Keystroke) -> bool {
        let required = self.platform || self.shift || self.alt || self.control;
        let actual =
            ks.modifiers.platform || ks.modifiers.shift || ks.modifiers.alt || ks.modifiers.control;
        // If the binding requires nothing at all, only match when the user
        // pressed exactly that key with no modifiers.
        if !required {
            return self.key == ks.key && !actual;
        }
        self.key == ks.key
            && self.platform == ks.modifiers.platform
            && self.shift == ks.modifiers.shift
            && self.alt == ks.modifiers.alt
            && self.control == ks.modifiers.control
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBindings {
    pub select_all: KeyBinding,
    pub copy: KeyBinding,
    pub copy_with_headers: KeyBinding,
    pub page_up: KeyBinding,
    pub page_down: KeyBinding,
    pub context_menu_modifier_control: bool,
    pub context_menu_modifier_alt: bool,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            select_all: KeyBinding {
                key: "a".into(),
                platform: true,
                shift: false,
                alt: false,
                control: false,
            },
            copy: KeyBinding {
                key: "c".into(),
                platform: true,
                shift: false,
                alt: false,
                control: false,
            },
            copy_with_headers: KeyBinding {
                key: "c".into(),
                platform: true,
                shift: true,
                alt: false,
                control: false,
            },
            page_up: KeyBinding {
                key: "pageup".into(),
                platform: false,
                shift: false,
                alt: false,
                control: false,
            },
            page_down: KeyBinding {
                key: "pagedown".into(),
                platform: false,
                shift: false,
                alt: false,
                control: false,
            },
            context_menu_modifier_control: true,
            context_menu_modifier_alt: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridConfig {
    pub key_bindings: KeyBindings,
    pub default_number: NumberFormat,
    pub default_date: DateFormat,
    pub default_boolean: BooleanFormat,
    pub default_string: StringFormat,
    /// Grid-wide display for cells with no value; per-column override via
    /// [`ColumnOverride::null`].
    pub default_null: NullFormat,
    pub default_replacements: Vec<ReplacementRule>,
    pub replacement_timing: ReplacementTiming,
    pub column_overrides: Vec<ColumnOverride>,
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            key_bindings: KeyBindings::default(),
            default_number: NumberFormat::default(),
            default_date: DateFormat::default(),
            default_boolean: BooleanFormat::default(),
            default_string: StringFormat::default(),
            default_null: NullFormat::default(),
            default_replacements: vec![],
            replacement_timing: ReplacementTiming::AfterFormat,
            column_overrides: vec![],
        }
    }
}

impl GridConfig {
    /// Resolve the format for a single column. Returns a freshly-merged
    /// [`ResolvedColumnFormat`] every call; the grid state caches the result.
    #[must_use]
    pub fn resolve(&self, col_idx: usize, kind: ColumnKind) -> ResolvedColumnFormat {
        let o = self.column_overrides.get(col_idx);
        ResolvedColumnFormat {
            kind,
            number: o.and_then(|o| o.number).unwrap_or(self.default_number),
            date: o
                .and_then(|o| o.date.clone())
                .unwrap_or_else(|| self.default_date.clone()),
            boolean: o
                .and_then(|o| o.boolean.clone())
                .unwrap_or_else(|| self.default_boolean.clone()),
            string: o
                .and_then(|o| o.string.clone())
                .unwrap_or_else(|| self.default_string.clone()),
            null: o
                .and_then(|o| o.null.clone())
                .unwrap_or_else(|| self.default_null.clone()),
            replacements: o
                .and_then(|o| o.replacements.clone())
                .unwrap_or_else(|| self.default_replacements.clone()),
            replacement_timing: o
                .and_then(|o| o.replacement_timing)
                .unwrap_or(self.replacement_timing),
        }
    }

    /// Resolve formats for every column. Used during state initialization and
    /// when the config changes.
    #[must_use]
    pub fn resolve_all(&self, columns: &[crate::data::Column]) -> Vec<ResolvedColumnFormat> {
        columns
            .iter()
            .enumerate()
            .map(|(i, c)| self.resolve(i, c.kind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    fn ks(key: &str, platform: bool, shift: bool, alt: bool, control: bool) -> Keystroke {
        Keystroke {
            key: key.into(),
            modifiers: gpui::Modifiers {
                platform,
                shift,
                alt,
                control,
                function: false,
            },
            ..Default::default()
        }
    }

    #[test]
    fn resolve_uses_defaults_without_override() {
        let cfg = GridConfig::default();
        let cols = vec![
            crate::data::Column::new("a", ColumnKind::Text, 80.0),
            crate::data::Column::new("b", ColumnKind::Integer, 80.0),
        ];
        let resolved = cfg.resolve_all(&cols);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].kind, ColumnKind::Text);
        assert_eq!(resolved[1].kind, ColumnKind::Integer);
        assert_eq!(resolved[0].number.alignment, TextAlignment::Right);
        assert_eq!(resolved[0].string.alignment, TextAlignment::Left);
    }

    #[test]
    fn resolve_uses_per_column_override() {
        let cfg = GridConfig {
            column_overrides: vec![
                ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 4,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                },
                ColumnOverride::default(),
            ],
            ..GridConfig::default()
        };
        let cols = vec![
            crate::data::Column::new("a", ColumnKind::Decimal, 80.0),
            crate::data::Column::new("b", ColumnKind::Decimal, 80.0),
        ];
        let resolved = cfg.resolve_all(&cols);
        assert_eq!(resolved[0].number.decimals, 4);
        assert_eq!(resolved[1].number.decimals, 2);
    }

    #[test]
    fn key_binding_matches_exact_modifier_set() {
        let binding = KeyBinding {
            key: "c".into(),
            platform: true,
            shift: false,
            alt: false,
            control: false,
        };
        assert!(binding.matches(&ks("c", true, false, false, false)));
        // Adding extra modifiers (Alt) should NOT match a binding that didn't request it.
        assert!(!binding.matches(&ks("c", true, false, true, false)));
        assert!(!binding.matches(&ks("c", true, false, false, true)));
        // Wrong key never matches.
        assert!(!binding.matches(&ks("x", true, false, false, false)));
    }

    #[test]
    fn key_binding_with_no_required_modifier_only_matches_bare_key() {
        let binding = KeyBinding {
            key: "pagedown".into(),
            platform: false,
            shift: false,
            alt: false,
            control: false,
        };
        assert!(binding.matches(&ks("pagedown", false, false, false, false)));
        assert!(!binding.matches(&ks("pagedown", true, false, false, false)));
    }

    #[test]
    fn key_binding_with_alt_true_accepts_alt_modifier() {
        let binding = KeyBinding {
            key: "c".into(),
            platform: true,
            shift: false,
            alt: true,
            control: false,
        };
        assert!(binding.matches(&ks("c", true, false, true, false)));
        assert!(!binding.matches(&ks("c", true, false, false, false)));
    }
}
