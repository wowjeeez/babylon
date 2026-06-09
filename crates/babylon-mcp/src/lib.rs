pub mod params;

use babylon_core::dto::{
    AgentInfo, CatchUp, ChannelInfo, MsgFull, MsgSummary, RegisterResult, ResolveResult,
};
use babylon_core::error::Error as CoreError;
use babylon_core::hub::Hub;
use babylon_core::types::Handle;
use params::{
    AckParams, CatchUpParams, ChannelNameParams, CreateChannelParams, DmParams, ListChannelsParams,
    OpenQuestionsParams, OpenTasksParams, PostParams, ReadParams, RegisterParams, ResolveParams,
    WaitForParams,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthedHandle(pub String);

#[derive(Clone, Serialize, JsonSchema)]
pub struct DmResult {
    pub id: i64,
    pub channel: String,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct ChannelList {
    pub channels: Vec<ChannelInfo>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct MessageList {
    pub messages: Vec<MsgFull>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct QuestionList {
    pub questions: Vec<MsgSummary>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct TaskList {
    pub tasks: Vec<MsgSummary>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct AgentList {
    pub agents: Vec<AgentInfo>,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct PostResult {
    pub id: i64,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct JoinResult {
    pub unread: i64,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct CreateChannelResult {
    pub created: bool,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct AckResult {
    pub acked: bool,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct OkResult {
    pub ok: bool,
}

#[derive(Clone)]
pub struct BabylonServer {
    hub: Arc<Hub>,
    tool_router: ToolRouter<BabylonServer>,
}

fn map_err(e: CoreError) -> McpError {
    match e {
        CoreError::Unauthorized | CoreError::TokenRevoked => {
            McpError::invalid_request(e.to_string(), None)
        }
        CoreError::Db(_) => McpError::internal_error(e.to_string(), None),
        other => McpError::invalid_params(other.to_string(), None),
    }
}

#[tool_router]
impl BabylonServer {
    #[must_use]
    pub fn new(hub: Arc<Hub>) -> Self {
        Self {
            hub,
            tool_router: Self::tool_router(),
        }
    }

    fn caller(ctx: &RequestContext<RoleServer>) -> Result<Handle, McpError> {
        let parts = ctx
            .extensions
            .get::<http::request::Parts>()
            .ok_or_else(|| McpError::internal_error("missing request parts", None))?;
        let a = parts
            .extensions
            .get::<AuthedHandle>()
            .ok_or_else(|| McpError::invalid_request("unauthenticated", None))?;
        Handle::parse(&a.0).map_err(|_| McpError::invalid_request("bad handle", None))
    }

    #[tool(
        description = "Register this agent (announce presence/role). Returns handle + unread counts."
    )]
    async fn register(
        &self,
        Parameters(p): Parameters<RegisterParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<RegisterResult>, McpError> {
        let h = Self::caller(&ctx)?;
        Ok(Json(self.hub.register(&h, p.role).await.map_err(map_err)?))
    }

    #[tool(description = "List channels with your unread counts.")]
    async fn list_channels(
        &self,
        Parameters(p): Parameters<ListChannelsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<ChannelList>, McpError> {
        let h = Self::caller(&ctx)?;
        let channels = self
            .hub
            .list_channels(&h, p.include_archived)
            .await
            .map_err(map_err)?;
        Ok(Json(ChannelList { channels }))
    }

    #[tool(description = "Create a channel (topic required).")]
    async fn create_channel(
        &self,
        Parameters(p): Parameters<CreateChannelParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<CreateChannelResult>, McpError> {
        let h = Self::caller(&ctx)?;
        let created = self
            .hub
            .create_channel(&h, &p.name, &p.topic)
            .await
            .map_err(map_err)?;
        Ok(Json(CreateChannelResult { created }))
    }

    #[tool(description = "Join (subscribe-from-now) a channel.")]
    async fn join_channel(
        &self,
        Parameters(p): Parameters<ChannelNameParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<JoinResult>, McpError> {
        let h = Self::caller(&ctx)?;
        self.hub.join_channel(&h, &p.name).await.map_err(map_err)?;
        Ok(Json(JoinResult { unread: 0 }))
    }

    #[tool(description = "Leave a channel (keeps your read cursor).")]
    async fn leave_channel(
        &self,
        Parameters(p): Parameters<ChannelNameParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<OkResult>, McpError> {
        let h = Self::caller(&ctx)?;
        self.hub.leave_channel(&h, &p.name).await.map_err(map_err)?;
        Ok(Json(OkResult { ok: true }))
    }

    #[tool(description = "Archive a channel (hide from list, keep history).")]
    async fn archive_channel(
        &self,
        Parameters(p): Parameters<ChannelNameParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<OkResult>, McpError> {
        let _ = Self::caller(&ctx)?;
        self.hub.archive_channel(&p.name).await.map_err(map_err)?;
        Ok(Json(OkResult { ok: true }))
    }

    #[tool(
        description = "Post a message. kind: question|answer|decision|status|note|task. task requires >=1 mention."
    )]
    async fn post(
        &self,
        Parameters(p): Parameters<PostParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<PostResult>, McpError> {
        let h = Self::caller(&ctx)?;
        let id = self
            .hub
            .post(
                &h,
                &p.channel,
                &p.kind,
                &p.summary,
                p.body.as_deref(),
                &p.mentions,
                p.reply_to,
            )
            .await
            .map_err(map_err)?;
        Ok(Json(PostResult { id }))
    }

    #[tool(
        description = "Catch up on unread (summaries). Non-advancing; ack to advance. Paginated per channel."
    )]
    async fn catch_up(
        &self,
        Parameters(p): Parameters<CatchUpParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<CatchUp>, McpError> {
        let h = Self::caller(&ctx)?;
        Ok(Json(
            self.hub
                .catch_up(
                    &h,
                    p.channels.as_deref(),
                    p.only_mentions,
                    p.limit.unwrap_or(50),
                )
                .await
                .map_err(map_err)?,
        ))
    }

    #[tool(description = "Read full message bodies by id.")]
    async fn read(
        &self,
        Parameters(p): Parameters<ReadParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<MessageList>, McpError> {
        let h = Self::caller(&ctx)?;
        let messages = self.hub.read(&h, &p.ids).await.map_err(map_err)?;
        Ok(Json(MessageList { messages }))
    }

    #[tool(description = "Acknowledge messages up to an id in a channel (advances your cursor).")]
    async fn ack(
        &self,
        Parameters(p): Parameters<AckParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<AckResult>, McpError> {
        let h = Self::caller(&ctx)?;
        self.hub
            .ack(&h, &p.channel, p.up_to_id)
            .await
            .map_err(map_err)?;
        Ok(Json(AckResult { acked: true }))
    }

    #[tool(description = "Long-poll until a relevant message arrives or timeout (<=50s).")]
    async fn wait_for(
        &self,
        Parameters(p): Parameters<WaitForParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<CatchUp>, McpError> {
        let h = Self::caller(&ctx)?;
        Ok(Json(
            self.hub
                .wait_for(
                    &h,
                    p.timeout_secs.unwrap_or(25),
                    p.channels.as_deref(),
                    p.only_mentions,
                )
                .await
                .map_err(map_err)?,
        ))
    }

    #[tool(description = "Direct message another agent (private 2-member channel).")]
    async fn dm(
        &self,
        Parameters(p): Parameters<DmParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<DmResult>, McpError> {
        let h = Self::caller(&ctx)?;
        let to = Handle::parse(&p.to).map_err(map_err)?;
        let (id, channel) = self
            .hub
            .dm(&h, &to, &p.kind, &p.summary, p.body.as_deref(), p.reply_to)
            .await
            .map_err(map_err)?;
        Ok(Json(DmResult { id, channel }))
    }

    #[tool(description = "Resolve a question/task (author, assignee, or operator).")]
    async fn resolve(
        &self,
        Parameters(p): Parameters<ResolveParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<ResolveResult>, McpError> {
        let h = Self::caller(&ctx)?;
        Ok(Json(
            self.hub
                .resolve(&h, p.id, p.note.as_deref())
                .await
                .map_err(map_err)?,
        ))
    }

    #[tool(description = "List open (unanswered) questions; mine_only = addressed to me.")]
    async fn open_questions(
        &self,
        Parameters(p): Parameters<OpenQuestionsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<QuestionList>, McpError> {
        let h = Self::caller(&ctx)?;
        let questions = self
            .hub
            .open_questions(&h, p.mine_only, p.channel.as_deref())
            .await
            .map_err(map_err)?;
        Ok(Json(QuestionList { questions }))
    }

    #[tool(description = "List open tasks; mine_only = assigned to me; owner filters by assignee.")]
    async fn open_tasks(
        &self,
        Parameters(p): Parameters<OpenTasksParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<TaskList>, McpError> {
        let h = Self::caller(&ctx)?;
        let tasks = self
            .hub
            .open_tasks(&h, p.mine_only, p.owner.as_deref(), p.channel.as_deref())
            .await
            .map_err(map_err)?;
        Ok(Json(TaskList { tasks }))
    }

    #[tool(description = "List registered agents + presence.")]
    async fn list_agents(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<AgentList>, McpError> {
        let _ = Self::caller(&ctx)?;
        let agents = self.hub.list_agents().await.map_err(map_err)?;
        Ok(Json(AgentList { agents }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BabylonServer {}
