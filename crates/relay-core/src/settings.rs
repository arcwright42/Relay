//! Application preferences are independent of projects and agent sessions.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    SimplifiedChinese,
    English,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "zh-CN" => Some(Self::SimplifiedChinese),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "简体中文",
            Self::English => "English",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsSnapshot {
    pub language: Language,
    pub saving: bool,
    pub error: Option<String>,
}

/// Changes apply in memory immediately; implementations persist off the UI thread.
pub trait SettingsService: Send + Sync {
    fn snapshot(&self) -> SettingsSnapshot;
    fn set_language(&self, language: Language);
}
