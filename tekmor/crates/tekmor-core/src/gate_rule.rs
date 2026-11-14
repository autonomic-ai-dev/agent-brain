#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    Allow,
    Warn(String),
    Block(String),
}

#[derive(Debug, Clone, Copy)]
pub struct GateInput<'a> {
    pub tool: &'a str,
    pub command: Option<&'a str>,
}

pub trait GateRule {
    fn check(&self, input: &GateInput<'_>) -> GateVerdict;
}

pub fn evaluate(rules: &[&dyn GateRule], input: &GateInput<'_>) -> GateVerdict {
    let mut warning = None;
    for rule in rules {
        match rule.check(input) {
            GateVerdict::Block(reason) => return GateVerdict::Block(reason),
            GateVerdict::Warn(reason) => warning = warning.or(Some(reason)),
            GateVerdict::Allow => {}
        }
    }
    warning.map_or(GateVerdict::Allow, GateVerdict::Warn)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(GateVerdict);

    impl GateRule for Fixed {
        fn check(&self, _: &GateInput<'_>) -> GateVerdict {
            self.0.clone()
        }
    }

    fn input() -> GateInput<'static> {
        GateInput {
            tool: "Bash",
            command: Some("ls"),
        }
    }

    #[test]
    fn no_rules_or_only_allows_allow() {
        assert_eq!(evaluate(&[], &input()), GateVerdict::Allow);
        assert_eq!(evaluate(&[&Fixed(GateVerdict::Allow)], &input()), GateVerdict::Allow);
    }

    #[test]
    fn a_block_from_any_rule_wins_over_a_warning_from_an_earlier_one() {
        let warn = Fixed(GateVerdict::Warn("careful".into()));
        let block = Fixed(GateVerdict::Block("no".into()));
        assert_eq!(
            evaluate(&[&warn, &block], &input()),
            GateVerdict::Block("no".into())
        );
    }

    #[test]
    fn the_first_warning_is_kept_when_nothing_blocks() {
        let first = Fixed(GateVerdict::Warn("first".into()));
        let second = Fixed(GateVerdict::Warn("second".into()));
        assert_eq!(
            evaluate(&[&first, &second], &input()),
            GateVerdict::Warn("first".into())
        );
    }
}
