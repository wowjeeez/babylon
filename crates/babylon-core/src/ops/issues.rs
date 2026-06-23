use crate::dto::FiledIssue;
use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::ops::messages::{insert_message_tx, wake_set};
use crate::types::Handle;
use sqlx::Connection;

const MAX_TITLE: usize = 1024;
const MAX_BODY: usize = 256 * 1024;

impl Hub {
    #[allow(clippy::similar_names)]
    pub(crate) async fn resolve_issue_ref(&self, raw: &str) -> Result<(i64, i64, i64, String)> {
        let (prefix, number) = crate::types::parse_issue_ref(raw)?;
        let row: Option<(i64, i64, i64)> = sqlx::query_as(
            "SELECT i.message_id, i.channel_id, i.number \
             FROM issues i JOIN channels c ON c.id=i.channel_id \
             WHERE c.issue_prefix=? AND i.number=?",
        )
        .bind(&prefix)
        .bind(number)
        .fetch_optional(self.store.reader())
        .await?;
        let (msg_id, cid, num) = row.ok_or_else(|| Error::UnknownIssue(raw.to_string()))?;
        Ok((msg_id, cid, num, prefix))
    }

    #[allow(clippy::too_many_arguments, clippy::single_match_else)]
    pub async fn file_issue(
        &self,
        by: &Handle,
        channel: &str,
        title: &str,
        body: Option<&str>,
        assignee: Option<&str>,
        parent: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<FiledIssue> {
        if title.is_empty() || title.len() > MAX_TITLE {
            return Err(Error::TooLarge("title".into()));
        }
        if body.is_some_and(|b| b.len() > MAX_BODY) {
            return Err(Error::TooLarge("body".into()));
        }
        let assignee_h = match assignee {
            Some(a) => Some(Handle::parse(a)?.into_string()),
            None => None,
        };
        let parent_msg_id = match parent {
            Some(p) => Some(self.resolve_issue_ref(p).await?.0),
            None => None,
        };

        let (by_s, chan_s, title_s) =
            (by.as_str().to_string(), channel.to_string(), title.to_string());
        let body_s = body.map(str::to_string);
        let prefix_in = prefix.map(str::to_ascii_lowercase);
        let now = self.now_ms();

        let (msg_id, number, eff_prefix, wake_names) = self
            .store
            .with_writer(move |c| {
                Box::pin(async move {
                    let mut tx = c.begin().await?;
                    let chan: Option<(i64, String, Option<String>)> = sqlx::query_as(
                        "SELECT id, kind, issue_prefix FROM channels WHERE name=?",
                    )
                    .bind(&chan_s)
                    .fetch_optional(&mut *tx)
                    .await?;
                    let (cid, ckind, existing_prefix) =
                        chan.ok_or_else(|| Error::UnknownChannel(chan_s.clone()))?;

                    let eff_prefix = match existing_prefix {
                        Some(p) => p,
                        None => {
                            let p = prefix_in.unwrap_or_else(|| chan_s.clone());
                            let clash: Option<i64> =
                                sqlx::query_scalar("SELECT 1 FROM channels WHERE issue_prefix=?")
                                    .bind(&p)
                                    .fetch_optional(&mut *tx)
                                    .await?;
                            if clash.is_some() {
                                return Err(Error::DuplicatePrefix(p));
                            }
                            sqlx::query("UPDATE channels SET issue_prefix=? WHERE id=?")
                                .bind(&p)
                                .bind(cid)
                                .execute(&mut *tx)
                                .await?;
                            p
                        }
                    };

                    let mentions: Vec<String> = assignee_h.into_iter().collect();
                    let msg_id = insert_message_tx(
                        &mut tx, cid, &ckind, &by_s, "task", &title_s,
                        body_s.as_deref(), None, &mentions, now,
                    )
                    .await?;

                    let number: i64 = sqlx::query_scalar(
                        "SELECT COALESCE(MAX(number),0)+1 FROM issues WHERE channel_id=?",
                    )
                    .bind(cid)
                    .fetch_one(&mut *tx)
                    .await?;

                    sqlx::query(
                        "INSERT INTO issues(message_id, channel_id, number, parent_id, status) \
                         VALUES (?,?,?,?, 'open')",
                    )
                    .bind(msg_id)
                    .bind(cid)
                    .bind(number)
                    .bind(parent_msg_id)
                    .execute(&mut *tx)
                    .await?;

                    tx.commit().await?;
                    let names = wake_set(c, cid).await?;
                    Ok::<(i64, i64, String, Vec<String>), Error>((msg_id, number, eff_prefix, names))
                })
            })
            .await?;

        for h in &wake_names {
            self.waiters.wake(h);
        }
        Ok(FiledIssue {
            reference: format!("#{eff_prefix}-{number}"),
            id: msg_id,
            number,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};

    async fn two_agents() -> (std::sync::Arc<Hub>, Handle, Handle) {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let dep = Handle::parse("deploy").unwrap();
        hub.mint_token(&code, AgentKind::Agent).await.unwrap();
        hub.mint_token(&dep, AgentKind::Agent).await.unwrap();
        (hub, code, dep)
    }

    #[tokio::test]
    async fn file_issue_assigns_ref_and_per_channel_number() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "polymarket v2").await.unwrap();
        let a = hub
            .file_issue(&code, "pmv2", "first", None, Some("deploy"), None, Some("pmv2"))
            .await
            .unwrap();
        let b = hub
            .file_issue(&code, "pmv2", "second", None, None, None, None)
            .await
            .unwrap();
        assert_eq!(a.reference, "#pmv2-1");
        assert_eq!(b.reference, "#pmv2-2");
        assert_eq!(b.number, 2);
    }

    #[tokio::test]
    async fn assigned_issue_reaches_assignee_via_catch_up() {
        let (hub, code, dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        let i = hub
            .file_issue(&code, "pmv2", "do x", None, Some("deploy"), None, None)
            .await
            .unwrap();
        let cu = hub.catch_up(&dep, None, false, 50).await.unwrap();
        assert!(cu.messages.iter().any(|m| m.id == i.id));
    }

    #[tokio::test]
    async fn channel_owned_issue_needs_no_assignee() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        let j = hub
            .file_issue(&code, "pmv2", "anyone", None, None, None, None)
            .await
            .unwrap();
        assert_eq!(j.reference, "#pmv2-1");
    }

    #[tokio::test]
    async fn duplicate_prefix_rejected() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "alpha", "t").await.unwrap();
        hub.create_channel(&code, "beta", "t").await.unwrap();
        hub.file_issue(&code, "alpha", "x", None, None, None, Some("shared"))
            .await
            .unwrap();
        let err = hub
            .file_issue(&code, "beta", "y", None, None, None, Some("shared"))
            .await;
        assert!(matches!(err, Err(crate::error::Error::DuplicatePrefix(_))));
    }
}
