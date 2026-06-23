use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unauthorized")]
    Unauthorized,
    #[error("token revoked")]
    TokenRevoked,
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
    #[error("unknown handle: {0}")]
    UnknownHandle(String),
    #[error("not a member of channel: {0}")]
    NotAMember(String),
    #[error("not subscribed to channel: {0}")]
    NotSubscribed(String),
    #[error("channel already exists: {0}")]
    ChannelExists(String),
    #[error("invalid name: {0}")]
    BadName(String),
    #[error("value too large: {0}")]
    TooLarge(String),
    #[error("reply target invalid: {0}")]
    BadReplyTarget(i64),
    #[error("resolve target invalid: {0}")]
    BadResolveTarget(i64),
    #[error("not authorized to resolve message {0}")]
    NotAuthorizedToResolve(i64),
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    #[error("handle already exists: {0}; use rotate_token to get a new token")]
    HandleExists(String),
    #[error("task needs at least one assignee mention")]
    TaskNeedsAssignee,
    #[error("invalid issue ref: {0}")]
    BadIssueRef(String),
    #[error("unknown issue: {0}")]
    UnknownIssue(String),
    #[error("issue prefix already in use: {0}")]
    DuplicatePrefix(String),
    #[error("invalid status: {0}")]
    BadStatus(String),
    #[error("issue cannot be its own ancestor")]
    IssueCycle,
    #[error("update needs at least one field")]
    EmptyUpdate,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
