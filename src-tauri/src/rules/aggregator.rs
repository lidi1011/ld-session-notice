use crate::domain::{NoticeEvent, NoticeEventType};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AggregationDecision {
    Record,
    NotifyNow(String),
    Aggregate,
}

#[allow(dead_code)]
pub fn decide(event: &NoticeEvent, consecutive_failures: i64) -> AggregationDecision {
    match event.event_type {
        NoticeEventType::UserConfirm => AggregationDecision::NotifyNow(event.title.clone()),
        NoticeEventType::TaskFinish => AggregationDecision::NotifyNow(event.title.clone()),
        NoticeEventType::TaskFail if consecutive_failures >= 5 => {
            AggregationDecision::NotifyNow("Continuous failures detected".to_string())
        }
        NoticeEventType::TaskFail => AggregationDecision::Aggregate,
        NoticeEventType::SessionIdle | NoticeEventType::SessionEnd | NoticeEventType::TaskStart => AggregationDecision::Record,
        NoticeEventType::Warning | NoticeEventType::Error => AggregationDecision::Aggregate,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::domain::{NoticeEvent, NoticeEventType, NoticeLevel, Provider};

    use super::{decide, AggregationDecision};

    fn event(event_type: NoticeEventType) -> NoticeEvent {
        NoticeEvent {
            id: Uuid::new_v4().to_string(),
            version: 1,
            provider: Provider::Codex,
            event_type,
            session_id: Some("s1".to_string()),
            run_id: None,
            dedupe_key: None,
            title: "event".to_string(),
            content: "content".to_string(),
            level: NoticeLevel::Info,
            project: None,
            cwd: None,
            command: None,
            exit_code: None,
            duration_ms: None,
            timestamp: Utc::now(),
            received_at: Utc::now(),
            raw_payload: None,
        }
    }

    #[test]
    fn stop_like_finish_notifies() {
        assert!(matches!(
            decide(&event(NoticeEventType::TaskFinish), 0),
            AggregationDecision::NotifyNow(_)
        ));
    }

    #[test]
    fn five_failures_notify() {
        assert!(matches!(
            decide(&event(NoticeEventType::TaskFail), 5),
            AggregationDecision::NotifyNow(_)
        ));
    }
}
