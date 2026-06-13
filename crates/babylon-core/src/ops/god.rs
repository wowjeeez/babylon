use crate::dto::{ConversationInfo, MessageFull};
use crate::error::{Error, Result};
use crate::hub::Hub;
use std::collections::HashMap;

const MAX_HISTORY_LIMIT: i64 = 200;

#[derive(sqlx::FromRow)]
struct HistoryRow {
    id: i64,
    author: String,
    kind: String,
    summary: String,
    body: Option<String>,
    created_at: i64,
    reply_to: Option<i64>,
    resolved_at: Option<i64>,
    resolved_by: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    name: String,
    kind: String,
    topic: String,
    archived_at: Option<i64>,
    message_count: i64,
    last_activity_ts: Option<i64>,
}

impl Hub {
    pub async fn channel_history(
        &self,
        channel: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<MessageFull>> {
        let cid: Option<i64> = sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
            .bind(channel)
            .fetch_optional(self.store.reader())
            .await?;
        let cid = cid.ok_or_else(|| Error::UnknownChannel(channel.to_string()))?;

        let limit = if limit <= 0 {
            MAX_HISTORY_LIMIT
        } else {
            limit.min(MAX_HISTORY_LIMIT)
        };

        let mut sql = String::from(
            "SELECT id, author, kind, summary, body, created_at, reply_to, resolved_at, resolved_by \
             FROM messages WHERE channel_id=?",
        );
        if before.is_some() {
            sql.push_str(" AND id<?");
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");

        let mut q = sqlx::query_as::<_, HistoryRow>(&sql).bind(cid);
        if let Some(b) = before {
            q = q.bind(b);
        }
        let rows = q.bind(limit).fetch_all(self.store.reader()).await?;

        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let mut mentions = load_mentions_batch(&ids, self.store.reader()).await?;

        let mut out: Vec<MessageFull> = rows
            .into_iter()
            .map(|r| MessageFull {
                channel: channel.to_string(),
                to: mentions.remove(&r.id).unwrap_or_default(),
                id: r.id,
                from: r.author,
                kind: r.kind,
                summary: r.summary,
                body: r.body,
                ts: r.created_at,
                reply_to: r.reply_to,
                resolved_at: r.resolved_at,
                resolved_by: r.resolved_by,
            })
            .collect();
        out.reverse();
        Ok(out)
    }

    pub async fn conversations(&self) -> Result<Vec<ConversationInfo>> {
        let rows: Vec<ConversationRow> = sqlx::query_as(
            "SELECT c.name, c.kind, c.topic, c.archived_at, \
                 (SELECT COUNT(*) FROM messages m WHERE m.channel_id=c.id) AS message_count, \
                 (SELECT MAX(created_at) FROM messages m WHERE m.channel_id=c.id) AS last_activity_ts \
                 FROM channels c \
                 ORDER BY last_activity_ts DESC, c.name ASC",
        )
        .fetch_all(self.store.reader())
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let members: Vec<String> = sqlx::query_scalar(
                "SELECT s.handle FROM subscriptions s \
                 JOIN channels c ON c.id=s.channel_id \
                 WHERE c.name=? AND s.active=1 ORDER BY s.handle",
            )
            .bind(&r.name)
            .fetch_all(self.store.reader())
            .await?;
            out.push(ConversationInfo {
                name: r.name,
                kind: r.kind,
                topic: r.topic,
                members,
                message_count: r.message_count,
                last_activity_ts: r.last_activity_ts,
                archived: r.archived_at.is_some(),
            });
        }
        Ok(out)
    }
}

async fn load_mentions_batch(
    ids: &[i64],
    pool: &sqlx::SqlitePool,
) -> Result<HashMap<i64, Vec<String>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT message_id, handle FROM message_mentions \
         WHERE message_id IN ({placeholders}) ORDER BY handle"
    );
    let mut q = sqlx::query_as::<_, (i64, String)>(&sql);
    for &id in ids {
        q = q.bind(id);
    }
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for (mid, handle) in q.fetch_all(pool).await? {
        map.entry(mid).or_default().push(handle);
    }
    Ok(map)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};
    use std::sync::Arc;

    async fn seeded() -> (Arc<Hub>, Handle, Handle) {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&deploy, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "deploy talk")
            .await
            .unwrap();
        (hub, code, deploy)
    }

    #[tokio::test]
    async fn channel_history_orders_oldest_to_newest_with_body_and_mentions() {
        let (hub, code, deploy) = seeded().await;
        let m1 = hub
            .post(&code, "deploy", "note", "m1", Some("body1"), &[], None)
            .await
            .unwrap();
        let q = hub
            .post(
                &code,
                "deploy",
                "question",
                "need creds?",
                Some("qbody"),
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        hub.post(&deploy, "deploy", "answer", "yes", None, &[], Some(q))
            .await
            .unwrap();

        let hist = hub.channel_history("deploy", None, 50).await.unwrap();
        let ids: Vec<i64> = hist.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![m1, q, q + 1], "oldest to newest");

        let first = &hist[0];
        assert_eq!(first.channel, "deploy");
        assert_eq!(first.body.as_deref(), Some("body1"));
        assert_eq!(first.from, "code");

        let question = hist.iter().find(|m| m.id == q).unwrap();
        assert_eq!(question.to, vec!["deploy".to_string()]);
        assert!(question.resolved_at.is_some(), "question auto-resolved");
        assert_eq!(question.resolved_by.as_deref(), Some("deploy"));
    }

    #[tokio::test]
    async fn channel_history_before_paginates() {
        let (hub, code, _deploy) = seeded().await;
        let mut ids = Vec::new();
        for i in 0..5 {
            ids.push(
                hub.post(&code, "deploy", "note", &format!("m{i}"), None, &[], None)
                    .await
                    .unwrap(),
            );
        }
        let page = hub
            .channel_history("deploy", Some(ids[2]), 50)
            .await
            .unwrap();
        let got: Vec<i64> = page.iter().map(|m| m.id).collect();
        assert_eq!(got, vec![ids[0], ids[1]], "only ids strictly < before");
    }

    #[tokio::test]
    async fn channel_history_god_view_reads_dm_without_membership() {
        let (hub, code, deploy) = seeded().await;
        let (id, chname) = hub
            .dm(&code, &deploy, "note", "secret", Some("dmbody"), None)
            .await
            .unwrap();
        assert_eq!(chname, "dm:code+deploy");
        let hist = hub.channel_history(&chname, None, 50).await.unwrap();
        assert_eq!(hist.iter().map(|m| m.id).collect::<Vec<_>>(), vec![id]);
        assert_eq!(hist[0].body.as_deref(), Some("dmbody"));
    }

    #[tokio::test]
    async fn channel_history_unknown_channel_errors() {
        let (hub, _code, _deploy) = seeded().await;
        assert!(matches!(
            hub.channel_history("nope", None, 50).await,
            Err(crate::error::Error::UnknownChannel(_))
        ));
    }

    #[tokio::test]
    async fn conversations_includes_dm_with_members_counts_and_archived() {
        let (hub, code, deploy) = seeded().await;
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.join_channel(&bob, "deploy").await.unwrap();
        hub.post(&code, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        hub.post(&code, "deploy", "note", "m2", None, &[], None)
            .await
            .unwrap();
        hub.create_channel(&code, "ops", "ops talk").await.unwrap();
        hub.archive_channel(&code, "ops").await.unwrap();
        hub.dm(&code, &deploy, "note", "secret", None, None)
            .await
            .unwrap();

        let convs = hub.conversations().await.unwrap();

        let dm = convs.iter().find(|c| c.name == "dm:code+deploy").unwrap();
        assert_eq!(dm.kind, "dm");
        let mut dm_members = dm.members.clone();
        dm_members.sort();
        assert_eq!(dm_members, vec!["code".to_string(), "deploy".to_string()]);
        assert_eq!(dm.message_count, 1);
        assert!(dm.last_activity_ts.is_some());

        let deploy_ch = convs.iter().find(|c| c.name == "deploy").unwrap();
        assert_eq!(deploy_ch.message_count, 2);
        let mut members = deploy_ch.members.clone();
        members.sort();
        assert_eq!(members, vec!["bob".to_string(), "code".to_string()]);
        assert!(!deploy_ch.archived);

        let ops = convs.iter().find(|c| c.name == "ops").unwrap();
        assert!(ops.archived);
        assert_eq!(ops.message_count, 0);
        assert_eq!(ops.last_activity_ts, None);
    }
}
