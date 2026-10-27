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

/// One log line with every switch that is set, for the start of the log.
pub fn log_set() {
    crate::log!("{}", set_line(std::env::vars()));
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

    #[test]
    fn on_takes_1_and_0_only() {
        assert_eq!(parse_on("1"), Some(true));
        assert_eq!(parse_on("0"), Some(false));
        assert_eq!(parse_on("yes"), None);
        assert_eq!(parse_on(""), None);
    }

    #[test]
    fn a_set_switch_is_read_through_each_form() {
        std::env::set_var("NIRENDER_TEST_SWITCH_ON", "1");
        std::env::set_var("NIRENDER_TEST_SWITCH_NUMBER", "12");
        std::env::set_var("NIRENDER_TEST_SWITCH_CHOICE", "check");
        assert_eq!(on("NIRENDER_TEST_SWITCH_ON"), Some(true));
        assert_eq!(on("NIRENDER_TEST_SWITCH_NOT_SET"), None);
        assert_eq!(number("NIRENDER_TEST_SWITCH_NUMBER", 3u32), 12);
        assert_eq!(number("NIRENDER_TEST_SWITCH_NOT_SET", 3u32), 3);
        assert_eq!(number_from_to("NIRENDER_TEST_SWITCH_NUMBER", 64u32, 1, 4096), 12);
        assert_eq!(choice("NIRENDER_TEST_SWITCH_CHOICE", &["0", "check"]), Some("check"));
        assert_eq!(parsed("NIRENDER_TEST_SWITCH_CHOICE", "check", |value| (value == "check").then_some(7)), Some(7));
    }

    #[test]
    fn a_bad_value_gives_the_default_and_a_line() {
        std::env::set_var("NIRENDER_TEST_SWITCH_BAD", "many");
        assert_eq!(on("NIRENDER_TEST_SWITCH_BAD"), None);
        assert_eq!(number("NIRENDER_TEST_SWITCH_BAD", 3u32), 3);
        assert_eq!(number_from_to("NIRENDER_TEST_SWITCH_BAD", 64u32, 1, 4096), 64);
        assert_eq!(choice("NIRENDER_TEST_SWITCH_BAD", &["0", "check"]), None);
        std::env::set_var("NIRENDER_TEST_SWITCH_LARGE", "5000");
        assert_eq!(number_from_to("NIRENDER_TEST_SWITCH_LARGE", 64u32, 1, 4096), 64);
        assert_eq!(bad_value_line("NIRENDER_X", "many", "1 or 0", "not set"), "switch NIRENDER_X=many: not 1 or 0; not set");
    }

    #[test]
    fn the_set_line_names_the_switches_in_order() {
        let variables = [("PATH", "x"), ("NIRENDER_B", "2"), ("MGE_XE_A", "1"), ("HOME", "y")];
        let line = set_line(variables.iter().map(|(name, value)| (name.to_string(), value.to_string())));
        assert_eq!(line, "switches set: MGE_XE_A=1, NIRENDER_B=2");
        assert_eq!(set_line(std::iter::empty()), "switches: none set");
    }
}
