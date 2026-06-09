use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct RegisterParams {
    pub role: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListChannelsParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateChannelParams {
    pub name: String,
    pub topic: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelNameParams {
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PostParams {
    pub channel: String,
    pub kind: String,
    pub summary: String,
    pub body: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
    pub reply_to: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CatchUpParams {
    pub channels: Option<Vec<String>>,
    #[serde(default)]
    pub only_mentions: bool,
    pub limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadParams {
    pub ids: Vec<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AckParams {
    pub channel: String,
    pub up_to_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitForParams {
    pub timeout_secs: Option<u64>,
    pub channels: Option<Vec<String>>,
    #[serde(default)]
    pub only_mentions: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct DmParams {
    pub to: String,
    pub kind: String,
    pub summary: String,
    pub body: Option<String>,
    pub reply_to: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ResolveParams {
    pub id: i64,
    pub note: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct OpenQuestionsParams {
    #[serde(default = "yes")]
    pub mine_only: bool,
    pub channel: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct OpenTasksParams {
    #[serde(default = "yes")]
    pub mine_only: bool,
    pub owner: Option<String>,
    pub channel: Option<String>,
}

const fn yes() -> bool {
    true
}
