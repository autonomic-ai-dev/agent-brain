use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::types::MustApply;

const RESEND_AFTER: Duration = Duration::from_secs(30 * 60);
pub const STILL_APPLIES_TOPIC: &str = "still-applies";

type Sent = HashMap<(String, String), Instant>;

fn key(rule: &MustApply) -> (String, String) {
    (rule.topic.clone(), rule.text.clone())
}

pub fn collapse_repeats(
    must_apply: &[MustApply],
    sent: &Sent,
    now: Instant,
) -> (Vec<MustApply>, Sent) {
    let (repeated, fresh): (Vec<&MustApply>, Vec<&MustApply>) =
        must_apply.iter().partition(|rule| {
            sent.get(&key(rule))
                .is_some_and(|at| now.duration_since(*at) < RESEND_AFTER)
        });

    let next: Sent = sent
        .iter()
        .map(|(sent_key, at)| (sent_key.clone(), *at))
        .chain(fresh.iter().map(|rule| (key(rule), now)))
        .collect();

    let summary = (!repeated.is_empty()).then(|| MustApply {
        topic: STILL_APPLIES_TOPIC.to_string(),
        text: format!(
            "{} constraints sent earlier in this session still apply unchanged: {}.",
            repeated.len(),
            repeated
                .iter()
                .map(|rule| rule.topic.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    });

    let collapsed = fresh.into_iter().cloned().chain(summary).collect();
    (collapsed, next)
}

#[derive(Debug, Default)]
pub struct SentConstraints {
    sent: Mutex<Sent>,
}

impl SentConstraints {
    pub fn collapse(&self, must_apply: &[MustApply]) -> Vec<MustApply> {
        let Ok(mut sent) = self.sent.lock() else {
            return must_apply.to_vec();
        };
        let (collapsed, next) = collapse_repeats(must_apply, &sent, Instant::now());
        *sent = next;
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(topic: &str, text: &str) -> MustApply {
        MustApply {
            topic: topic.to_string(),
            text: text.to_string(),
        }
    }

    fn topics(rules: &[MustApply]) -> Vec<&str> {
        rules.iter().map(|rule| rule.topic.as_str()).collect()
    }

    #[test]
    fn the_first_route_sends_every_constraint_in_full() {
        let rules = [
            rule("evidence", "cite file:line"),
            rule("names", "no guesses"),
        ];
        let (collapsed, sent) = collapse_repeats(&rules, &Sent::new(), Instant::now());
        assert_eq!(topics(&collapsed), ["evidence", "names"]);
        assert_eq!(sent.len(), 2);
    }

    #[test]
    fn a_repeat_route_names_unchanged_constraints_in_one_line() {
        let rules = [
            rule("evidence", "cite file:line"),
            rule("names", "no guesses"),
        ];
        let start = Instant::now();
        let (_, sent) = collapse_repeats(&rules, &Sent::new(), start);

        let (collapsed, _) = collapse_repeats(&rules, &sent, start + Duration::from_secs(60));
        assert_eq!(topics(&collapsed), [STILL_APPLIES_TOPIC]);
        assert_eq!(
            collapsed[0].text,
            "2 constraints sent earlier in this session still apply unchanged: evidence, names."
        );
    }

    #[test]
    fn a_new_or_changed_constraint_is_sent_in_full_beside_the_summary() {
        let start = Instant::now();
        let (_, sent) = collapse_repeats(
            &[
                rule("evidence", "cite file:line"),
                rule("recipe", "cargo test"),
            ],
            &Sent::new(),
            start,
        );

        let (collapsed, _) = collapse_repeats(
            &[
                rule("evidence", "cite file:line"),
                rule("recipe", "cargo test --lib"),
                rule("trap", "never read dist"),
            ],
            &sent,
            start + Duration::from_secs(60),
        );
        assert_eq!(topics(&collapsed), ["recipe", "trap", STILL_APPLIES_TOPIC]);
        assert_eq!(collapsed[0].text, "cargo test --lib");
    }

    #[test]
    fn constraints_are_resent_in_full_after_the_resend_window() {
        let rules = [rule("evidence", "cite file:line")];
        let start = Instant::now();
        let (_, sent) = collapse_repeats(&rules, &Sent::new(), start);

        let (collapsed, resent) = collapse_repeats(&rules, &sent, start + RESEND_AFTER);
        assert_eq!(topics(&collapsed), ["evidence"]);

        let (again, _) = collapse_repeats(
            &rules,
            &resent,
            start + RESEND_AFTER + Duration::from_secs(60),
        );
        assert_eq!(topics(&again), [STILL_APPLIES_TOPIC]);
    }

    #[test]
    fn a_route_with_no_constraints_stays_empty() {
        let (collapsed, sent) = collapse_repeats(&[], &Sent::new(), Instant::now());
        assert!(collapsed.is_empty());
        assert!(sent.is_empty());
    }

    #[test]
    fn each_session_tracks_its_own_constraints() {
        let rules = [rule("evidence", "cite file:line")];
        let first = SentConstraints::default();
        let second = SentConstraints::default();
        first.collapse(&rules);
        assert_eq!(topics(&first.collapse(&rules)), [STILL_APPLIES_TOPIC]);
        assert_eq!(topics(&second.collapse(&rules)), ["evidence"]);
    }
}
