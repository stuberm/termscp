//! Validated comparison settings shared by profiles, preferences and workers.
use serde::{Deserialize, Serialize};

/// Number of simultaneous file-pair comparisons (one connection per side/pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize", into = "usize")]
pub struct Parallelism(usize);

impl Parallelism {
    pub const MIN: usize = 1;
    pub const MAX: usize = 16;

    pub fn get(self) -> usize {
        self.0
    }

    pub fn adjusted(self, forward: bool) -> Self {
        Self(if forward {
            (self.0 + 1).min(Self::MAX)
        } else {
            self.0.saturating_sub(1).max(Self::MIN)
        })
    }
}

impl Default for Parallelism {
    fn default() -> Self {
        Self(4)
    }
}

impl TryFrom<usize> for Parallelism {
    type Error = String;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(format!(
                "Parallel comparisons must be between {} and {}",
                Self::MIN,
                Self::MAX
            ))
        }
    }
}

impl From<Parallelism> for usize {
    fn from(value: Parallelism) -> Self {
        value.0
    }
}

/// Metadata tests for quick scans. None selects full content verification instead.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickCheck {
    None,
    Size,
    Timestamp,
    #[default]
    Both,
}

impl QuickCheck {
    pub fn uses_size(self) -> bool {
        matches!(self, Self::Size | Self::Both)
    }
    pub fn uses_timestamp(self) -> bool {
        matches!(self, Self::Timestamp | Self::Both)
    }
    pub fn is_default(&self) -> bool {
        *self == Self::Both
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "disabled (full comparison)",
            Self::Size => "size only",
            Self::Timestamp => "modified time only",
            Self::Both => "size + modified time",
        }
    }

    pub fn toggle_size(self) -> Self {
        match self {
            Self::Both => Self::Timestamp,
            Self::Timestamp => Self::Both,
            Self::Size => Self::None,
            Self::None => Self::Size,
        }
    }

    pub fn toggle_timestamp(self) -> Self {
        match self {
            Self::Both => Self::Size,
            Self::Size => Self::Both,
            Self::Timestamp => Self::None,
            Self::None => Self::Timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_checks_default_to_both_and_allow_disabled_selection() {
        assert_eq!(QuickCheck::default(), QuickCheck::Both);
        for (value, encoded, size, timestamp) in [
            (QuickCheck::None, "\"none\"", false, false),
            (QuickCheck::Both, "\"both\"", true, true),
            (QuickCheck::Size, "\"size\"", true, false),
            (QuickCheck::Timestamp, "\"timestamp\"", false, true),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), encoded);
            assert_eq!(serde_json::from_str::<QuickCheck>(encoded).unwrap(), value);
            assert_eq!(value.uses_size(), size);
            assert_eq!(value.uses_timestamp(), timestamp);
            assert_eq!(value.toggle_size().toggle_size(), value);
            assert_eq!(value.toggle_timestamp().toggle_timestamp(), value);
        }
        assert_eq!(QuickCheck::Both.toggle_timestamp(), QuickCheck::Size);
        assert_eq!(QuickCheck::Both.toggle_size(), QuickCheck::Timestamp);
        assert_eq!(QuickCheck::Size.toggle_size(), QuickCheck::None);
        assert_eq!(QuickCheck::None.toggle_size(), QuickCheck::Size);
        assert_eq!(QuickCheck::None.toggle_timestamp(), QuickCheck::Timestamp);
        assert_eq!(QuickCheck::Timestamp.toggle_timestamp(), QuickCheck::None);
        for invalid in ["null", "false", "0", "\"invalid\"", "\"size_only\"", "{}"] {
            assert!(
                serde_json::from_str::<QuickCheck>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn validates_limits_and_serializes_as_a_number() {
        assert_eq!(Parallelism::default().get(), 4);
        for count in Parallelism::MIN..=Parallelism::MAX {
            let value = Parallelism::try_from(count).unwrap();
            assert_eq!(serde_json::to_string(&value).unwrap(), count.to_string());
            assert_eq!(
                serde_json::from_str::<Parallelism>(&count.to_string()).unwrap(),
                value
            );
        }
        for invalid in [
            "0",
            "17",
            "-1",
            "1.5",
            "null",
            "\"4\"",
            "18446744073709551615",
        ] {
            assert!(
                serde_json::from_str::<Parallelism>(invalid).is_err(),
                "{invalid}"
            );
        }
        assert_eq!(Parallelism::try_from(1).unwrap().adjusted(false).get(), 1);
        assert_eq!(Parallelism::try_from(16).unwrap().adjusted(true).get(), 16);
    }
}
