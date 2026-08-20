//! Config loading — the only fatal error class in hew; bad input lines pass
//! through, a bad config exits nonzero.

use std::{
    error::Error as StdError,
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::models::Config;

pub const DEFAULT_PATH: &str = "config.toml";

#[derive(Debug)]
pub enum Error {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, .. } => write!(f, "cannot read config {}", path.display()),
            Self::Parse { path, .. } => write!(f, "cannot parse config {}", path.display()),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

pub fn parse(source: &str, path: &Path) -> Result<Config, Error> {
    toml::from_str(source).map_err(|err| Error::Parse {
        path: path.to_path_buf(),
        source: Box::new(err),
    })
}

pub fn load(path: &Path) -> Result<Config, Error> {
    let source = fs::read_to_string(path).map_err(|err| Error::Read {
        path: path.to_path_buf(),
        source: err,
    })?;
    parse(&source, path)
}

#[cfg(test)]
mod tests {
    use std::{error::Error as StdError, path::Path};

    use super::{Error, load, parse};
    use crate::models::Condition;

    /// Mirrors the shipped `config.toml`; `tests/cli.rs` checks the real file.
    const SAMPLE: &str = r#"
[[sections]]
name = "timestamp"
attribute = "timestamp"
affixes = []

[[sections]]
name = "level"
attribute = "level"

[[sections.affixes]]
condition = { Equals = "INFO" }
prefix = "\u001b[32m"
suffix = "\u001b[0m"

[[sections.affixes]]
condition = "Always"
prefix = " "
suffix = " "

[[sections]]
name = "message"
attribute = "message"
affixes = []
"#;

    fn at(name: &str) -> &Path {
        Path::new(name)
    }

    #[test]
    fn parses_sections_in_declaration_order() {
        let config = parse(SAMPLE, at("<test>")).unwrap();
        let names: Vec<&str> = config.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["timestamp", "level", "message"]);
    }

    #[test]
    fn parses_both_serde_enum_representations() {
        let config = parse(SAMPLE, at("<test>")).unwrap();
        let level = &config.sections[1];
        assert_eq!(level.affixes.len(), 2);
        assert_eq!(
            level.affixes[0].condition,
            Condition::Equals("INFO".to_owned())
        );
        assert_eq!(level.affixes[1].condition, Condition::Always);
    }

    #[test]
    fn parses_ansi_escapes_from_toml_unicode_escapes() {
        let config = parse(SAMPLE, at("<test>")).unwrap();
        assert_eq!(
            config.sections[1].affixes[0].prefix.as_deref(),
            Some("\u{1b}[32m")
        );
        assert_eq!(
            config.sections[1].affixes[0].suffix.as_deref(),
            Some("\u{1b}[0m")
        );
    }

    #[test]
    fn a_missing_affixes_key_defaults_to_none() {
        let config = parse(SAMPLE, at("<test>")).unwrap();
        assert!(config.sections[0].affixes.is_empty());
    }

    #[test]
    fn syntactically_invalid_toml_is_an_error_not_a_panic() {
        let err = parse("this is not toml at all [[[", at("bad.toml")).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[test]
    fn a_schema_mismatch_is_an_error_not_a_panic() {
        let err = parse("[[sections]]\nname = 42\n", at("bad.toml")).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[test]
    fn an_unknown_condition_variant_is_an_error() {
        let source = "[[sections]]\nname = \"a\"\nattribute = \"a\"\n\
                      [[sections.affixes]]\ncondition = \"Sometimes\"\nprefix = \"\"\nsuffix = \"\"\n";
        assert!(matches!(
            parse(source, at("bad.toml")),
            Err(Error::Parse { .. })
        ));
    }

    #[test]
    fn loading_a_missing_file_reports_read_not_parse() {
        let err = load(at("/nonexistent/definitely/not/here/config.toml")).unwrap_err();
        assert!(matches!(err, Error::Read { .. }));
    }

    #[test]
    fn errors_display_the_offending_path_and_expose_a_source() {
        let err = load(at("/nonexistent/xyzzy.toml")).unwrap_err();
        assert!(format!("{err}").contains("xyzzy.toml"), "got: {err}");
        assert!(
            StdError::source(&err).is_some(),
            "the underlying cause must survive"
        );
    }
}
