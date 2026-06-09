use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::{Handle, MessageKind};
use sqlx::Connection;

impl Hub {
    pub async fn dm(
        &self,
        from: &Handle,
        to: &Handle,
        kind: &str,
        summary: &str,
        body: Option<&str>,
        reply_to: Option<i64>,
    ) -> Result<(i64, String)> {
        let name = crate::types::dm_channel_name(from, to);
        let (n2, a, b, now) = (
            name.clone(),
            from.as_str().to_string(),
            to.as_str().to_string(),
            self.now_ms(),
        );
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let mut tx = c.begin().await?;
                    let cid: i64 =
                        match sqlx::query_scalar::<_, i64>("SELECT id FROM channels WHERE name=?")
                            .bind(&n2)
                            .fetch_optional(&mut *tx)
                            .await?
                        {
                            Some(id) => id,
                            None => sqlx::query_scalar(
                                "INSERT INTO channels(name, topic, kind, created_by, created_at) \
                             VALUES (?,?, 'dm', ?, ?) RETURNING id",
                            )
                            .bind(&n2)
                            .bind(format!("dm {a}/{b}"))
                            .bind(&a)
                            .bind(now)
                            .fetch_one(&mut *tx)
                            .await?,
                        };
                    let max_id: i64 = sqlx::query_scalar(
                        "SELECT COALESCE(MAX(id),0) FROM messages WHERE channel_id=?",
                    )
                    .bind(cid)
                    .fetch_one(&mut *tx)
                    .await?;
                    for h in [&a, &b] {
                        sqlx::query(
                            "INSERT OR IGNORE INTO channel_members(channel_id, handle) \
                             VALUES (?,?)",
                        )
                        .bind(cid)
                        .bind(h)
                        .execute(&mut *tx)
                        .await?;
                        sqlx::query(
                            "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                             VALUES (?,?,?,1) \
                             ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                        )
                        .bind(h)
                        .bind(cid)
                        .bind(max_id)
                        .execute(&mut *tx)
                        .await?;
                    }
                    tx.commit().await?;
                    Ok(())
                })
            })
            .await?;
        let id = self
            .post(from, &name, kind, summary, body, &[], reply_to)
            .await?;
        Ok((id, name))
    }
}

const MAX_SUMMARY: usize = 1024;
const MAX_BODY: usize = 256 * 1024;

impl Hub {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub async fn post(
        &self,
        author: &Handle,
        channel: &str,
        kind: &str,
        summary: &str,
        body: Option<&str>,
        mentions: &[String],
        reply_to: Option<i64>,
    ) -> Result<i64> {
        if summary.is_empty() || summary.len() > MAX_SUMMARY {
            return Err(Error::TooLarge("summary".into()));
        }
        if body.is_some_and(|b| b.len() > MAX_BODY) {
            return Err(Error::TooLarge("body".into()));
        }
        let k = MessageKind::parse(kind)?;
        let mut mhandles = Vec::new();
        for m in mentions {
            mhandles.push(Handle::parse(m)?.into_string());
        }
        if k == MessageKind::Task && mhandles.is_empty() {
            return Err(Error::TaskNeedsAssignee);
        }

        let (author_s, chan_s, summary_s) = (
            author.as_str().to_string(),
            channel.to_string(),
            summary.to_string(),
        );
        let body_s = body.map(str::to_string);
        let kind_s = k.as_str().to_string();
        let now = self.now_ms();

        let new_id = self
            .store
            .with_writer(move |c| {
                Box::pin(async move {
                    let mut tx = c.begin().await?;
                    let chan: Option<(i64, String)> =
                        sqlx::query_as("SELECT id, kind FROM channels WHERE name=?")
                            .bind(&chan_s)
                            .fetch_optional(&mut *tx)
                            .await?;
                    let (cid, ckind) =
                        chan.ok_or_else(|| Error::UnknownChannel(chan_s.clone()))?;

                    if let Some(rt) = reply_to {
                        let rc: Option<i64> =
                            sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
                                .bind(rt)
                                .fetch_optional(&mut *tx)
                                .await?;
                        if rc != Some(cid) {
                            return Err(Error::BadReplyTarget(rt));
                        }
                    }

                    let mut kept: Vec<String> = Vec::new();
                    for m in &mhandles {
                        let exists: Option<i64> =
                            sqlx::query_scalar("SELECT 1 FROM agents WHERE handle=?")
                                .bind(m)
                                .fetch_optional(&mut *tx)
                                .await?;
                        if exists.is_none() {
                            return Err(Error::UnknownHandle(m.clone()));
                        }
                        if ckind == "dm" {
                            let member: Option<i64> = sqlx::query_scalar(
                                "SELECT 1 FROM channel_members WHERE channel_id=? AND handle=?",
                            )
                            .bind(cid)
                            .bind(m)
                            .fetch_optional(&mut *tx)
                            .await?;
                            if member.is_none() {
                                continue;
                            }
                        }
                        kept.push(m.clone());
                    }

                    let id: i64 = sqlx::query_scalar(
                        "INSERT INTO messages(channel_id, author, kind, summary, body, reply_to, created_at) \
                         VALUES (?,?,?,?,?,?,?) RETURNING id",
                    )
                    .bind(cid)
                    .bind(&author_s)
                    .bind(&kind_s)
                    .bind(&summary_s)
                    .bind(&body_s)
                    .bind(reply_to)
                    .bind(now)
                    .fetch_one(&mut *tx)
                    .await?;

                    sqlx::query(
                        "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                         VALUES (?,?,?,1) \
                         ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                    )
                    .bind(&author_s)
                    .bind(cid)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;

                    for m in &kept {
                        sqlx::query(
                            "INSERT INTO message_mentions(message_id, handle) VALUES (?,?)",
                        )
                        .bind(id)
                        .bind(m)
                        .execute(&mut *tx)
                        .await?;
                        sqlx::query(
                            "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                             VALUES (?,?,?,1) \
                             ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                        )
                        .bind(m)
                        .bind(cid)
                        .bind(id - 1)
                        .execute(&mut *tx)
                        .await?;
                    }

                    if k == MessageKind::Answer {
                        if let Some(rt) = reply_to {
                            sqlx::query(
                                "UPDATE messages SET resolved_at=?, resolved_by=? \
                                 WHERE id=? AND kind='question' AND resolved_at IS NULL",
                            )
                            .bind(now)
                            .bind(&author_s)
                            .bind(rt)
                            .execute(&mut *tx)
                            .await?;
                        }
                    }
                    tx.commit().await?;
                    Ok::<i64, Error>(id)
                })
            })
            .await?;

        let names: Vec<String> = sqlx::query_scalar(
            "SELECT s.handle FROM subscriptions s JOIN messages m ON m.id=? \
             WHERE s.channel_id=m.channel_id AND s.active=1",
        )
        .bind(new_id)
        .fetch_all(self.store.reader())
        .await
        .unwrap_or_default();
        for h in names {
            self.waiters.wake(&h);
        }
        Ok(new_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};
    use std::sync::Arc;

    async fn fixture() -> (Arc<Hub>, Handle, Handle) {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&deploy, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        (hub, code, deploy)
    }

    #[tokio::test]
    async fn mention_autosubscribes_new_handle_including_the_mention() {
        let (hub, code, deploy) = fixture().await;
        let id = hub
            .post(
                &code,
                "deploy",
                "question",
                "need creds?",
                None,
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        let cu = hub.catch_up(&deploy, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![id]
        );
    }

    #[tokio::test]
    async fn mention_does_not_clobber_existing_subscribers_cursor() {
        let (hub, code, deploy) = fixture().await;
        hub.join_channel(&deploy, "deploy").await.unwrap();
        let m1 = hub
            .post(&code, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        let m2 = hub
            .post(&code, "deploy", "note", "m2", None, &[], None)
            .await
            .unwrap();
        let m3 = hub
            .post(
                &code,
                "deploy",
                "note",
                "m3",
                None,
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        let cu = hub.catch_up(&deploy, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![m1, m2, m3]
        );
    }

    #[tokio::test]
    async fn unknown_mention_is_rejected() {
        let (hub, code, _deploy) = fixture().await;
        let err = hub
            .post(&code, "deploy", "note", "hi", None, &["ghost".into()], None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::UnknownHandle(_))));
    }

    #[tokio::test]
    async fn dm_creates_private_channel_and_only_members_see_it() {
        let (hub, code, deploy) = fixture().await;
        let (id, chname) = hub
            .dm(&code, &deploy, "note", "secret", Some("body"), None)
            .await
            .unwrap();
        assert_eq!(chname, "dm:code+deploy");
        let cu = hub.catch_up(&deploy, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![id]
        );
        let mallory = Handle::parse("mallory").unwrap();
        hub.mint_token(&mallory, AgentKind::Agent).await.unwrap();
        let cu = hub.catch_up(&mallory, None, false, 50).await.unwrap();
        assert!(cu.messages.is_empty());
        assert!(hub.read(&mallory, &[id]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn task_requires_assignee() {
        let (hub, code, _d) = fixture().await;
        assert!(matches!(
            hub.post(&code, "deploy", "task", "do x", None, &[], None)
                .await,
            Err(crate::error::Error::TaskNeedsAssignee)
        ));
        assert!(
            hub.post(
                &code,
                "deploy",
                "task",
                "do x",
                None,
                &["deploy".into()],
                None
            )
            .await
            .is_ok()
        );
    }
}
