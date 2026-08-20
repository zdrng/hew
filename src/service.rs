use std::{
    borrow::Cow,
    io::{self, BufRead, BufWriter, IsTerminal, Write},
};

use serde_json::{Map, Value};

use crate::models::{Config, Section};

pub const OUTPUT_BUFFER_BYTES: usize = 64 * 1024;

pub fn run(config: &Config, force_flush: bool) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let flush_each_line = force_flush || stdout.is_terminal();
    let output = BufWriter::with_capacity(OUTPUT_BUFFER_BYTES, stdout.lock());

    filter(stdin.lock(), output, config, flush_each_line)
}

pub fn filter<R, W>(
    mut input: R,
    mut output: W,
    config: &Config,
    flush_each_line: bool,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    let mut raw: Vec<u8> = Vec::new();
    let mut formatted = String::new();

    loop {
        raw.clear();
        if input.read_until(b'\n', &mut raw)? == 0 {
            break;
        }

        let body = raw.strip_suffix(b"\n").unwrap_or(&raw);
        let body = body.strip_suffix(b"\r").unwrap_or(body);

        let line =
            std::str::from_utf8(body).map_or_else(|_| String::from_utf8_lossy(body), Cow::Borrowed);

        formatted.clear();
        format_line(&line, &config.sections, &mut formatted);

        output.write_all(formatted.as_bytes())?;
        output.write_all(b"\n")?;
        if flush_each_line {
            output.flush()?;
        }
    }

    output.flush()
}

fn format_line(line: &str, sections: &[Section], out: &mut String) {
    let Ok(parsed) = serde_json::from_str::<Value>(line) else {
        out.push_str(line);
        return;
    };
    let Some(fields) = parsed.as_object() else {
        out.push_str(line);
        return;
    };

    let mut matched = false;
    for section in sections {
        if format_section(fields, section, out) {
            matched = true;
        }
    }

    if !matched {
        out.clear();
        out.push_str(line);
    }
}

fn format_section(fields: &Map<String, Value>, section: &Section, out: &mut String) -> bool {
    let Some(field) = fields.get(&section.attribute) else {
        return false;
    };

    let value = field
        .as_str()
        .map_or_else(|| Cow::Owned(field.to_string()), Cow::Borrowed);

    // Prefixes in reverse order, suffixes forward, so earlier affixes nest
    // outside later ones.
    for affix in section.affixes.iter().rev() {
        if affix.applies_to(&value)
            && let Some(prefix) = &affix.prefix
        {
            out.push_str(prefix);
        }
    }
    out.push_str(&value);
    for affix in &section.affixes {
        if affix.applies_to(&value)
            && let Some(suffix) = &affix.suffix
        {
            out.push_str(suffix);
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{filter, format_line};
    use crate::models::{Affix, Condition, Config, Section};

    fn affix(condition: Condition, prefix: &str, suffix: &str) -> Affix {
        Affix {
            condition,
            prefix: Some(prefix.to_owned()),
            suffix: Some(suffix.to_owned()),
        }
    }

    fn section(attribute: &str, affixes: Vec<Affix>) -> Section {
        Section {
            name: attribute.to_owned(),
            attribute: attribute.to_owned(),
            affixes,
        }
    }

    fn config() -> Config {
        Config {
            sections: vec![
                section("timestamp", vec![]),
                section(
                    "level",
                    vec![
                        affix(Condition::Equals("INFO".to_owned()), "<g>", "</g>"),
                        affix(Condition::Always, " ", " "),
                    ],
                ),
                section("message", vec![]),
            ],
        }
    }

    fn format(line: &str) -> String {
        let mut out = String::new();
        format_line(line, &config().sections, &mut out);
        out
    }

    #[test]
    fn formats_a_complete_object() {
        let out = format(r#"{"timestamp":"T","level":"INFO","message":"hello"}"#);
        assert_eq!(out, "T <g>INFO</g> hello");
    }

    #[test]
    fn affixes_nest_outermost_first() {
        // The `Equals` affix is declared before the `Always` affix, so its
        // prefix must sit outside the space and its suffix inside it. This is
        // the one piece of non-obvious ordering logic in the crate.
        let out = format(r#"{"timestamp":"T","level":"INFO","message":"m"}"#);
        assert_eq!(
            out, "T <g>INFO</g> m",
            "expected <g> outside the padding space"
        );
        assert!(
            out.contains("<g>INFO</g>"),
            "suffix must close before the trailing space"
        );
    }

    #[test]
    fn a_condition_that_does_not_hold_contributes_nothing() {
        let out = format(r#"{"timestamp":"T","level":"WARN","message":"m"}"#);
        assert_eq!(out, "T WARN m");
    }

    #[test]
    fn non_string_values_render_through_display() {
        let config = Config {
            sections: vec![section("v", vec![])],
        };
        let render = |line: &str| {
            let mut out = String::new();
            format_line(line, &config.sections, &mut out);
            out
        };
        assert_eq!(render(r#"{"v":42}"#), "42");
        assert_eq!(render(r#"{"v":true}"#), "true");
        assert_eq!(render(r#"{"v":null}"#), "null");
        assert_eq!(render(r#"{"v":1.5}"#), "1.5");
        assert_eq!(render(r#"{"v":[1,2]}"#), "[1,2]");
        assert_eq!(render(r#"{"v":{"a":1}}"#), r#"{"a":1}"#);
    }

    #[test]
    fn malformed_json_passes_through_verbatim() {
        let line = "this is not JSON at all";
        assert_eq!(format(line), line);
        let truncated = r#"{"timestamp":"T","level":"#;
        assert_eq!(format(truncated), truncated);
    }

    #[test]
    fn valid_json_that_is_not_an_object_passes_through_verbatim() {
        for line in [r"[1,2]", r#""a string""#, "42", "true", "null"] {
            assert_eq!(format(line), line, "{line} should have passed through");
        }
    }

    #[test]
    fn a_missing_middle_attribute_skips_only_its_own_section() {
        // `level` is absent, so timestamp and message still render. They end up
        // adjacent because the only separator in this config is `level`'s Always
        // affix, and a skipped section takes its affixes with it. Ragged, and
        // deliberately so: compensating here would fabricate spacing the config
        // never asked for.
        assert_eq!(format(r#"{"timestamp":"T","message":"m"}"#), "Tm");
    }

    #[test]
    fn a_missing_first_attribute_leaves_the_following_affixes_in_place() {
        // No timestamp: the line opens with the space from `level`'s Always
        // prefix rather than being trimmed back.
        assert_eq!(
            format(r#"{"level":"INFO","message":"m"}"#),
            " <g>INFO</g> m"
        );
    }

    #[test]
    fn a_missing_last_attribute_still_renders_the_rest() {
        assert_eq!(
            format(r#"{"timestamp":"T","level":"INFO"}"#),
            "T <g>INFO</g> "
        );
    }

    #[test]
    fn an_object_matching_no_section_passes_through_verbatim() {
        // Another producer's JSON in the same stream. Nothing to format, so the
        // raw line is more use than the blank one a strict skip would produce.
        let line = r#"{"foo":1,"bar":"baz"}"#;
        assert_eq!(format(line), line);
    }

    #[test]
    fn an_empty_object_passes_through_verbatim() {
        assert_eq!(format("{}"), "{}");
    }

    #[test]
    fn an_empty_line_stays_empty() {
        assert_eq!(format(""), "");
    }

    #[test]
    fn escaped_newlines_and_tabs_are_unescaped_into_the_output() {
        // The stack-trace case benchmark.sh generates: real newlines and tabs
        // embedded in a JSON string. They must survive into the output as real
        // control characters, which is also why the benchmark counts lines on
        // the producer side.
        let line = r#"{"timestamp":"T","level":"ERROR","message":"boom\n\tat Foo.java:1"}"#;
        assert_eq!(format(line), "T ERROR boom\n\tat Foo.java:1");
    }

    #[test]
    fn extra_attributes_in_the_input_are_ignored() {
        let line = r#"{"timestamp":"T","level":"INFO","message":"m","package":"db","extra":1}"#;
        assert_eq!(format(line), "T <g>INFO</g> m");
    }

    #[test]
    fn a_config_with_no_sections_passes_everything_through() {
        // With nothing declared no section can match, which is the no-match
        // fallback: the stream comes out as it went in rather than as blanks.
        let empty = Config { sections: vec![] };
        let mut out = String::new();
        format_line(r#"{"a":1}"#, &empty.sections, &mut out);
        assert_eq!(out, r#"{"a":1}"#);
    }

    // --- stream level -------------------------------------------------------

    fn run_filter(input: &str) -> String {
        let mut out: Vec<u8> = Vec::new();
        filter(input.as_bytes(), &mut out, &config(), false).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_mixed_stream_formats_what_it_can_and_passes_the_rest() {
        let input = concat!(
            r#"{"timestamp":"T","level":"INFO","message":"one"}"#,
            "\n",
            "a plain log line\n",
            r#"{"timestamp":"T","level":"WARN","message":"two"}"#,
            "\n",
        );
        assert_eq!(
            run_filter(input),
            "T <g>INFO</g> one\na plain log line\nT WARN two\n"
        );
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert_eq!(run_filter(""), "");
    }

    #[test]
    fn a_final_line_without_a_newline_is_still_emitted() {
        // read_until returns the trailing fragment with no terminator; dropping
        // it would silently swallow the last line of a truncated file.
        assert_eq!(run_filter("plain"), "plain\n");
    }

    #[test]
    fn crlf_terminators_are_stripped() {
        assert_eq!(run_filter("plain\r\n"), "plain\n");
    }

    #[test]
    fn blank_lines_are_preserved() {
        assert_eq!(run_filter("\n\n"), "\n\n");
    }

    #[test]
    fn invalid_utf8_does_not_abort_the_stream() {
        // A stray non-UTF-8 byte becomes U+FFFD rather than killing the process.
        // Lossy replacement is a deliberate trade: the alternative is failing,
        // and the pass-through contract forbids that.
        let mut input: Vec<u8> = Vec::new();
        input.extend_from_slice(b"before\n");
        input.extend_from_slice(&[0xff, 0xfe, b'\n']);
        input.extend_from_slice(b"after\n");

        let mut out: Vec<u8> = Vec::new();
        filter(input.as_slice(), &mut out, &config(), false).unwrap();
        let text = String::from_utf8(out).unwrap();

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "every line must survive: {text:?}");
        assert_eq!(lines[0], "before");
        assert_eq!(lines[2], "after");
        assert!(lines[1].contains('\u{fffd}'), "got: {:?}", lines[1]);
    }
}
