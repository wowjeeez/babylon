use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct MsgSummary {
    pub id: i64,
    pub ch: String,
    pub from: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub re: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    pub sum: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MsgFull {
    pub id: i64,
    pub ch: String,
    pub from: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub re: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    pub sum: String,
    pub ts: i64,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatchUp {
    pub messages: Vec<MsgSummary>,
    pub next_cursors: BTreeMap<String, i64>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub woke: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterResult {
    pub handle: String,
    pub unread: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelInfo {
    pub name: String,
    pub topic: String,
    pub kind: String,
    pub subscribed: bool,
    pub unread: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentInfo {
    pub handle: String,
    pub role: Option<String>,
    pub kind: String,
    pub last_seen: i64,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveResult {
    pub id: i64,
    pub resolved_at: i64,
    pub resolved_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdminChannelInfo {
    pub name: String,
    pub topic: String,
    pub kind: String,
    pub archived: bool,
    pub member_count: i64,
    pub message_count: i64,
    pub last_activity_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GlobalStats {
    pub agents: i64,
    pub channels: i64,
    pub messages: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageFull {
    pub id: i64,
    pub channel: String,
    pub from: String,
    pub kind: String,
    pub summary: String,
    pub body: Option<String>,
    pub ts: i64,
    pub reply_to: Option<i64>,
    pub resolved_at: Option<i64>,
    pub resolved_by: Option<String>,
    pub to: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversationInfo {
    pub name: String,
    pub kind: String,
    pub topic: String,
    pub members: Vec<String>,
    pub message_count: i64,
    pub last_activity_ts: Option<i64>,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FiledIssue {
    #[serde(rename = "ref")]
    pub reference: String,
    pub id: i64,
    pub number: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IssueInfo {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignee: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    pub open_children: i64,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IssueDetail {
    #[serde(rename = "ref")]
    pub reference: String,
    pub channel: String,
    pub title: String,
    pub body: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignee: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    pub open_children: i64,
    pub ts: i64,
    pub children: Vec<IssueInfo>,
}
