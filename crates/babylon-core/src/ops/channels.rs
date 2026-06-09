use crate::dto::ChannelInfo;
use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::Handle;

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

    pub async fn archive_channel(&self, name: &str) -> Result<()> {
        let (nm, now) = (name.to_string(), self.now_ms());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
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
