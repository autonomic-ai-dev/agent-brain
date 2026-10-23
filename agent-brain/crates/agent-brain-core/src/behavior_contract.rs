use crate::types::MustApply;

pub fn rules() -> Vec<MustApply> {
    [
        (
            "search-before-asking",
            "For any task, search the repository first (grep_search, search_symbols, query_codebase, or read the named files) and proceed. Ask the user only for facts the repository cannot answer, and say what you searched.",
        ),
        (
            "evidence-required",
            "Every claim about this code cites file:line from a read or search in this turn. If there is no evidence, write \"unverified\" instead of guessing.",
        ),
        (
            "no-invented-names",
            "Do not name a function, type, flag, file, or command unless you confirmed it exists in this turn.",
        ),
        (
            "verify-before-done",
            "Do not claim tests or checks pass without a recorded `agent-brain verify -- <cmd>` run that exited 0.",
        ),
        (
            "one-question-at-a-time",
            "When a question is unavoidable, ask one, state the assumption you would otherwise make, and continue on that assumption if the user does not answer.",
        ),
    ]
    .into_iter()
    .map(|(topic, text)| MustApply {
        topic: topic.into(),
        text: text.into(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_asks_agents_to_search_the_repo_before_asking_the_user() {
        let rules = rules();
        let search = rules
            .iter()
            .find(|rule| rule.topic == "search-before-asking")
            .expect("search-before-asking rule must be present");
        assert!(search.text.contains("search the repository first"));
        assert!(search.text.contains("Ask the user only"));
    }

    #[test]
    fn contract_requires_evidence_and_forbids_invented_names() {
        let rules = rules();
        for topic in [
            "evidence-required",
            "no-invented-names",
            "verify-before-done",
        ] {
            assert!(
                rules.iter().any(|rule| rule.topic == topic),
                "missing rule {topic}"
            );
        }
    }
}
