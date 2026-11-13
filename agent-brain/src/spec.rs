#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    pub text: String,
    pub check: Option<String>,
}

pub fn parse(text: &str) -> Vec<Requirement> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let (id, body) = requirement_head(line)?;
            let check = check_command(body).or_else(|| {
                lines
                    .get(index + 1)
                    .filter(|next| next.trim_start().starts_with("check:"))
                    .and_then(|next| check_command(next))
            });
            Some(Requirement {
                id,
                text: body.trim().to_string(),
                check,
            })
        })
        .collect()
}

fn requirement_head(line: &str) -> Option<(String, &str)> {
    let rest = line.trim_start().strip_prefix('R')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let body = rest[digits.len()..].strip_prefix('.')?;
    Some((format!("R{digits}"), body))
}

fn check_command(text: &str) -> Option<String> {
    let after = text.split_once("check:")?.1;
    let start = after.find('`')? + 1;
    let len = after[start..].find('`')?;
    let command = after[start..start + len].trim();
    (!command.is_empty()).then(|| command.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_id_text_and_same_line_check() {
        let requirements = parse("R1. Users can log in. check: `cargo test login`\n");
        assert_eq!(
            requirements,
            [Requirement {
                id: "R1".into(),
                text: "Users can log in. check: `cargo test login`".into(),
                check: Some("cargo test login".into()),
            }]
        );
    }

    #[test]
    fn reads_a_check_on_the_next_indented_line() {
        let requirements = parse("R2. Sessions expire.\n  check: `cargo test expiry`\n");
        assert_eq!(requirements[0].id, "R2");
        assert_eq!(requirements[0].check.as_deref(), Some("cargo test expiry"));
    }

    #[test]
    fn a_requirement_without_a_check_has_none() {
        let requirements = parse("R3. The page loads quickly.\n");
        assert_eq!(requirements[0].check, None);
    }

    #[test]
    fn a_check_without_backticks_is_not_a_check() {
        let requirements = parse("R4. Works. check: cargo test\n");
        assert_eq!(requirements[0].check, None);
    }

    #[test]
    fn ids_keep_all_their_digits() {
        let requirements = parse("R10. Tenth.\n");
        assert_eq!(requirements[0].id, "R10");
    }

    #[test]
    fn lines_that_are_not_requirements_are_ignored() {
        let text = "Rate limits apply.\nR. no number\nR5 no dot\n- R6. in a list\n## R7. heading\n";
        assert!(parse(text).is_empty());
    }
}
