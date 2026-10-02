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

pub fn classify_command(command: &str) -> RiskMatch {
    let normalized = command.to_ascii_lowercase();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.contains("rm -rf /")
        || collapsed.contains("sudo rm")
        || collapsed.contains("diskutil erase")
        || collapsed.contains("drop table")
        || collapsed.contains("truncate table")
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

#[cfg(test)]
mod tests {
    use super::{classify_command, RiskLevel};

    #[test]
    fn rm_root_is_critical() {
        let risk = classify_command("rm -rf /");
        assert_eq!(risk.level, RiskLevel::Critical);
    }

    #[test]
    fn ordinary_delete_is_not_blocking() {
        let risk = classify_command("echo delete button label");
        assert_eq!(risk.level, RiskLevel::Medium);
    }
}
