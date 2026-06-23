use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Handle(String);

impl Handle {
    pub fn parse(raw: &str) -> Result<Self> {
        let h = raw.trim().to_ascii_lowercase();
        let ok = (1..=64).contains(&h.len())
            && h.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if ok {
            Ok(Self(h))
        } else {
            Err(Error::BadName(raw.to_string()))
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Question,
    Answer,
    Decision,
    Status,
    Note,
    Task,
}

impl MessageKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Answer => "answer",
            Self::Decision => "decision",
            Self::Status => "status",
            Self::Note => "note",
            Self::Task => "task",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "question" => Self::Question,
            "answer" => Self::Answer,
            "decision" => Self::Decision,
            "status" => Self::Status,
            "note" => Self::Note,
            "task" => Self::Task,
            _ => return Err(Error::BadName(s.to_string())),
        })
    }
    #[must_use]
    pub const fn has_lifecycle(self) -> bool {
        matches!(self, Self::Question | Self::Task)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueStatus {
    Open,
    InProgress,
    Blocked,
    Closed,
}

impl IssueStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "open" => Self::Open,
            "in_progress" => Self::InProgress,
            "blocked" => Self::Blocked,
            "closed" => Self::Closed,
            _ => return Err(Error::BadStatus(s.to_string())),
        })
    }
}

#[must_use]
pub fn issue_ref(prefix: &str, number: i64) -> String {
    format!("#{prefix}-{number}")
}

pub fn parse_issue_ref(raw: &str) -> Result<(String, i64)> {
    let s = raw.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let (prefix, num) = s
        .rsplit_once('-')
        .ok_or_else(|| Error::BadIssueRef(raw.to_string()))?;
    let number: i64 = num
        .parse()
        .map_err(|_| Error::BadIssueRef(raw.to_string()))?;
    let prefix = prefix.to_ascii_lowercase();
    if prefix.is_empty() || number <= 0 {
        return Err(Error::BadIssueRef(raw.to_string()));
    }
    Ok((prefix, number))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Channel,
    Dm,
}
impl ChannelKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Dm => "dm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Agent,
    Operator,
}
impl AgentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Operator => "operator",
        }
    }
}

#[must_use]
pub fn dm_channel_name(a: &Handle, b: &Handle) -> String {
    let (lo, hi) = if a.as_str() <= b.as_str() {
        (a, b)
    } else {
        (b, a)
    };
    format!("dm:{}+{}", lo.as_str(), hi.as_str())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn handle_accepts_valid_lowercases_and_rejects_bad() {
        assert_eq!(Handle::parse("Code").unwrap().as_str(), "code");
        assert_eq!(Handle::parse("dep-loy_2").unwrap().as_str(), "dep-loy_2");
        assert!(Handle::parse("").is_err());
        assert!(Handle::parse("has space").is_err());
        assert!(Handle::parse("plus+sign").is_err());
        assert!(Handle::parse("colon:x").is_err());
        assert!(Handle::parse(&"x".repeat(65)).is_err());
    }

    #[test]
    fn kinds_roundtrip_str() {
        assert_eq!(MessageKind::Task.as_str(), "task");
        assert_eq!(
            MessageKind::parse("question").unwrap(),
            MessageKind::Question
        );
        assert!(MessageKind::parse("nope").is_err());
        assert!(MessageKind::Task.has_lifecycle());
        assert!(MessageKind::Question.has_lifecycle());
        assert!(!MessageKind::Note.has_lifecycle());
        assert_eq!(ChannelKind::Dm.as_str(), "dm");
        assert_eq!(AgentKind::Operator.as_str(), "operator");
    }

    #[test]
    fn dm_name_is_sorted_and_prefixed() {
        let a = Handle::parse("code").unwrap();
        let b = Handle::parse("deploy").unwrap();
        assert_eq!(dm_channel_name(&a, &b), "dm:code+deploy");
        assert_eq!(dm_channel_name(&b, &a), "dm:code+deploy");
    }

    #[test]
    fn issue_status_roundtrips() {
        assert_eq!(IssueStatus::InProgress.as_str(), "in_progress");
        assert_eq!(IssueStatus::parse("blocked").unwrap(), IssueStatus::Blocked);
        assert_eq!(IssueStatus::parse("closed").unwrap(), IssueStatus::Closed);
        assert!(IssueStatus::parse("nope").is_err());
    }

    #[test]
    fn issue_ref_parses_prefix_and_number() {
        assert_eq!(parse_issue_ref("#pmv2-12").unwrap(), ("pmv2".to_string(), 12));
        assert_eq!(parse_issue_ref("pmv2-12").unwrap(), ("pmv2".to_string(), 12));
        assert_eq!(parse_issue_ref("#poly-market-3").unwrap(), ("poly-market".to_string(), 3));
        assert!(parse_issue_ref("nodash").is_err());
        assert!(parse_issue_ref("#pmv2-x").is_err());
        assert!(parse_issue_ref("#pmv2-0").is_err());
        assert!(parse_issue_ref("#-5").is_err());
    }
}
