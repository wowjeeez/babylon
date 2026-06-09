use crate::dto::{CatchUp, MsgSummary, ResolveResult};
use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::Handle;
use std::collections::BTreeMap;
use std::time::Duration;

const MAX_WAIT_SECS: u64 = 50;
const MAX_WAITS_PER_HANDLE: u32 = 2;

struct WaitGuard<'a> {
    hub: &'a Hub,
    key: String,
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        if let Some(mut e) = self.hub.waits.get_mut(&self.key) {
            *e = e.saturating_sub(1);
        }
    }
}

impl Hub {
    pub async fn wait_for(
        &self,
        handle: &Handle,
        timeout_secs: u64,
        channels: Option<&[String]>,
        only_mentions: bool,
    ) -> Result<CatchUp> {
        {
            let mut e = self.waits.entry(handle.as_str().to_string()).or_insert(0);
            if *e >= MAX_WAITS_PER_HANDLE {
                return Err(Error::TooLarge("wait_for concurrency".into()));
            }
            *e += 1;
        }
        let _guard = WaitGuard {
            hub: self,
            key: handle.as_str().to_string(),
        };

        let budget = Duration::from_secs(timeout_secs.clamp(1, MAX_WAIT_SECS));
        let deadline = tokio::time::Instant::now() + budget;
        let notify = self.waiters.for_handle(handle.as_str());
        loop {
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let mut cu = self.catch_up(handle, channels, only_mentions, 50).await?;
            if !cu.messages.is_empty() {
                cu.woke = Some(true);
                return Ok(cu);
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(CatchUp {
                    messages: vec![],
                    next_cursors: BTreeMap::default(),
                    has_more: false,
                    woke: Some(false),
                });
            }
            tokio::select! {
                () = &mut notified => {}
                () = tokio::time::sleep(remaining) => {
                    return Ok(CatchUp {
                        messages: vec![],
                        next_cursors: BTreeMap::default(),
                        has_more: false,
                        woke: Some(false),
                    });
                }
            }
        }
    }

    pub async fn resolve(&self, by: &Handle, id: i64, note: Option<&str>) -> Result<ResolveResult> {
        #[allow(clippy::type_complexity)]
        let row: Option<(i64, String, String, Option<i64>, Option<String>, String)> =
            sqlx::query_as(
                "SELECT m.channel_id, m.kind, m.author, m.resolved_at, m.resolved_by, c.kind \
                 FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id=?",
            )
            .bind(id)
            .fetch_optional(self.store.reader())
            .await?;
        let (channel_id, kind, author, resolved_at, resolved_by, ckind) =
            row.ok_or(Error::BadResolveTarget(id))?;
        if kind != "question" && kind != "task" {
            return Err(Error::BadResolveTarget(id));
        }
        if ckind == "dm" && !self.can_see(by, channel_id, &ckind).await? {
            return Err(Error::NotAuthorizedToResolve(id));
        }
        if let (Some(at), Some(b)) = (resolved_at, resolved_by) {
            return Ok(ResolveResult {
                id,
                resolved_at: at,
                resolved_by: b,
            });
        }
        let is_operator: bool =
            sqlx::query_scalar::<_, i64>("SELECT 1 FROM agents WHERE handle=? AND kind='operator'")
                .bind(by.as_str())
                .fetch_optional(self.store.reader())
                .await?
                .is_some();
        let is_assignee: bool = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM message_mentions WHERE message_id=? AND handle=?",
        )
        .bind(id)
        .bind(by.as_str())
        .fetch_optional(self.store.reader())
        .await?
        .is_some();
        if by.as_str() != author && !is_assignee && !is_operator {
            return Err(Error::NotAuthorizedToResolve(id));
        }
        let (b, now, _note) = (
            by.as_str().to_string(),
            self.now_ms(),
            note.map(str::to_string),
        );
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE messages SET resolved_at=?, resolved_by=? \
                         WHERE id=? AND resolved_at IS NULL",
                    )
                    .bind(now)
                    .bind(&b)
                    .bind(id)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await?;
        let (at, b): (i64, String) =
            sqlx::query_as("SELECT resolved_at, resolved_by FROM messages WHERE id=?")
                .bind(id)
                .fetch_one(self.store.reader())
                .await?;
        Ok(ResolveResult {
            id,
            resolved_at: at,
            resolved_by: b,
        })
    }

    pub async fn open_questions(
        &self,
        handle: &Handle,
        mine_only: bool,
        channel: Option<&str>,
    ) -> Result<Vec<MsgSummary>> {
        self.open_of_kind("question", handle, mine_only, None, channel)
            .await
    }

    pub async fn open_tasks(
        &self,
        handle: &Handle,
        mine_only: bool,
        owner: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<MsgSummary>> {
        let mine = if owner.is_some() { false } else { mine_only };
        self.open_of_kind("task", handle, mine, owner, channel)
            .await
    }

    async fn open_of_kind(
        &self,
        kind: &str,
        handle: &Handle,
        mine_only: bool,
        owner: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<MsgSummary>> {
        let mut sql = String::from(
            "SELECT m.id, c.name AS ch, m.author, m.kind, m.reply_to, m.summary, \
             m.resolved_at, m.created_at \
             FROM messages m JOIN channels c ON c.id=m.channel_id \
             WHERE m.kind=? AND m.resolved_at IS NULL",
        );
        if channel.is_some() {
            sql.push_str(" AND c.name=?");
        }
        if owner.is_some() || mine_only {
            sql.push_str(
                " AND EXISTS(SELECT 1 FROM message_mentions x \
                 WHERE x.message_id=m.id AND x.handle=?)",
            );
        }
        sql.push_str(" ORDER BY m.id ASC");
        let mut q = sqlx::query_as::<_, crate::ops::reads::MsgRow>(&sql).bind(kind.to_string());
        if let Some(ch) = channel {
            q = q.bind(ch.to_string());
        }
        if let Some(o) = owner {
            q = q.bind(o.to_string());
        } else if mine_only {
            q = q.bind(handle.as_str().to_string());
        }
        let rows = q.fetch_all(self.store.reader()).await?;
        let mut out = Vec::new();
        for r in rows {
            let rid = r.id;
            let mut s = r.into_summary("");
            s.to = self.load_mentions(rid).await?;
            out.push(s);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};
    use std::time::Duration;

    #[tokio::test]
    async fn wait_for_caps_concurrent_per_handle() {
        let hub = Hub::new_in_memory().await.unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        let h1 = hub.clone();
        let b1 = bob.clone();
        let w1 = tokio::spawn(async move { h1.wait_for(&b1, 2, None, false).await });
        let h2 = hub.clone();
        let b2 = bob.clone();
        let w2 = tokio::spawn(async move { h2.wait_for(&b2, 2, None, false).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let third = hub.wait_for(&bob, 1, None, false).await;
        assert!(matches!(third, Err(crate::error::Error::TooLarge(_))));
        let _ = (w1.await, w2.await);
    }

    #[tokio::test]
    async fn post_wakes_only_subscribers() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let sub = Handle::parse("sub").unwrap();
        let other = Handle::parse("other").unwrap();
        for h in [&code, &sub, &other] {
            hub.mint_token(h, AgentKind::Agent).await.unwrap();
        }
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        hub.join_channel(&sub, "deploy").await.unwrap();
        let hs = hub.clone();
        let subbed = tokio::spawn(async move {
            hs.wait_for(&Handle::parse("sub").unwrap(), 3, None, false)
                .await
                .unwrap()
        });
        let ho = hub.clone();
        let outsider = tokio::spawn(async move {
            ho.wait_for(&Handle::parse("other").unwrap(), 1, None, false)
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        hub.post(&code, "deploy", "note", "ping", None, &[], None)
            .await
            .unwrap();
        assert_eq!(subbed.await.unwrap().woke, Some(true));
        assert_eq!(outsider.await.unwrap().woke, Some(false));
    }

    #[tokio::test]
    async fn wait_for_wakes_on_post_before_timeout() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        hub.join_channel(&bob, "deploy").await.unwrap();
        let h2 = hub.clone();
        let poster = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            h2.post(
                &Handle::parse("code").unwrap(),
                "deploy",
                "note",
                "ping",
                None,
                &[],
                None,
            )
            .await
            .unwrap();
        });
        let cu = hub.wait_for(&bob, 5, None, false).await.unwrap();
        assert_eq!(cu.woke, Some(true));
        assert_eq!(cu.messages.len(), 1);
        poster.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_times_out_empty() {
        let hub = Hub::new_in_memory().await.unwrap();
        let bob = Handle::parse("bob").unwrap();
        hub.mint_token(&bob, AgentKind::Agent).await.unwrap();
        let cu = hub.wait_for(&bob, 1, None, false).await.unwrap();
        assert_eq!(cu.woke, Some(false));
        assert!(cu.messages.is_empty());
    }

    #[tokio::test]
    async fn resolve_authz_idempotent_and_kind_checked() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        let mallory = Handle::parse("mallory").unwrap();
        for h in [&code, &deploy, &mallory] {
            hub.mint_token(h, AgentKind::Agent).await.unwrap();
        }
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        let task = hub
            .post(
                &code,
                "deploy",
                "task",
                "hand creds",
                None,
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            hub.resolve(&mallory, task, None).await,
            Err(crate::error::Error::NotAuthorizedToResolve(_))
        ));
        let r1 = hub.resolve(&deploy, task, None).await.unwrap();
        let r2 = hub.resolve(&code, task, None).await.unwrap();
        assert_eq!(r1.resolved_by, "deploy");
        assert_eq!(r2.resolved_by, "deploy");
        let note = hub
            .post(&code, "deploy", "note", "fyi", None, &[], None)
            .await
            .unwrap();
        assert!(matches!(
            hub.resolve(&code, note, None).await,
            Err(crate::error::Error::BadResolveTarget(_))
        ));
    }

    #[tokio::test]
    async fn answer_auto_resolves_question_and_open_lists_filter() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        for h in [&code, &deploy] {
            hub.mint_token(h, AgentKind::Agent).await.unwrap();
        }
        hub.create_channel(&code, "deploy", "t").await.unwrap();
        let q = hub
            .post(
                &code,
                "deploy",
                "question",
                "deploy now?",
                None,
                &["deploy".into()],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            hub.open_questions(&deploy, true, None).await.unwrap().len(),
            1
        );
        hub.post(&deploy, "deploy", "answer", "yes", None, &[], Some(q))
            .await
            .unwrap();
        assert_eq!(
            hub.open_questions(&deploy, true, None).await.unwrap().len(),
            0,
            "answer auto-resolves"
        );
        hub.post(
            &code,
            "deploy",
            "task",
            "do x",
            None,
            &["deploy".into()],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            hub.open_tasks(&deploy, true, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            hub.open_tasks(&code, true, None, None).await.unwrap().len(),
            0,
            "not assigned to code"
        );
        assert_eq!(
            hub.open_tasks(&code, true, Some("deploy"), None)
                .await
                .unwrap()
                .len(),
            1,
            "owner filter"
        );
    }
}
