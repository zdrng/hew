//! `Condition` uses serde's externally tagged representation: a unit variant is
//! a bare string (`condition = "Always"`) and a newtype variant an inline table
//! (`condition = { Equals = "INFO" }`, `condition = { Regex = "^ERR" }`).

use regex_lite::Regex;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, Deserialize)]
pub enum Condition {
    Always,
    Equals(String),
    Contains(String),
    Regex(#[serde(deserialize_with = "deserialize_regex")] Regex),
}

fn deserialize_regex<'de, D>(deserializer: D) -> Result<Regex, D::Error>
where
    D: Deserializer<'de>,
{
    let pattern = String::deserialize(deserializer)?;
    Regex::new(&pattern).map_err(serde::de::Error::custom)
}

impl PartialEq for Condition {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Always, Self::Always) => true,
            (Self::Equals(a), Self::Equals(b)) | (Self::Contains(a), Self::Contains(b)) => a == b,
            (Self::Regex(a), Self::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl Eq for Condition {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Affix {
    pub condition: Condition,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

impl Affix {
    #[must_use]
    pub fn applies_to(&self, value: &str) -> bool {
        match &self.condition {
            Condition::Always => true,
            Condition::Equals(expected) => value == expected,
            Condition::Contains(needle) => value.contains(needle.as_str()),
            Condition::Regex(regex) => regex.is_match(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Section {
    pub name: String,
    pub attribute: String,
    pub affixes: Vec<Affix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub sections: Vec<Section>,
}

#[cfg(test)]
mod tests {
    use super::{Affix, Condition, Regex};

    fn affix(condition: Condition) -> Affix {
        Affix {
            condition,
            prefix: Some("<".to_owned()),
            suffix: Some(">".to_owned()),
        }
    }

    #[test]
    fn always_applies_to_anything() {
        let affix = affix(Condition::Always);
        assert!(affix.applies_to("INFO"));
        assert!(affix.applies_to(""));
        assert!(affix.applies_to("  \n  "));
    }

    #[test]
    fn equals_matches_only_the_exact_string() {
        let affix = affix(Condition::Equals("INFO".to_owned()));
        assert!(affix.applies_to("INFO"));
        assert!(
            !affix.applies_to("INFORMATION"),
            "must not match a longer string"
        );
        assert!(!affix.applies_to("INF"), "must not match a prefix");
        assert!(!affix.applies_to(""));
    }

    #[test]
    fn equals_is_case_sensitive() {
        let affix = affix(Condition::Equals("INFO".to_owned()));
        assert!(!affix.applies_to("info"));
        assert!(!affix.applies_to("Info"));
    }

    #[test]
    fn contains_matches_a_substring_anywhere() {
        let affix = affix(Condition::Contains("err".to_owned()));
        assert!(affix.applies_to("err"));
        assert!(affix.applies_to("stderr"));
        assert!(affix.applies_to("errno 2"));
        assert!(!affix.applies_to("ERR"), "must be case sensitive");
        assert!(!affix.applies_to("warning"));
    }

    #[test]
    fn contains_with_an_empty_needle_matches_everything() {
        // `str::contains("")` is true for every string, including the empty one.
        let affix = affix(Condition::Contains(String::new()));
        assert!(affix.applies_to("anything"));
        assert!(affix.applies_to(""));
    }

    #[test]
    fn regex_matches_by_pattern() {
        let affix = affix(Condition::Regex(
            Regex::new("^(ERROR|WARN)$").expect("valid pattern"),
        ));
        assert!(affix.applies_to("ERROR"));
        assert!(affix.applies_to("WARN"));
        assert!(!affix.applies_to("WARNING"), "anchors must hold");
        assert!(!affix.applies_to("error"));
    }

    #[test]
    fn regex_condition_deserialises_from_toml() {
        let affix: Affix =
            toml::from_str(r#"condition = { Regex = "^ERR" }"#).expect("valid config");
        assert!(affix.applies_to("ERROR"));
        assert!(!affix.applies_to("no match"));
    }

    #[test]
    fn invalid_regex_fails_at_deserialisation() {
        let result: Result<Affix, _> = toml::from_str(r#"condition = { Regex = "(" }"#);
        assert!(result.is_err());
    }
}
