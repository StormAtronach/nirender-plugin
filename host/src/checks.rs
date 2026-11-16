//! The checks of a session that a tool reads: for each, a fixed name, how many things it
//! judged, and how many faults it found.
//!
//! The report has many sentences with counts, and a tool once took its verdict from the words
//! of a sentence: a sentence that was reworded, or that the helper printed only when it had
//! something to say, read as "no fault". So each fact that a tool judges is a check here. The
//! helper prints one line for each check at the end of its report, always, in one form
//! ([`Check::line`]); `tools/common.py` (`helper_checks`) is the one reader, and its self-test
//! holds the form and the names against this file.
//!
//! `judged` says whether the check could judge at all: 0 things judged is no pass, and the
//! tool decides what that means for its capture. A check is one entry of `Player::checks`.
//!
//! A replay with a fault of a check ends with an exit code after its pictures and its report
//! ([`replay_exit_code`]). A live session never ends for a check.

use crate::stream_format::{EXIT_REPLAY_CHECKS, EXIT_REPLAY_STAGE_SHADERS};

/// The draws of a world that were in the target before the extension's parts began: judged are
/// the starts of a world for the extension, faults the scenes with such draws.
pub(crate) const EARLY_WORLD_DRAWS: &str = "early world draws";
/// The places of the views of their own (a save's picture, a mod's camera): judged are the
/// places of such views, faults those not drawn because their scene had no view.
pub(crate) const OWN_VIEWS: &str = "views of their own";
/// The reads past the end of a record or of a part of one: judged are the records read.
pub(crate) const READS_PAST_END: &str = "reads past the end";
/// The records of a tag that this build does not know: judged are the records read.
pub(crate) const UNKNOWN_RECORDS: &str = "unknown records";
/// The errors that the GPU's device gave: judged are the frames drawn.
pub(crate) const GPU_ERRORS: &str = "gpu errors";
/// The stage shaders: judged are the stage keys, faults the keys whose rewrite or pipeline
/// failed and, in a replay, the draws that the uber-shader took for a key or past the cap.
pub(crate) const STAGE_SHADERS: &str = "stage shaders";

/// One check: its name, the things it judged and the faults it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Check {
    pub(crate) name: &'static str,
    pub(crate) judged: u64,
    pub(crate) faults: u64,
}

impl Check {
    pub(crate) fn new(name: &'static str, judged: impl Into<u64>, faults: impl Into<u64>) -> Check {
        Check {
            name,
            judged: judged.into(),
            faults: faults.into(),
        }
    }

    /// The line of the check in the report. `tools/common.py` reads this form.
    pub(crate) fn line(&self) -> String {
        format!("check {}: {} judged, {} faults", self.name, self.judged, self.faults)
    }
}

/// The exit code of a replay with these checks, or None for a replay with no fault. A fault of
/// the stage shaders alone gives [`EXIT_REPLAY_STAGE_SHADERS`]: the pictures are right, and the
/// speed is lost. A fault of any other check gives [`EXIT_REPLAY_CHECKS`], also together with
/// a fault of the stage shaders: a fault that can change a picture comes first.
pub(crate) fn replay_exit_code(checks: &[Check]) -> Option<u32> {
    let faulty = |stage_shaders: bool| {
        checks
            .iter()
            .any(|check| check.faults != 0 && (check.name == STAGE_SHADERS) == stage_shaders)
    };
    match (faulty(false), faulty(true)) {
        (true, _) => Some(EXIT_REPLAY_CHECKS),
        (false, true) => Some(EXIT_REPLAY_STAGE_SHADERS),
        (false, false) => None,
    }
}

/// The line of a replay that ends with [`EXIT_REPLAY_CHECKS`]: each check with faults, but the
/// stage shaders, which have a line of their own. None when no such check has a fault.
pub(crate) fn replay_fault(checks: &[Check]) -> Option<String> {
    let faults: Vec<String> = checks
        .iter()
        .filter(|check| check.faults != 0 && check.name != STAGE_SHADERS)
        .map(|check| format!("{}: {} faults of {} judged", check.name, check.faults, check.judged))
        .collect();
    match faults.is_empty() {
        true => None,
        false => Some(format!("checks: a fault of the replay: {}", faults.join("; "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks(order_faults: u64, stage_faults: u64) -> Vec<Check> {
        vec![
            Check::new(EARLY_WORLD_DRAWS, 12u64, order_faults),
            Check::new(GPU_ERRORS, 30u64, 0u64),
            Check::new(STAGE_SHADERS, 13u64, stage_faults),
        ]
    }

    /// `tools/common.py` reads this form, and its self-test reads the format string in this
    /// file: a change here changes the reader with it.
    #[test]
    fn the_line_of_a_check_has_one_form() {
        assert_eq!(
            Check::new(EARLY_WORLD_DRAWS, 12u64, 0u64).line(),
            "check early world draws: 12 judged, 0 faults"
        );
        assert_eq!(
            Check::new(OWN_VIEWS, 0u32, 3u32).line(),
            "check views of their own: 0 judged, 3 faults"
        );
    }

    /// The names are lower-case words with single spaces: the reader's pattern takes no other.
    #[test]
    fn the_names_of_the_checks_fit_the_reader() {
        for name in [
            EARLY_WORLD_DRAWS,
            OWN_VIEWS,
            READS_PAST_END,
            UNKNOWN_RECORDS,
            GPU_ERRORS,
            STAGE_SHADERS,
        ] {
            let words: Vec<&str> = name.split(' ').collect();
            assert!(
                words
                    .iter()
                    .all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase())),
                "{name}"
            );
        }
    }

    /// No fault: no code. The stage shaders alone: their code. Any other check: the code of the
    /// checks, also when the stage shaders have a fault too.
    #[test]
    fn a_replay_ends_with_the_code_of_its_faults() {
        assert_eq!(replay_exit_code(&checks(0, 0)), None);
        assert_eq!(replay_exit_code(&checks(0, 9)), Some(EXIT_REPLAY_STAGE_SHADERS));
        assert_eq!(replay_exit_code(&checks(2, 0)), Some(EXIT_REPLAY_CHECKS));
        assert_eq!(replay_exit_code(&checks(2, 9)), Some(EXIT_REPLAY_CHECKS));
        assert_eq!((EXIT_REPLAY_STAGE_SHADERS, EXIT_REPLAY_CHECKS), (40, 41));
    }

    /// The fault line names each check with faults and its counts, and leaves the stage shaders
    /// to their own line.
    #[test]
    fn the_fault_line_names_each_check_with_faults() {
        assert_eq!(replay_fault(&checks(0, 0)), None);
        assert_eq!(replay_fault(&checks(0, 9)), None);
        assert_eq!(
            replay_fault(&checks(2, 9)).as_deref(),
            Some("checks: a fault of the replay: early world draws: 2 faults of 12 judged")
        );
        let mut two = checks(2, 0);
        two[1].faults = 1;
        assert_eq!(
            replay_fault(&two).as_deref(),
            Some(
                "checks: a fault of the replay: early world draws: 2 faults of 12 judged; gpu errors: 1 faults of 30 judged"
            )
        );
    }
}
