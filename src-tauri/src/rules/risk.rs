use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    Critical,
    High,
    Medium,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskMatch {
    pub level: RiskLevel,
    pub rule: String,
}

/// Whole-argument targets that make a recursive delete wipe the disk or home.
const ROOT_TARGETS: [&str; 11] = [
    "/", "/*", "~", "~/", "~/*", "$home", "$home/", "$home/*", "${home}", "${home}/", "${home}/*",
];
const FIND_ROOTS: [&str; 5] = ["/", "~", "~/", "$home", "${home}"];

pub fn classify_command(command: &str) -> RiskMatch {
    let normalized = command.to_ascii_lowercase();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.contains("rm -rf /")
        || collapsed.contains("sudo rm")
        || collapsed.contains("diskutil erase")
        || collapsed.contains("drop table")
        || collapsed.contains("truncate table")
        || has_destructive_segment(&collapsed)
    {
        return RiskMatch {
            level: RiskLevel::Critical,
            rule: "critical-destructive-command".to_string(),
        };
    }

    if collapsed.contains("chmod 777")
        || collapsed.contains("rm -rf")
        || collapsed.contains("docker system prune")
        || collapsed.contains("git clean -fd")
    {
        return RiskMatch {
            level: RiskLevel::High,
            rule: "high-risk-command".to_string(),
        };
    }

    if collapsed.contains(" delete ") || collapsed.starts_with("delete ") {
        return RiskMatch {
            level: RiskLevel::Medium,
            rule: "medium-delete-token".to_string(),
        };
    }

    RiskMatch {
        level: RiskLevel::None,
        rule: "none".to_string(),
    }
}

/// Catches spellings the substring checks miss (`rm -fr /`, `rm -r -f ~`,
/// `sudo -u x rm`, `find / -delete`). Quotes are dropped and `; & | ( )` end a
/// command segment. Cases live in risk_cases.tsv.
fn has_destructive_segment(collapsed: &str) -> bool {
    let spaced: String = collapsed
        .chars()
        .map(|c| match c {
            '\\' | '"' | '\'' | '`' => ' ',
            ';' | '&' | '|' | '(' | ')' => ';',
            other => other,
        })
        .collect();
    spaced.split(';').any(|segment| {
        let words: Vec<&str> = segment.split_whitespace().collect();
        words.iter().enumerate().any(|(i, word)| {
            let rest = &words[i + 1..];
            if is_program(word, "rm") {
                rest.iter().any(|w| ROOT_TARGETS.contains(w))
            } else if *word == "sudo" {
                rest.iter().any(|w| is_program(w, "rm"))
            } else if is_program(word, "find") {
                rest.iter()
                    .position(|w| FIND_ROOTS.contains(w))
                    .map(|at| rest[at + 1..].contains(&"-delete"))
                    .unwrap_or(false)
            } else {
                false
            }
        })
    })
}

fn is_program(word: &str, name: &str) -> bool {
    word == name || word.strip_suffix(name).is_some_and(|prefix| prefix.ends_with('/'))
}

#[cfg(test)]
mod tests {
    use super::{classify_command, RiskLevel};

    #[test]
    fn rm_root_is_critical() {
        let risk = classify_command("rm -rf /");
        assert_eq!(risk.level, RiskLevel::Critical);
    }

    #[test]
    fn shared_cases_match() {
        for line in include_str!("risk_cases.tsv").lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (expected, command) = line.split_once('\t').unwrap();
            let critical = classify_command(command).level == RiskLevel::Critical;
            assert_eq!(critical, expected == "critical", "{command:?}");
        }
    }

    #[test]
    fn ordinary_delete_is_not_blocking() {
        let risk = classify_command("echo delete button label");
        assert_eq!(risk.level, RiskLevel::Medium);
    }
}
