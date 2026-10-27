//! The switches of the environment that the plugin reads (`NIRENDER_*`, `MGE_XE_*`): every
//! read goes through here. At the install the log gets one line with each switch that is set
//! and its value, so that a leftover variable of a test leaves a trace. A value that a read
//! cannot take gives one line that names the switch, the value and what is taken instead, in
//! place of a silent fallback. README.md names every switch, and `tools/check_readme.py`
//! checks that it does.

use core::fmt::Display;
use core::str::FromStr;

/// The prefixes of the switches of the plugin and of the helper.
const PREFIXES: [&str; 3] = ["NIRENDER_", "MGE_XE_", "WGPU_"];

/// The value of a switch, when it is set.
pub fn text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A switch that is on (`1`) or off (`0`): `None` when it is not set, or when it has another
/// value, which the log names.
pub fn on(name: &str) -> Option<bool> {
    let value = text(name)?;
    report(name, &value, parse_on(&value), "1 or 0", "not set")
}

/// A number; a value that is not one is named in the log, and `default` is taken.
pub fn number<T: FromStr + Display + Copy>(name: &str, default: T) -> T {
    let Some(value) = text(name) else {
        return default;
    };
    report(name, &value, value.parse().ok(), "a number", &default.to_string()).unwrap_or(default)
}

/// A number from `least` to `most`; a value outside, or not a number, is named in the log, and
/// `default` is taken.
pub fn number_from_to<T: FromStr + Display + Copy + PartialOrd>(name: &str, default: T, least: T, most: T) -> T {
    let Some(value) = text(name) else {
        return default;
    };
    let number = value.parse().ok().filter(|number| (least..=most).contains(number));
    report(name, &value, number, &format!("a number from {least} to {most}"), &default.to_string()).unwrap_or(default)
}

/// One of the values in `allowed`: `None` when the switch is not set, or when it has another
/// value, which the log names with the values allowed.
pub fn choice<'a>(name: &str, allowed: &[&'a str]) -> Option<&'a str> {
    let value = text(name)?;
    let choice = allowed.iter().copied().find(|allowed| *allowed == value);
    report(name, &value, choice, &allowed.join(", "), "not set")
}

/// The value of a switch through `parse`; a value that `parse` refuses is named in the log
/// with `expected`, the values that it takes.
pub fn parsed<T>(name: &str, expected: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    let value = text(name)?;
    report(name, &value, parse(&value), expected, "not set")
}

/// `1` is on and `0` is off.
fn parse_on(value: &str) -> Option<bool> {
    match value {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// Gives `parsed` back; for a value that did not parse, the log names the switch, the value,
/// the values expected and what is taken instead.
fn report<T>(name: &str, value: &str, parsed: Option<T>, expected: &str, taken: &str) -> Option<T> {
    if parsed.is_none() {
        crate::log!("{}", bad_value_line(name, value, expected, taken));
    }
    parsed
}

fn bad_value_line(name: &str, value: &str, expected: &str, taken: &str) -> String {
    format!("switch {name}={value}: not {expected}; {taken}")
}

/// One log line with every switch that is set, for the start of the log. A name or a value
/// that is not Unicode is shown lossily (`vars` would panic on it).
pub fn log_set() {
    let variables = std::env::vars_os().map(|(name, value)| (name.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()));
    crate::log!("{}", set_line(variables));
}

/// The switches among `variables`, in the order of their names.
fn set_line(variables: impl Iterator<Item = (String, String)>) -> String {
    let mut set: Vec<String> = variables.filter(|(name, _)| PREFIXES.iter().any(|prefix| name.starts_with(prefix))).map(|(name, value)| format!("{name}={value}")).collect();
    set.sort();
    if set.is_empty() {
        "switches: none set".to_string()
    } else {
        format!("switches set: {}", set.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name of a test switch, from the prefix: a name in quotes would look like a read of
    /// the code to `tools/check_readme.py`, which wants every read in README.md.
    fn test_switch(suffix: &str) -> String {
        format!("{}TEST_SWITCH_{suffix}", PREFIXES[0])
    }

    #[test]
    fn on_takes_1_and_0_only() {
        assert_eq!(parse_on("1"), Some(true));
        assert_eq!(parse_on("0"), Some(false));
        assert_eq!(parse_on("yes"), None);
        assert_eq!(parse_on(""), None);
    }

    #[test]
    fn a_set_switch_is_read_through_each_form() {
        let (on_name, number_name, choice_name, not_set) = (test_switch("ON"), test_switch("NUMBER"), test_switch("CHOICE"), test_switch("NOT_SET"));
        std::env::set_var(&on_name, "1");
        std::env::set_var(&number_name, "12");
        std::env::set_var(&choice_name, "check");
        assert_eq!(on(&on_name), Some(true));
        assert_eq!(on(&not_set), None);
        assert_eq!(number(&number_name, 3u32), 12);
        assert_eq!(number(&not_set, 3u32), 3);
        assert_eq!(number_from_to(&number_name, 64u32, 1, 512), 12);
        assert_eq!(choice(&choice_name, &["0", "check"]), Some("check"));
        assert_eq!(parsed(&choice_name, "check", |value| (value == "check").then_some(7)), Some(7));
    }

    #[test]
    fn a_bad_value_gives_the_default_and_a_line() {
        let (bad, large) = (test_switch("BAD"), test_switch("LARGE"));
        std::env::set_var(&bad, "many");
        assert_eq!(on(&bad), None);
        assert_eq!(number(&bad, 3u32), 3);
        assert_eq!(number_from_to(&bad, 64u32, 1, 512), 64);
        assert_eq!(choice(&bad, &["0", "check"]), None);
        std::env::set_var(&large, "5000");
        assert_eq!(number_from_to(&large, 64u32, 1, 512), 64);
        assert_eq!(bad_value_line(&test_switch("X"), "many", "1 or 0", "not set"), format!("switch {}: not 1 or 0; not set", format!("{}=many", test_switch("X"))));
    }

    #[test]
    fn the_set_line_names_the_switches_in_order() {
        let (b, a) = (format!("{}B", PREFIXES[0]), format!("{}A", PREFIXES[1]));
        let variables = [("PATH", "x"), (b.as_str(), "2"), (a.as_str(), "1"), ("HOME", "y")];
        let line = set_line(variables.iter().map(|(name, value)| (name.to_string(), value.to_string())));
        assert_eq!(line, format!("switches set: {a}=1, {b}=2"));
        assert_eq!(set_line(std::iter::empty()), "switches: none set");
    }
}
