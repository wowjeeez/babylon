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

    pub(crate) async fn issue_ref_for(&self, msg_id: i64) -> Result<Option<String>> {
        let row: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT c.issue_prefix, i.number FROM issues i \
             JOIN channels c ON c.id=i.channel_id WHERE i.message_id=?",
        )
        .bind(msg_id)
        .fetch_optional(self.store.reader())
        .await?;
        Ok(row.and_then(|(p, n)| p.map(|p| format!("#{p}-{n}"))))
    }

    #[allow(clippy::items_after_statements)]
    pub async fn list_issues(
        &self,
        by: &Handle,
        channel: Option<&str>,
        assignee: Option<&str>,
        status: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<crate::dto::IssueInfo>> {
        if let Some(st) = status {
            crate::types::IssueStatus::parse(st)?;
        }
        let parent_id = match parent {
            Some(p) => Some(self.resolve_issue_ref(p).await?.0),
            None => None,
        };

        let mut sql = String::from(
            "SELECT i.message_id, i.number, c.issue_prefix, c.id, c.kind, \
             m.summary, i.status, i.parent_id, m.created_at \
             FROM issues i JOIN channels c ON c.id=i.channel_id \
             JOIN messages m ON m.id=i.message_id WHERE 1=1",
        );
        if channel.is_some() {
            sql.push_str(" AND c.name=?");
        }
        match status {
            Some(_) => sql.push_str(" AND i.status=?"),
            None => sql.push_str(" AND i.status<>'closed'"),
        }
        if parent_id.is_some() {
            sql.push_str(" AND i.parent_id=?");
        }
        if assignee.is_some() {
            sql.push_str(
                " AND EXISTS(SELECT 1 FROM message_mentions x \
                 WHERE x.message_id=i.message_id AND x.handle=?)",
            );
        }
        sql.push_str(" ORDER BY c.issue_prefix, i.number");

        type Row = (i64, i64, Option<String>, i64, String, String, String, Option<i64>, i64);
        let mut q = sqlx::query_as::<_, Row>(&sql);
        if let Some(ch) = channel {
            q = q.bind(ch.to_string());
        }
        if let Some(st) = status {
            q = q.bind(st.to_string());
        }
        if let Some(pid) = parent_id {
            q = q.bind(pid);
        }
        if let Some(a) = assignee {
            q = q.bind(a.to_string());
        }
        let rows = q.fetch_all(self.store.reader()).await?;

        let mut out = Vec::new();
        for (mid, number, prefix, cid, ckind, summary, st, parent_pid, ts) in rows {
            if !self.can_see(by, cid, &ckind).await? {
                continue;
            }
            let assignee = self.load_mentions(mid).await?;
            let open_children: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM issues WHERE parent_id=? AND status<>'closed'",
            )
            .bind(mid)
            .fetch_one(self.store.reader())
            .await?;
            let parent_ref = match parent_pid {
                Some(pp) => self.issue_ref_for(pp).await?,
                None => None,
            };
            out.push(crate::dto::IssueInfo {
                reference: format!("#{}-{number}", prefix.unwrap_or_default()),
                title: summary,
                status: st,
                assignee,
                parent_ref,
                open_children,
                ts,
            });
        }
        Ok(out)
    }

    #[allow(clippy::items_after_statements)]
    pub async fn get_issue(&self, by: &Handle, ref_str: &str) -> Result<crate::dto::IssueDetail> {
        let (msg_id, cid, number, prefix) = self.resolve_issue_ref(ref_str).await?;
        let (ckind, cname): (String, String) =
            sqlx::query_as("SELECT kind, name FROM channels WHERE id=?")
                .bind(cid)
                .fetch_one(self.store.reader())
                .await?;
        if !self.can_see(by, cid, &ckind).await? {
            return Err(Error::UnknownIssue(ref_str.to_string()));
        }
        let (summary, body, status, parent_pid, ts): (String, Option<String>, String, Option<i64>, i64) =
            sqlx::query_as(
                "SELECT m.summary, m.body, i.status, i.parent_id, m.created_at \
                 FROM issues i JOIN messages m ON m.id=i.message_id WHERE i.message_id=?",
            )
            .bind(msg_id)
            .fetch_one(self.store.reader())
            .await?;
        let assignee = self.load_mentions(msg_id).await?;
        let parent_ref = match parent_pid {
            Some(pp) => self.issue_ref_for(pp).await?,
            None => None,
        };
        let open_children: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM issues WHERE parent_id=? AND status<>'closed'",
        )
        .bind(msg_id)
        .fetch_one(self.store.reader())
        .await?;

        type ChildRow = (i64, i64, Option<String>, String, String, i64);
        let child_rows: Vec<ChildRow> = sqlx::query_as(
            "SELECT i.message_id, i.number, c.issue_prefix, m.summary, i.status, m.created_at \
             FROM issues i JOIN channels c ON c.id=i.channel_id \
             JOIN messages m ON m.id=i.message_id WHERE i.parent_id=? ORDER BY i.number",
        )
        .bind(msg_id)
        .fetch_all(self.store.reader())
        .await?;
        let mut children = Vec::new();
        for (cmid, cnum, cprefix, csum, cstatus, cts) in child_rows {
            let cassignee = self.load_mentions(cmid).await?;
            let coc: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM issues WHERE parent_id=? AND status<>'closed'",
            )
            .bind(cmid)
            .fetch_one(self.store.reader())
            .await?;
            children.push(crate::dto::IssueInfo {
                reference: format!("#{}-{cnum}", cprefix.unwrap_or_default()),
                title: csum,
                status: cstatus,
                assignee: cassignee,
                parent_ref: Some(format!("#{prefix}-{number}")),
                open_children: coc,
                ts: cts,
            });
        }

        Ok(crate::dto::IssueDetail {
            reference: format!("#{prefix}-{number}"),
            channel: cname,
            title: summary,
            body,
            status,
            assignee,
            parent_ref,
            open_children,
            ts,
            children,
        })
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

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        clippy::cognitive_complexity
    )]
    pub async fn update_issue(
        &self,
        by: &Handle,
        ref_str: &str,
        status: Option<&str>,
        assignee: Option<&str>,
        parent: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<(String, String)> {
        if status.is_none()
            && assignee.is_none()
            && parent.is_none()
            && title.is_none()
            && body.is_none()
        {
            return Err(Error::TooLarge("update needs at least one field".into()));
        }
        let (msg_id, cid, num, prefix) = self.resolve_issue_ref(ref_str).await?;
        let ckind: String = sqlx::query_scalar("SELECT kind FROM channels WHERE id=?")
            .bind(cid)
            .fetch_one(self.store.reader())
            .await?;
        if !self.can_see(by, cid, &ckind).await? {
            return Err(Error::UnknownIssue(ref_str.to_string()));
        }

        let parsed_status = match status {
            Some(st) => Some(crate::types::IssueStatus::parse(st)?),
            None => None,
        };

        let assignee_h = match assignee {
            Some(a) => {
                let ah = Handle::parse(a)?.into_string();
                let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM agents WHERE handle=?")
                    .bind(&ah)
                    .fetch_optional(self.store.reader())
                    .await?;
                if exists.is_none() {
                    return Err(Error::UnknownHandle(ah));
                }
                if ckind == "dm" {
                    let member: Option<i64> = sqlx::query_scalar(
                        "SELECT 1 FROM channel_members WHERE channel_id=? AND handle=?",
                    )
                    .bind(cid)
                    .bind(&ah)
                    .fetch_optional(self.store.reader())
                    .await?;
                    if member.is_none() {
                        return Err(Error::NotAMember(ah));
                    }
                }
                Some(ah)
            }
            None => None,
        };

        let new_parent = match parent {
            Some(p) => {
                let pid = self.resolve_issue_ref(p).await?.0;
                self.assert_no_cycle(msg_id, pid).await?;
                Some(pid)
            }
            None => None,
        };

        if let Some(t) = title {
            if t.is_empty() || t.len() > MAX_TITLE {
                return Err(Error::TooLarge("title".into()));
            }
        }
        if let Some(b) = body {
            if b.len() > MAX_BODY {
                return Err(Error::TooLarge("body".into()));
            }
        }

        if let Some(parsed) = parsed_status {
            if parsed == crate::types::IssueStatus::Closed {
                self.resolve(by, msg_id, None).await?;
            } else {
                self.assert_can_resolve(by, msg_id).await?;
                self.clear_resolved(msg_id).await?;
            }
            self.set_issue_status(msg_id, parsed.as_str()).await?;
        }

        if let Some(ah) = assignee_h {
            self.reassign_issue(msg_id, cid, &ah).await?;
            self.waiters.wake(&ah);
        }

        if let Some(pid) = new_parent {
            self.set_issue_parent(msg_id, Some(pid)).await?;
        }

        if let Some(t) = title {
            self.set_message_field(msg_id, "summary", t).await?;
        }
        if let Some(b) = body {
            self.set_message_field(msg_id, "body", b).await?;
        }

        let status_out: String = sqlx::query_scalar("SELECT status FROM issues WHERE message_id=?")
            .bind(msg_id)
            .fetch_one(self.store.reader())
            .await?;
        Ok((format!("#{prefix}-{num}"), status_out))
    }

    async fn set_issue_status(&self, msg_id: i64, status: &str) -> Result<()> {
        let s = status.to_string();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query("UPDATE issues SET status=? WHERE message_id=?")
                        .bind(s)
                        .bind(msg_id)
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .await
    }

    async fn clear_resolved(&self, msg_id: i64) -> Result<()> {
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query("UPDATE messages SET resolved_at=NULL, resolved_by=NULL WHERE id=?")
                        .bind(msg_id)
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .await
    }

    async fn reassign_issue(&self, msg_id: i64, cid: i64, assignee: &str) -> Result<()> {
        let a = assignee.to_string();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query("DELETE FROM message_mentions WHERE message_id=?")
                        .bind(msg_id)
                        .execute(&mut *c)
                        .await?;
                    sqlx::query("INSERT INTO message_mentions(message_id, handle) VALUES (?,?)")
                        .bind(msg_id)
                        .bind(&a)
                        .execute(&mut *c)
                        .await?;
                    sqlx::query(
                        "INSERT INTO subscriptions(handle, channel_id, last_acked_id, active) \
                         VALUES (?,?,?,1) ON CONFLICT(handle, channel_id) DO UPDATE SET active=1",
                    )
                    .bind(&a)
                    .bind(cid)
                    .bind(msg_id - 1)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await
    }

    async fn assert_no_cycle(&self, msg_id: i64, new_parent: i64) -> Result<()> {
        let mut visited = std::collections::HashSet::new();
        let mut cur = Some(new_parent);
        while let Some(p) = cur {
            if p == msg_id || !visited.insert(p) {
                return Err(Error::IssueCycle);
            }
            cur = sqlx::query_scalar::<_, Option<i64>>(
                "SELECT parent_id FROM issues WHERE message_id=?",
            )
            .bind(p)
            .fetch_optional(self.store.reader())
            .await?
            .flatten();
        }
        Ok(())
    }

    async fn set_issue_parent(&self, msg_id: i64, parent: Option<i64>) -> Result<()> {
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query("UPDATE issues SET parent_id=? WHERE message_id=?")
                        .bind(parent)
                        .bind(msg_id)
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .await
    }

    async fn set_message_field(&self, msg_id: i64, field: &str, value: &str) -> Result<()> {
        let sql = match field {
            "summary" => "UPDATE messages SET summary=? WHERE id=?",
            _ => "UPDATE messages SET body=? WHERE id=?",
        };
        let v = value.to_string();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query(sql).bind(v).bind(msg_id).execute(c).await?;
                    Ok(())
                })
            })
            .await
    }

    pub(crate) async fn channel_id(&self, name: &str) -> Result<i64> {
        sqlx::query_scalar("SELECT id FROM channels WHERE name=?")
            .bind(name)
            .fetch_optional(self.store.reader())
            .await?
            .ok_or_else(|| Error::UnknownChannel(name.to_string()))
    }

    pub async fn save_template(
        &self,
        by: &Handle,
        name: &str,
        body: &str,
        channel: Option<&str>,
        title: Option<&str>,
    ) -> Result<()> {
        if name.is_empty() || name.len() > 128 {
            return Err(Error::TooLarge("name".into()));
        }
        if body.is_empty() || body.len() > MAX_BODY {
            return Err(Error::TooLarge("body".into()));
        }
        let cid = match channel {
            Some(ch) => Some(self.channel_id(ch).await?),
            None => None,
        };
        let (n, b, t, who, now) = (
            name.to_string(),
            body.to_string(),
            title.map(str::to_string),
            by.as_str().to_string(),
            self.now_ms(),
        );
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let existing: Option<i64> = sqlx::query_scalar(
                        "SELECT 1 FROM templates WHERE IFNULL(channel_id,0)=IFNULL(?,0) AND name=?",
                    )
                    .bind(cid)
                    .bind(&n)
                    .fetch_optional(&mut *c)
                    .await?;
                    if existing.is_some() {
                        sqlx::query(
                            "UPDATE templates SET title=?, body=?, updated_by=?, updated_at=? \
                             WHERE IFNULL(channel_id,0)=IFNULL(?,0) AND name=?",
                        )
                        .bind(&t)
                        .bind(&b)
                        .bind(&who)
                        .bind(now)
                        .bind(cid)
                        .bind(&n)
                        .execute(c)
                        .await?;
                    } else {
                        sqlx::query(
                            "INSERT INTO templates(channel_id, name, title, body, updated_by, updated_at) \
                             VALUES (?,?,?,?,?,?)",
                        )
                        .bind(cid)
                        .bind(&n)
                        .bind(&t)
                        .bind(&b)
                        .bind(&who)
                        .bind(now)
                        .execute(c)
                        .await?;
                    }
                    Ok(())
                })
            })
            .await
    }

    #[allow(clippy::items_after_statements)]
    pub async fn list_templates(
        &self,
        _by: &Handle,
        channel: Option<&str>,
    ) -> Result<Vec<crate::dto::TemplateInfo>> {
        let cid = match channel {
            Some(ch) => Some(self.channel_id(ch).await?),
            None => None,
        };
        type Row = (Option<i64>, String, Option<String>, String, String, i64);
        let rows: Vec<Row> = match cid {
            Some(id) => {
                sqlx::query_as(
                    "SELECT channel_id, name, title, body, updated_by, updated_at FROM templates \
                     WHERE channel_id=? OR channel_id IS NULL \
                     ORDER BY channel_id IS NULL DESC, name",
                )
                .bind(id)
                .fetch_all(self.store.reader())
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT channel_id, name, title, body, updated_by, updated_at FROM templates \
                     WHERE channel_id IS NULL ORDER BY name",
                )
                .fetch_all(self.store.reader())
                .await?
            }
        };
        let mut out = Vec::new();
        for (ch_id, name, title, body, updated_by, updated_at) in rows {
            let scope = match ch_id {
                Some(id) => sqlx::query_scalar::<_, String>("SELECT name FROM channels WHERE id=?")
                    .bind(id)
                    .fetch_one(self.store.reader())
                    .await?,
                None => "global".to_string(),
            };
            out.push(crate::dto::TemplateInfo {
                name,
                title,
                body,
                scope,
                updated_by,
                updated_at,
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

    #[tokio::test]
    async fn close_sets_resolved_then_reopen_clears() {
        let (hub, code, dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        let i = hub
            .file_issue(&code, "pmv2", "x", None, Some("deploy"), None, None)
            .await
            .unwrap();
        let (_r, st) = hub
            .update_issue(&dep, "#pmv2-1", Some("closed"), None, None, None, None)
            .await
            .unwrap();
        assert_eq!(st, "closed");
        let ra: Option<i64> = sqlx::query_scalar("SELECT resolved_at FROM messages WHERE id=?")
            .bind(i.id)
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert!(ra.is_some());
        let (_r2, st2) = hub
            .update_issue(&dep, "#pmv2-1", Some("open"), None, None, None, None)
            .await
            .unwrap();
        assert_eq!(st2, "open");
        let ra2: Option<i64> = sqlx::query_scalar("SELECT resolved_at FROM messages WHERE id=?")
            .bind(i.id)
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert!(ra2.is_none());
    }

    #[tokio::test]
    async fn close_authz_rejects_outsider() {
        let (hub, code, _dep) = two_agents().await;
        let mallory = Handle::parse("mallory").unwrap();
        hub.mint_token(&mallory, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.file_issue(&code, "pmv2", "x", None, Some("deploy"), None, None)
            .await
            .unwrap();
        let err = hub
            .update_issue(&mallory, "#pmv2-1", Some("closed"), None, None, None, None)
            .await;
        assert!(matches!(
            err,
            Err(crate::error::Error::NotAuthorizedToResolve(_))
        ));
    }

    #[tokio::test]
    async fn reassign_replaces_mentions_and_reparent_blocks_cycle() {
        let (hub, code, _dep) = two_agents().await;
        let weather = Handle::parse("weather").unwrap();
        hub.mint_token(&weather, AgentKind::Agent).await.unwrap();
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        let a = hub
            .file_issue(&code, "pmv2", "parent", None, Some("deploy"), None, None)
            .await
            .unwrap();
        hub.file_issue(&code, "pmv2", "child", None, None, None, None)
            .await
            .unwrap();
        hub.update_issue(&code, "#pmv2-2", None, None, Some("#pmv2-1"), None, None)
            .await
            .unwrap();
        let err = hub
            .update_issue(&code, "#pmv2-1", None, None, Some("#pmv2-2"), None, None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::IssueCycle)));
        hub.update_issue(&code, "#pmv2-1", None, Some("weather"), None, None, None)
            .await
            .unwrap();
        let m = hub.load_mentions(a.id).await.unwrap();
        assert_eq!(m, vec!["weather".to_string()]);
    }

    #[tokio::test]
    async fn update_with_no_fields_errors() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.file_issue(&code, "pmv2", "x", None, None, None, None)
            .await
            .unwrap();
        assert!(hub
            .update_issue(&code, "#pmv2-1", None, None, None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn list_filters_and_counts_open_children() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.file_issue(&code, "pmv2", "parent", None, Some("deploy"), None, None)
            .await
            .unwrap();
        hub.file_issue(&code, "pmv2", "child", None, None, None, None)
            .await
            .unwrap();
        hub.update_issue(&code, "#pmv2-2", None, None, Some("#pmv2-1"), None, None)
            .await
            .unwrap();

        let open = hub.list_issues(&code, Some("pmv2"), None, None, None).await.unwrap();
        assert_eq!(open.len(), 2);
        let parent = open.iter().find(|i| i.reference == "#pmv2-1").unwrap();
        assert_eq!(parent.open_children, 1);

        let mine = hub.list_issues(&code, None, Some("deploy"), None, None).await.unwrap();
        assert!(mine.iter().any(|i| i.reference == "#pmv2-1"));

        hub.update_issue(&code, "#pmv2-2", Some("closed"), None, None, None, None)
            .await
            .unwrap();
        let still_open = hub.list_issues(&code, Some("pmv2"), None, None, None).await.unwrap();
        assert_eq!(still_open.len(), 1);
        let closed = hub.list_issues(&code, Some("pmv2"), None, Some("closed"), None).await.unwrap();
        assert_eq!(closed.len(), 1);
    }

    #[tokio::test]
    async fn get_issue_returns_body_and_children() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.file_issue(&code, "pmv2", "parent", Some("details"), None, None, None)
            .await
            .unwrap();
        hub.file_issue(&code, "pmv2", "child", None, None, None, None)
            .await
            .unwrap();
        hub.update_issue(&code, "#pmv2-2", None, None, Some("#pmv2-1"), None, None)
            .await
            .unwrap();
        let d = hub.get_issue(&code, "#pmv2-1").await.unwrap();
        assert_eq!(d.body.as_deref(), Some("details"));
        assert_eq!(d.children.len(), 1);
        assert_eq!(d.children[0].reference, "#pmv2-2");
        assert_eq!(d.children[0].parent_ref.as_deref(), Some("#pmv2-1"));
    }

    #[tokio::test]
    async fn dm_reassign_requires_membership() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        let mallory = Handle::parse("mallory").unwrap();
        for h in [&code, &deploy, &mallory] {
            hub.mint_token(h, AgentKind::Agent).await.unwrap();
        }
        hub.dm(&code, &deploy, "note", "hi", None, None).await.unwrap();
        let i = hub
            .file_issue(&code, "dm:code+deploy", "t", None, Some("deploy"), None, Some("dmx"))
            .await
            .unwrap();
        hub.update_issue(&code, "#dmx-1", None, Some("deploy"), None, None, None)
            .await
            .unwrap();
        let m = hub.load_mentions(i.id).await.unwrap();
        assert_eq!(m, vec!["deploy".to_string()]);
        let err = hub
            .update_issue(&code, "#dmx-1", None, Some("mallory"), None, None, None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::NotAMember(_))));
    }

    #[tokio::test]
    async fn dm_update_hides_existence_from_outsider() {
        let hub = Hub::new_in_memory().await.unwrap();
        let code = Handle::parse("code").unwrap();
        let deploy = Handle::parse("deploy").unwrap();
        let mallory = Handle::parse("mallory").unwrap();
        for h in [&code, &deploy, &mallory] {
            hub.mint_token(h, AgentKind::Agent).await.unwrap();
        }
        hub.dm(&code, &deploy, "note", "hi", None, None).await.unwrap();
        hub.file_issue(&code, "dm:code+deploy", "t", None, Some("deploy"), None, Some("dmx"))
            .await
            .unwrap();
        let err = hub
            .update_issue(&mallory, "#dmx-1", Some("in_progress"), None, None, None, None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::UnknownIssue(_))));
    }

    #[tokio::test]
    async fn templates_upsert_and_scope() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.save_template(&code, "bug", "## Repro\n", Some("pmv2"), Some("Bug"))
            .await
            .unwrap();
        hub.save_template(&code, "bug", "## Steps\n", Some("pmv2"), Some("Bug"))
            .await
            .unwrap();
        hub.save_template(&code, "task", "## Goal\n", None, None)
            .await
            .unwrap();

        let t = hub.list_templates(&code, Some("pmv2")).await.unwrap();
        assert_eq!(t.len(), 2);
        let bug = t.iter().find(|x| x.name == "bug").unwrap();
        assert_eq!(bug.body, "## Steps\n");
        assert_eq!(bug.scope, "pmv2");
        let g = t.iter().find(|x| x.name == "task").unwrap();
        assert_eq!(g.scope, "global");

        let only_global = hub.list_templates(&code, None).await.unwrap();
        assert_eq!(only_global.len(), 1);
    }

    #[tokio::test]
    async fn update_no_partial_commit_when_later_field_invalid() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        let i = hub
            .file_issue(&code, "pmv2", "x", None, Some("deploy"), None, None)
            .await
            .unwrap();

        let err = hub
            .update_issue(&code, "#pmv2-1", Some("closed"), None, None, Some(""), None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::TooLarge(_))));

        let ra: Option<i64> = sqlx::query_scalar("SELECT resolved_at FROM messages WHERE id=?")
            .bind(i.id)
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert!(ra.is_none());
        let st: String = sqlx::query_scalar("SELECT status FROM issues WHERE message_id=?")
            .bind(i.id)
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert_eq!(st, "open");

        let err2 = hub
            .update_issue(
                &code,
                "#pmv2-1",
                Some("in_progress"),
                Some("ghost_unknown"),
                None,
                None,
                None,
            )
            .await;
        assert!(matches!(err2, Err(crate::error::Error::UnknownHandle(_))));
        let st2: String = sqlx::query_scalar("SELECT status FROM issues WHERE message_id=?")
            .bind(i.id)
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert_eq!(st2, "open");
    }

    #[tokio::test]
    async fn reparent_cycle_walk_terminates_on_corrupt_data() {
        let (hub, code, _dep) = two_agents().await;
        hub.create_channel(&code, "pmv2", "t").await.unwrap();
        hub.file_issue(&code, "pmv2", "one", None, None, None, None)
            .await
            .unwrap();
        let b = hub
            .file_issue(&code, "pmv2", "two", None, None, None, None)
            .await
            .unwrap();
        let cc = hub
            .file_issue(&code, "pmv2", "three", None, None, None, None)
            .await
            .unwrap();

        let (bid, cid) = (b.id, cc.id);
        hub.store
            .with_writer(move |conn| {
                Box::pin(async move {
                    sqlx::query("UPDATE issues SET parent_id=? WHERE message_id=?")
                        .bind(cid)
                        .bind(bid)
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("UPDATE issues SET parent_id=? WHERE message_id=?")
                        .bind(bid)
                        .bind(cid)
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .await
            .unwrap();

        let err = hub
            .update_issue(&code, "#pmv2-1", None, None, Some("#pmv2-2"), None, None)
            .await;
        assert!(matches!(err, Err(crate::error::Error::IssueCycle)));
    }
}
