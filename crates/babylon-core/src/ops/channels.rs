use crate::dto::{AdminChannelInfo, ChannelInfo, GlobalStats};
use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::Handle;

type AdminChannelRow = (String, String, String, Option<i64>, i64, i64, Option<i64>);

impl Hub {
    pub async fn create_channel(&self, by: &Handle, name: &str, topic: &str) -> Result<bool> {
        if name.starts_with("dm:") {
            return Err(Error::BadName(name.into()));
        }
        let nm = Handle::parse(name)
            .map_err(|_| Error::BadName(name.into()))?
            .into_string();
        if topic.is_empty() || topic.len() > 1024 {
            return Err(Error::TooLarge("topic".into()));
        }
        let (by_s, topic_s, now) = (by.as_str().to_string(), topic.to_string(), self.now_ms());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let exists: Option<i64> =
                        sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
                            .bind(&nm)
                            .fetch_optional(&mut *c)
                            .await?;
                    if exists.is_some() {
                        return Err(Error::ChannelExists(nm));
                    }
                    sqlx::query(
                        "INSERT INTO channels(name, topic, kind, created_by, created_at) \
                         VALUES (?,?,'channel',?,?)",
                    )
                    .bind(nm)
                    .bind(topic_s)
                    .bind(by_s)
                    .bind(now)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await?;
        Ok(true)
    }

    pub async fn join_channel(&self, handle: &Handle, name: &str) -> Result<()> {
        if name.starts_with("dm:") {
            return Err(Error::BadName(name.into()));
        }
        let (h, nm) = (handle.as_str().to_string(), name.to_string());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let cid: Option<i64> =
                        sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
                            .bind(&nm)
                            .fetch_optional(&mut *c)
                            .await?;
                    let cid = cid.ok_or_else(|| Error::UnknownChannel(nm))?;
                    let max_id: i64 = sqlx::query_scalar(
                        "SELECT COALESCE(MAX(id),0) FROM messages WHERE channel_id=?",
                    )
                    .bind(cid)
                    .fetch_one(&mut *c)
                    .await?;
                    sqlx::query(
                        "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                         VALUES (?,?,?,1) \
                         ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                    )
                    .bind(h)
                    .bind(cid)
                    .bind(max_id)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await
    }

    pub async fn leave_channel(&self, handle: &Handle, name: &str) -> Result<()> {
        let (h, nm) = (handle.as_str().to_string(), name.to_string());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE subscriptions SET active=0 \
                         WHERE handle=? AND channel_id=(SELECT id FROM channels WHERE name=?)",
                    )
                    .bind(h)
                    .bind(nm)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await
    }

    pub async fn archive_channel(&self, by: &Handle, name: &str) -> Result<()> {
        let (by_s, nm, now) = (by.as_str().to_string(), name.to_string(), self.now_ms());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let row: Option<(Option<String>,)> =
                        sqlx::query_as("SELECT created_by FROM channels WHERE name=?")
                            .bind(&nm)
                            .fetch_optional(&mut *c)
                            .await?;
                    let (created_by,) = row.ok_or_else(|| Error::UnknownChannel(nm.clone()))?;
                    let is_operator: bool = sqlx::query_scalar::<_, i64>(
                        "SELECT 1 FROM agents WHERE handle=? AND kind='operator'",
                    )
                    .bind(&by_s)
                    .fetch_optional(&mut *c)
                    .await?
                    .is_some();
                    let is_creator = created_by.as_deref() == Some(by_s.as_str());
                    if !is_creator && !is_operator {
                        return Err(Error::NotAuthorized(format!("{by_s} cannot archive {nm}")));
                    }
                    sqlx::query("UPDATE channels SET archived_at=? WHERE name=?")
                        .bind(now)
                        .bind(nm)
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .await
    }

    pub async fn list_channels(
        &self,
        handle: &Handle,
        include_archived: bool,
    ) -> Result<Vec<ChannelInfo>> {
        let rows: Vec<(i64, String, String, String, Option<i64>)> =
            sqlx::query_as("SELECT id, name, topic, kind, archived_at FROM channels ORDER BY name")
                .fetch_all(self.store.reader())
                .await?;
        let mut out = Vec::new();
        for (id, name, topic, kind, archived) in rows {
            if archived.is_some() && !include_archived {
                continue;
            }
            let sub: Option<(i64, i64)> = sqlx::query_as(
                "SELECT last_acked_id, active FROM subscriptions WHERE handle=? AND channel_id=?",
            )
            .bind(handle.as_str())
            .bind(id)
            .fetch_optional(self.store.reader())
            .await?;
            let (subscribed, unread) = match sub {
                Some((cur, 1)) => {
                    let n: i64 = sqlx::query_scalar(
                        "SELECT MIN(50, COUNT(*)) FROM messages WHERE channel_id=? AND id>?",
                    )
                    .bind(id)
                    .bind(cur)
                    .fetch_one(self.store.reader())
                    .await?;
                    (true, n)
                }
                _ => (false, 0),
            };
            out.push(ChannelInfo {
                name,
                topic,
                kind,
                subscribed,
                unread,
            });
        }
        Ok(out)
    }

    pub async fn admin_channels(&self) -> Result<Vec<AdminChannelInfo>> {
        let rows: Vec<AdminChannelRow> = sqlx::query_as(
            "SELECT c.name, c.topic, c.kind, c.archived_at, \
                 (SELECT COUNT(*) FROM subscriptions s WHERE s.channel_id=c.id AND s.active=1), \
                 (SELECT COUNT(*) FROM messages m WHERE m.channel_id=c.id), \
                 (SELECT MAX(created_at) FROM messages m WHERE m.channel_id=c.id) \
                 FROM channels c \
                 WHERE c.name NOT LIKE 'dm:%' \
                 ORDER BY c.name",
        )
        .fetch_all(self.store.reader())
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    name,
                    topic,
                    kind,
                    archived_at,
                    member_count,
                    message_count,
                    last_activity_ts,
                )| {
                    AdminChannelInfo {
                        name,
                        topic,
                        kind,
                        archived: archived_at.is_some(),
                        member_count,
                        message_count,
                        last_activity_ts,
                    }
                },
            )
            .collect())
    }

    pub async fn global_stats(&self) -> Result<GlobalStats> {
        let (agents, channels, messages): (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM agents), \
             (SELECT COUNT(*) FROM channels WHERE name NOT LIKE 'dm:%'), \
             (SELECT COUNT(*) FROM messages)",
        )
        .fetch_one(self.store.reader())
        .await?;
        Ok(GlobalStats {
            agents,
            channels,
            messages,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};

    async fn seeded() -> std::sync::Arc<Hub> {
        let hub = Hub::new_in_memory().await.unwrap();
        hub.mint_token(&Handle::parse("code").unwrap(), AgentKind::Agent)
            .await
            .unwrap();
        hub
    }

    #[tokio::test]
    async fn archive_channel_authz() {
        let hub = seeded().await;
        let creator = Handle::parse("code").unwrap();
        let other = Handle::parse("other").unwrap();
        let op = Handle::parse("operator").unwrap();
        hub.mint_token(&other, AgentKind::Agent).await.unwrap();
        hub.mint_token(&op, AgentKind::Operator).await.unwrap();
        hub.create_channel(&creator, "deploy", "t").await.unwrap();
        assert!(
            matches!(
                hub.archive_channel(&other, "deploy").await,
                Err(crate::error::Error::NotAuthorized(_))
            ),
            "non-creator non-operator must be rejected"
        );
        hub.archive_channel(&creator, "deploy").await.unwrap();
        hub.create_channel(&creator, "ops", "t").await.unwrap();
        hub.archive_channel(&op, "ops").await.unwrap();
    }

    #[tokio::test]
    async fn create_rejects_dm_prefix_and_dupes() {
        let hub = seeded().await;
        let by = Handle::parse("code").unwrap();
        hub.create_channel(&by, "deploy", "deploy talk")
            .await
            .unwrap();
        assert!(hub.create_channel(&by, "deploy", "x").await.is_err());
        assert!(hub.create_channel(&by, "dm:x+y", "x").await.is_err());
    }

    #[tokio::test]
    async fn join_channel_rejects_dm_prefix() {
        let hub = seeded().await;
        let (a, b) = (
            Handle::parse("code").unwrap(),
            Handle::parse("bob").unwrap(),
        );
        hub.mint_token(&b, AgentKind::Agent).await.unwrap();
        let mallory = Handle::parse("mallory").unwrap();
        hub.mint_token(&mallory, AgentKind::Agent).await.unwrap();
        hub.dm(&a, &b, "note", "hello", None, None).await.unwrap();
        let err = hub.join_channel(&mallory, "dm:bob+code").await;
        assert!(
            matches!(err, Err(crate::error::Error::BadName(_))),
            "join_channel must reject dm: prefix"
        );
    }

    #[tokio::test]
    async fn stray_dm_subscription_excluded_from_catch_up() {
        let hub = seeded().await;
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        let mallory = Handle::parse("mallory").unwrap();
        hub.mint_token(&mallory, AgentKind::Agent).await.unwrap();
        let (id, chname) = hub
            .dm(&code, &bob, "note", "secret", None, None)
            .await
            .unwrap();
        let cid: i64 = hub
            .store
            .with_writer(move |c| {
                Box::pin(async move {
                    let cid: i64 = sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
                        .bind(&chname)
                        .fetch_one(&mut *c)
                        .await?;
                    sqlx::query(
                        "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                         VALUES (?,?,0,1) \
                         ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                    )
                    .bind("mallory")
                    .bind(cid)
                    .execute(c)
                    .await?;
                    Ok::<i64, crate::error::Error>(cid)
                })
            })
            .await
            .unwrap();
        let _ = cid;
        let cu = hub.catch_up(&mallory, None, false, 50).await.unwrap();
        assert!(
            cu.messages.iter().all(|m| m.id != id),
            "stray dm subscription must not expose dm messages"
        );
    }

    #[tokio::test]
    async fn admin_channels_counts_excludes_dm_and_flags_archived() {
        let hub = seeded().await;
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "deploy talk")
            .await
            .unwrap();
        hub.create_channel(&code, "ops", "ops talk").await.unwrap();
        hub.join_channel(&bob, "deploy").await.unwrap();
        hub.post(&code, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        hub.post(&code, "deploy", "note", "m2", None, &[], None)
            .await
            .unwrap();
        hub.dm(&code, &bob, "note", "secret", None, None)
            .await
            .unwrap();
        hub.archive_channel(&code, "ops").await.unwrap();

        let admin = hub.admin_channels().await.unwrap();
        assert!(
            admin.iter().all(|c| !c.name.starts_with("dm:")),
            "dm channels must be excluded"
        );
        let deploy = admin.iter().find(|c| c.name == "deploy").unwrap();
        assert!(!deploy.archived);
        assert_eq!(deploy.member_count, 2);
        assert_eq!(deploy.message_count, 2);
        assert!(deploy.last_activity_ts.is_some());
        let ops = admin.iter().find(|c| c.name == "ops").unwrap();
        assert!(ops.archived);
        assert_eq!(ops.message_count, 0);
        assert_eq!(ops.last_activity_ts, None);
    }

    #[tokio::test]
    async fn global_stats_totals_exclude_dm_channels() {
        let hub = seeded().await;
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        hub.create_channel(&code, "ops", "t").await.unwrap();
        hub.post(&code, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        hub.dm(&code, &bob, "note", "secret", None, None)
            .await
            .unwrap();

        let stats = hub.global_stats().await.unwrap();
        assert_eq!(stats.agents, 2);
        assert_eq!(stats.channels, 2, "dm channel must not be counted");
        assert_eq!(stats.messages, 2);
    }

    #[tokio::test]
    async fn join_is_subscribe_from_now_then_leave_retains_cursor() {
        let hub = seeded().await;
        let by = Handle::parse("code").unwrap();
        hub.create_channel(&by, "deploy", "t").await.unwrap();
        hub.post(&by, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        let id2 = hub
            .post(&by, "deploy", "note", "m2", None, &[], None)
            .await
            .unwrap();
        let other = Handle::parse("weather").unwrap();
        hub.mint_token(&other, AgentKind::Agent).await.unwrap();
        hub.join_channel(&other, "deploy").await.unwrap();
        let cu = hub.catch_up(&other, None, false, 50).await.unwrap();
        assert!(cu.messages.is_empty(), "join is from-now");
        let id3 = hub
            .post(&by, "deploy", "note", "m3", None, &[], None)
            .await
            .unwrap();
        let cu = hub.catch_up(&other, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![id3]
        );
        assert!(id3 > id2);
        hub.ack(&other, "deploy", id3).await.unwrap();
        hub.leave_channel(&other, "deploy").await.unwrap();
        hub.join_channel(&other, "deploy").await.unwrap();
        let cu = hub.catch_up(&other, None, false, 50).await.unwrap();
        assert!(cu.messages.is_empty(), "rejoin retains cursor");
    }
}
