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
}
