use crate::dto::{CatchUp, MsgFull, MsgSummary};
use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::Handle;
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};

const DEFAULT_LIMIT: i64 = 50;

pub(crate) const MSG_SELECT: &str = "SELECT m.id, c.name AS ch, m.author, m.kind, m.reply_to, m.summary, m.resolved_at, m.created_at \
     FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.channel_id=? AND m.id>?";

const MSG_SELECT_FULL: &str = "SELECT m.id, m.channel_id, c.kind AS channel_kind, c.name AS ch, m.author, m.kind, \
     m.reply_to, m.summary, m.body, m.resolved_at, m.created_at \
     FROM messages m JOIN channels c ON c.id=m.channel_id WHERE 1=1";

#[derive(sqlx::FromRow)]
pub(crate) struct MsgRow {
    pub(crate) id: i64,
    pub(crate) ch: String,
    pub(crate) author: String,
    pub(crate) kind: String,
    pub(crate) reply_to: Option<i64>,
    pub(crate) summary: String,
    pub(crate) resolved_at: Option<i64>,
    pub(crate) created_at: i64,
}

#[derive(sqlx::FromRow)]
pub(crate) struct MsgRowFull {
    pub(crate) id: i64,
    pub(crate) channel_id: i64,
    pub(crate) channel_kind: String,
    pub(crate) ch: String,
    pub(crate) author: String,
    pub(crate) kind: String,
    pub(crate) reply_to: Option<i64>,
    pub(crate) summary: String,
    pub(crate) body: Option<String>,
    pub(crate) resolved_at: Option<i64>,
    pub(crate) created_at: i64,
}

impl MsgRow {
    pub(crate) fn into_summary(self, _name: &str) -> MsgSummary {
        let open =
            (self.kind == "question" || self.kind == "task").then_some(self.resolved_at.is_none());
        MsgSummary {
            id: self.id,
            ch: self.ch,
            from: self.author,
            kind: self.kind,
            re: self.reply_to,
            to: Vec::new(),
            open,
            sum: self.summary,
            ts: self.created_at,
        }
    }
}

impl MsgRowFull {
    pub(crate) fn into_full(self) -> MsgFull {
        let open =
            (self.kind == "question" || self.kind == "task").then_some(self.resolved_at.is_none());
        MsgFull {
            id: self.id,
            ch: self.ch,
            from: self.author,
            kind: self.kind,
            re: self.reply_to,
            to: Vec::new(),
            open,
            sum: self.summary,
            ts: self.created_at,
            body: self.body,
        }
    }
}

async fn load_batch_mentions(ids: &[i64], pool: &SqlitePool) -> Result<HashMap<i64, Vec<String>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT message_id, handle FROM message_mentions WHERE message_id IN ({placeholders}) ORDER BY handle"
    );
    let mut q = sqlx::query_as::<_, (i64, String)>(&sql);
    for &id in ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await?;
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for (mid, handle) in rows {
        map.entry(mid).or_default().push(handle);
    }
    Ok(map)
}

async fn batch_fill_mentions(msgs: &mut [MsgSummary], pool: &SqlitePool) -> Result<()> {
    let ids: Vec<i64> = msgs.iter().map(|m| m.id).collect();
    let mut map = load_batch_mentions(&ids, pool).await?;
    for m in msgs.iter_mut() {
        m.to = map.remove(&m.id).unwrap_or_default();
    }
    Ok(())
}

async fn batch_fill_full_mentions(msgs: &mut [MsgFull], pool: &SqlitePool) -> Result<()> {
    let ids: Vec<i64> = msgs.iter().map(|m| m.id).collect();
    let mut map = load_batch_mentions(&ids, pool).await?;
    for m in msgs.iter_mut() {
        m.to = map.remove(&m.id).unwrap_or_default();
    }
    Ok(())
}

impl Hub {
    pub async fn catch_up(
        &self,
        handle: &Handle,
        channels: Option<&[String]>,
        only_mentions: bool,
        limit: i64,
    ) -> Result<CatchUp> {
        let limit = if limit <= 0 {
            DEFAULT_LIMIT
        } else {
            limit.min(200)
        };
        let subs: Vec<(i64, String, i64)> = match channels {
            Some(names) => {
                let mut v = Vec::new();
                for n in names {
                    let row: Option<(i64, i64)> = sqlx::query_as(
                        "SELECT c.id, s.last_acked_id FROM subscriptions s \
                         JOIN channels c ON c.id=s.channel_id \
                         WHERE s.handle=? AND s.active=1 AND c.name=? \
                         AND (c.kind <> 'dm' OR EXISTS (\
                           SELECT 1 FROM channel_members cm \
                           WHERE cm.channel_id = c.id AND cm.handle = ?\
                         ))",
                    )
                    .bind(handle.as_str())
                    .bind(n)
                    .bind(handle.as_str())
                    .fetch_optional(self.store.reader())
                    .await?;
                    if let Some((id, cur)) = row {
                        v.push((id, n.clone(), cur));
                    }
                }
                v
            }
            None => {
                sqlx::query_as(
                    "SELECT c.id, c.name, s.last_acked_id FROM subscriptions s \
                     JOIN channels c ON c.id=s.channel_id \
                     WHERE s.handle=? AND s.active=1 \
                     AND (c.kind <> 'dm' OR EXISTS (\
                       SELECT 1 FROM channel_members cm \
                       WHERE cm.channel_id = c.id AND cm.handle = ?\
                     ))",
                )
                .bind(handle.as_str())
                .bind(handle.as_str())
                .fetch_all(self.store.reader())
                .await?
            }
        };

        let mut messages = Vec::new();
        let mut next_cursors = BTreeMap::new();
        let mut has_more = false;
        for (cid, name, cur) in subs {
            let mut rows: Vec<MsgRow> = if only_mentions {
                sqlx::query_as::<_, MsgRow>(&format!(
                    "{MSG_SELECT} AND EXISTS(\
                     SELECT 1 FROM message_mentions x WHERE x.message_id=m.id AND x.handle=?\
                     ) ORDER BY m.id ASC LIMIT ?"
                ))
                .bind(cid)
                .bind(cur)
                .bind(handle.as_str())
                .bind(limit + 1)
                .fetch_all(self.store.reader())
                .await?
            } else {
                sqlx::query_as::<_, MsgRow>(&format!("{MSG_SELECT} ORDER BY m.id ASC LIMIT ?"))
                    .bind(cid)
                    .bind(cur)
                    .bind(limit + 1)
                    .fetch_all(self.store.reader())
                    .await?
            };
            let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
            if rows.len() > limit_usize {
                has_more = true;
                rows.truncate(limit_usize);
            }
            if let Some(last) = rows.last() {
                next_cursors.insert(name.clone(), last.id);
            }
            for r in rows {
                messages.push(r.into_summary(&name));
            }
        }
        messages.sort_by_key(|m| m.id);
        batch_fill_mentions(&mut messages, self.store.reader()).await?;
        Ok(CatchUp {
            messages,
            next_cursors,
            has_more,
            woke: None,
        })
    }

    pub async fn read(&self, handle: &Handle, ids: &[i64]) -> Result<Vec<MsgFull>> {
        let mut out = Vec::new();
        for &id in ids {
            let row: Option<MsgRowFull> =
                sqlx::query_as::<_, MsgRowFull>(&format!("{MSG_SELECT_FULL} AND m.id=?"))
                    .bind(id)
                    .fetch_optional(self.store.reader())
                    .await?;
            let Some(r) = row else {
                continue;
            };
            if !self.can_see(handle, r.channel_id, &r.channel_kind).await? {
                continue;
            }
            out.push(r.into_full());
        }
        batch_fill_full_mentions(&mut out, self.store.reader()).await?;
        Ok(out)
    }

    pub(crate) async fn can_see(
        &self,
        handle: &Handle,
        channel_id: i64,
        channel_kind: &str,
    ) -> Result<bool> {
        if channel_kind == "dm" {
            let m: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM channel_members WHERE channel_id=? AND handle=?")
                    .bind(channel_id)
                    .bind(handle.as_str())
                    .fetch_optional(self.store.reader())
                    .await?;
            Ok(m.is_some())
        } else {
            Ok(true)
        }
    }

    pub(crate) async fn load_mentions(&self, message_id: i64) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT handle FROM message_mentions WHERE message_id=? ORDER BY handle",
        )
        .bind(message_id)
        .fetch_all(self.store.reader())
        .await?)
    }

    pub async fn ack(&self, handle: &Handle, channel: &str, up_to_id: i64) -> Result<()> {
        let (h, nm) = (handle.as_str().to_string(), channel.to_string());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let cid: Option<i64> =
                        sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
                            .bind(&nm)
                            .fetch_optional(&mut *c)
                            .await?;
                    let cid = cid.ok_or_else(|| Error::UnknownChannel(nm.clone()))?;
                    let active: Option<i64> = sqlx::query_scalar(
                        "SELECT 1 FROM subscriptions WHERE handle=? AND channel_id=? AND active=1",
                    )
                    .bind(&h)
                    .bind(cid)
                    .fetch_optional(&mut *c)
                    .await?;
                    if active.is_none() {
                        return Err(Error::NotSubscribed(nm));
                    }
                    let chan_max: i64 = sqlx::query_scalar(
                        "SELECT COALESCE(MAX(id),0) FROM messages WHERE channel_id=?",
                    )
                    .bind(cid)
                    .fetch_one(&mut *c)
                    .await?;
                    let clamped = up_to_id.clamp(0, chan_max);
                    sqlx::query(
                        "UPDATE subscriptions SET last_acked_id = MAX(last_acked_id, ?) \
                         WHERE handle=? AND channel_id=?",
                    )
                    .bind(clamped)
                    .bind(h)
                    .bind(cid)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};

    #[tokio::test]
    async fn task_mentions_populated_in_catch_up_and_read() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&deploy, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        let id = hub
            .post(
                &code,
                "deploy",
                "task",
                "do the thing",
                Some("body"),
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        let cu = hub.catch_up(&deploy, None, false, 50).await.unwrap();
        let msg = cu.messages.iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            msg.to,
            vec!["deploy".to_string()],
            "catch_up must populate to"
        );
        let full = hub.read(&deploy, &[id]).await.unwrap();
        assert_eq!(
            full[0].to,
            vec!["deploy".to_string()],
            "read must populate to"
        );
    }

    #[tokio::test]
    async fn catch_up_is_non_advancing_and_paginates_per_channel() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        hub.join_channel(&bob, "deploy").await.unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            ids.push(
                hub.post(&code, "deploy", "note", &format!("m{i}"), None, &[], None)
                    .await
                    .unwrap(),
            );
        }
        let p1 = hub.catch_up(&bob, None, false, 2).await.unwrap();
        assert_eq!(p1.messages.len(), 2);
        assert!(p1.has_more);
        assert_eq!(p1.next_cursors["deploy"], ids[1]);
        let p1b = hub.catch_up(&bob, None, false, 2).await.unwrap();
        assert_eq!(p1b.messages.len(), 2, "non-advancing: same page until ack");
        hub.ack(&bob, "deploy", ids[1]).await.unwrap();
        let p2 = hub.catch_up(&bob, None, false, 2).await.unwrap();
        assert_eq!(
            p2.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![ids[2]]
        );
        assert!(!p2.has_more);
    }

    #[tokio::test]
    async fn read_returns_bodies_for_visible_messages() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        let id = hub
            .post(
                &code,
                "deploy",
                "note",
                "tldr",
                Some("full body"),
                &[],
                None,
            )
            .await
            .unwrap();
        let full = hub.read(&code, &[id]).await.unwrap();
        assert_eq!(full[0].body.as_deref(), Some("full body"));
        assert_eq!(full[0].sum, "tldr");
    }

    #[tokio::test]
    async fn ack_is_monotonic_and_clamped() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        hub.join_channel(&bob, "deploy").await.unwrap();
        let m1 = hub
            .post(&code, "deploy", "note", "m1", None, &[], None)
            .await
            .unwrap();
        let m2 = hub
            .post(&code, "deploy", "note", "m2", None, &[], None)
            .await
            .unwrap();
        hub.ack(&bob, "deploy", m2 + 10_000).await.unwrap();
        let m3 = hub
            .post(&code, "deploy", "note", "m3", None, &[], None)
            .await
            .unwrap();
        let cu = hub.catch_up(&bob, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![m3],
            "future ack must not eat m3"
        );
        hub.ack(&bob, "deploy", m1).await.unwrap();
        let cu = hub.catch_up(&bob, None, false, 50).await.unwrap();
        assert_eq!(
            cu.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![m3]
        );
        assert!(matches!(
            hub.ack(&bob, "nope", 1).await,
            Err(crate::error::Error::UnknownChannel(_))
        ));
    }
}
