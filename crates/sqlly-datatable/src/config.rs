use crate::data::ColumnKind;

#[derive(Clone, Debug, PartialEq)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextCase {
    Upper,
    Lower,
    Title,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TruncationBehavior {
    Ellipsis,
    CutOff,
    Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RelativeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplacementTiming {
    BeforeFormat,
    AfterFormat,
}

impl Default for ReplacementTiming {
    fn default() -> Self {
        ReplacementTiming::AfterFormat
    }
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct ReplacementRule {
    pub find: String,
    pub replace: String,
}

#[derive(Clone, Debug, Default)]
pub struct ColumnOverride {
    pub number: Option<NumberFormat>,
    pub date: Option<DateFormat>,
    pub boolean: Option<BooleanFormat>,
    pub string: Option<StringFormat>,
    pub replacements: Option<Vec<ReplacementRule>>,
    pub replacement_timing: Option<ReplacementTiming>,
}

#[derive(Clone, Debug)]
pub struct ResolvedColumnFormat {
    pub kind: ColumnKind,
    pub number: NumberFormat,
    pub date: DateFormat,
    pub boolean: BooleanFormat,
    pub string: StringFormat,
    pub replacements: Vec<ReplacementRule>,
    pub replacement_timing: ReplacementTiming,
}

impl ResolvedColumnFormat {
    pub fn alignment(&self) -> TextAlignment {
        match self.kind {
            ColumnKind::Integer | ColumnKind::Decimal => self.number.alignment.clone(),
            ColumnKind::Date => self.date.alignment.clone(),
            ColumnKind::Boolean => self.boolean.alignment.clone(),
            ColumnKind::Text => self.string.alignment.clone(),
            ColumnKind::None => TextAlignment::Left,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeyBinding {
    pub key: String,
    pub platform: bool,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

impl KeyBinding {
    pub fn matches(&self, ks: &gpui::Keystroke) -> bool {
        self.key == ks.key
            && (!self.platform || ks.modifiers.platform)
            && (!self.shift || ks.modifiers.shift)
            && (!self.alt || ks.modifiers.alt)
            && (!self.control || ks.modifiers.control)
    }
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct GridConfig {
    pub key_bindings: KeyBindings,
    pub default_number: NumberFormat,
    pub default_date: DateFormat,
    pub default_boolean: BooleanFormat,
    pub default_string: StringFormat,
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
            default_replacements: vec![],
            replacement_timing: ReplacementTiming::AfterFormat,
            column_overrides: vec![],
        }
    }
}

impl GridConfig {
    pub fn resolve(&self, col_idx: usize, kind: ColumnKind) -> ResolvedColumnFormat {
        let o = self.column_overrides.get(col_idx);
        ResolvedColumnFormat {
            kind,
            number: o.and_then(|o| o.number.clone()).unwrap_or_else(|| self.default_number.clone()),
            date: o.and_then(|o| o.date.clone()).unwrap_or_else(|| self.default_date.clone()),
            boolean: o.and_then(|o| o.boolean.clone()).unwrap_or_else(|| self.default_boolean.clone()),
            string: o.and_then(|o| o.string.clone()).unwrap_or_else(|| self.default_string.clone()),
            replacements: o.and_then(|o| o.replacements.clone()).unwrap_or_else(|| self.default_replacements.clone()),
            replacement_timing: o.and_then(|o| o.replacement_timing.clone()).unwrap_or_else(|| self.replacement_timing.clone()),
        }
    }

    pub fn resolve_all(&self, columns: &[crate::data::Column]) -> Vec<ResolvedColumnFormat> {
        columns.iter().enumerate().map(|(i, c)| self.resolve(i, c.kind.clone())).collect()
    }
}
