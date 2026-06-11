use crate::dto::{AgentInfo, RegisterResult};
use crate::error::{Error, Result};
use crate::hub::{Hub, PRESENCE_WINDOW_SECS};
use crate::token::{generate_token, hash_token, verify};
use crate::types::{AgentKind, Handle};
use rand::RngCore;
use std::collections::BTreeMap;

impl Hub {
    pub async fn mint_token(&self, handle: &Handle, kind: AgentKind) -> Result<String> {
        let token = generate_token();
        let hash = hash_token(&token);
        let (h, k, now) = (handle.as_str().to_string(), kind.as_str(), self.now_ms());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let existing: Option<i64> =
                        sqlx::query_scalar("SELECT 1 FROM agents WHERE handle=?")
                            .bind(&h)
                            .fetch_optional(&mut *c)
                            .await?;
                    if existing.is_some() {
                        return Err(Error::HandleExists(h));
                    }
                    sqlx::query(
                        "INSERT INTO agents(handle, kind, token_hash, created_at) VALUES (?,?,?,?)",
                    )
                    .bind(h)
                    .bind(k)
                    .bind(hash)
                    .bind(now)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await?;
        Ok(token)
    }

    pub async fn rotate_token(&self, handle: &Handle) -> Result<String> {
        let token = generate_token();
        let hash = hash_token(&token);
        let h = handle.as_str().to_string();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let n = sqlx::query(
                        "UPDATE agents SET token_hash=?, token_revoked=NULL WHERE handle=?",
                    )
                    .bind(hash)
                    .bind(&h)
                    .execute(c)
                    .await?
                    .rows_affected();
                    if n == 0 {
                        return Err(Error::UnknownHandle(h));
                    }
                    Ok(())
                })
            })
            .await?;
        Ok(token)
    }

    pub async fn revoke_token(&self, handle: &Handle) -> Result<()> {
        let (h, now) = (handle.as_str().to_string(), self.now_ms());
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    let n = sqlx::query("UPDATE agents SET token_revoked=? WHERE handle=?")
                        .bind(now)
                        .bind(&h)
                        .execute(c)
                        .await?
                        .rows_affected();
                    if n == 0 {
                        return Err(Error::UnknownHandle(h));
                    }
                    Ok(())
                })
            })
            .await
    }

    pub async fn ensure_operator(&self) -> Result<()> {
        let mut hash = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut hash);
        let now = self.now_ms();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT OR IGNORE INTO agents(handle, kind, token_hash, created_at, last_seen_at) \
                         VALUES ('operator','operator',?,?,0)",
                    )
                    .bind(hash)
                    .bind(now)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await
    }

    pub async fn resolve_token(&self, token: &str) -> Result<Handle> {
        let row: Option<(String, Vec<u8>, Option<i64>)> = sqlx::query_as(
            "SELECT handle, token_hash, token_revoked FROM agents WHERE token_hash = ?",
        )
        .bind(hash_token(token))
        .fetch_optional(self.store.reader())
        .await?;
        let (handle, hash, revoked) = row.ok_or(Error::Unauthorized)?;
        if !verify(token, &hash) {
            return Err(Error::Unauthorized);
        }
        if revoked.is_some() {
            return Err(Error::TokenRevoked);
        }
        Handle::parse(&handle)
    }

    pub async fn register(&self, handle: &Handle, role: Option<String>) -> Result<RegisterResult> {
        let (h, now) = (handle.as_str().to_string(), self.now_ms());
        let role2 = role.clone();
        self.store
            .with_writer(move |c| {
                Box::pin(async move {
                    sqlx::query(
                        "UPDATE agents SET role=COALESCE(?, role), last_seen_at=? WHERE handle=?",
                    )
                    .bind(role2)
                    .bind(now)
                    .bind(h)
                    .execute(c)
                    .await?;
                    Ok(())
                })
            })
            .await?;
        self.presence.set_live(handle.as_str(), true);
        self.presence.touch(handle.as_str());
        let unread = self.unread_counts(handle).await?;
        Ok(RegisterResult {
            handle: handle.as_str().to_string(),
            unread,
        })
    }

    pub async fn unread_counts(&self, handle: &Handle) -> Result<BTreeMap<String, i64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT c.name, MIN(50, (SELECT COUNT(*) FROM messages m \
             WHERE m.channel_id=c.id AND m.id > s.last_acked_id)) \
             FROM subscriptions s JOIN channels c ON c.id=s.channel_id \
             WHERE s.handle=? AND s.active=1",
        )
        .bind(handle.as_str())
        .fetch_all(self.store.reader())
        .await?;
        Ok(rows.into_iter().filter(|(_, n)| *n > 0).collect())
    }

    pub async fn agent_exists(&self, handle: &str) -> Result<bool> {
        let row: Option<i64> = sqlx::query_scalar("SELECT 1 FROM agents WHERE handle=?")
            .bind(handle)
            .fetch_optional(self.store.reader())
            .await?;
        Ok(row.is_some())
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let rows: Vec<(String, Option<String>, String, i64)> =
            sqlx::query_as("SELECT handle, role, kind, last_seen_at FROM agents ORDER BY handle")
                .fetch_all(self.store.reader())
                .await?;
        Ok(rows
            .into_iter()
            .map(|(handle, role, kind, last_seen)| {
                let online = self.presence.online(&handle, PRESENCE_WINDOW_SECS);
                AgentInfo {
                    handle,
                    role,
                    kind,
                    last_seen,
                    online,
                }
            })
            .collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::hub::Hub;
    use crate::types::{AgentKind, Handle};

    #[tokio::test]
    async fn mint_existing_handle_returns_handle_exists() {
        let hub = Hub::new_in_memory().await.unwrap();
        let h = Handle::parse("code").unwrap();
        let original_token = hub.mint_token(&h, AgentKind::Agent).await.unwrap();
        let err = hub.mint_token(&h, AgentKind::Agent).await;
        assert!(
            matches!(err, Err(crate::error::Error::HandleExists(_))),
            "minting existing handle must return HandleExists, got {err:?}"
        );
        assert_eq!(
            hub.resolve_token(&original_token).await.unwrap().as_str(),
            "code",
            "original token must still be valid after failed re-mint"
        );
    }

    #[tokio::test]
    async fn mint_then_resolve_then_revoke() {
        let hub = Hub::new_in_memory().await.unwrap();
        let h = Handle::parse("code").unwrap();
        let token = hub.mint_token(&h, AgentKind::Agent).await.unwrap();
        assert_eq!(hub.resolve_token(&token).await.unwrap().as_str(), "code");
        hub.revoke_token(&h).await.unwrap();
        assert!(matches!(
            hub.resolve_token(&token).await,
            Err(crate::error::Error::TokenRevoked)
        ));
    }

    #[tokio::test]
    async fn rotate_invalidates_old() {
        let hub = Hub::new_in_memory().await.unwrap();
        let h = Handle::parse("code").unwrap();
        let t1 = hub.mint_token(&h, AgentKind::Agent).await.unwrap();
        let t2 = hub.rotate_token(&h).await.unwrap();
        assert!(hub.resolve_token(&t1).await.is_err());
        assert_eq!(hub.resolve_token(&t2).await.unwrap().as_str(), "code");
    }

    #[tokio::test]
    async fn ensure_operator_is_idempotent_and_unusable() {
        let hub = Hub::new_in_memory().await.unwrap();
        hub.ensure_operator().await.unwrap();
        let (cnt, kind, hash1): (i64, String, Vec<u8>) = sqlx::query_as(
            "SELECT COUNT(*), MAX(kind), MAX(token_hash) FROM agents WHERE handle='operator'",
        )
        .fetch_one(hub.store.reader())
        .await
        .unwrap();
        assert_eq!(cnt, 1);
        assert_eq!(kind, "operator");
        hub.ensure_operator().await.unwrap();
        let (cnt2, hash2): (i64, Vec<u8>) =
            sqlx::query_as("SELECT COUNT(*), MAX(token_hash) FROM agents WHERE handle='operator'")
                .fetch_one(hub.store.reader())
                .await
                .unwrap();
        assert_eq!(cnt2, 1, "repeat call must not create a second row");
        assert_eq!(hash1, hash2, "repeat call must not mutate the hash");
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents")
            .fetch_one(hub.store.reader())
            .await
            .unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn revoke_and_rotate_missing_handle_return_unknown_handle() {
        let hub = Hub::new_in_memory().await.unwrap();
        let ghost = Handle::parse("ghost").unwrap();
        assert!(matches!(
            hub.revoke_token(&ghost).await,
            Err(crate::error::Error::UnknownHandle(_))
        ));
        assert!(matches!(
            hub.rotate_token(&ghost).await,
            Err(crate::error::Error::UnknownHandle(_))
        ));
    }

    #[tokio::test]
    async fn register_sets_role_and_presence_online() {
        let hub = Hub::new_in_memory().await.unwrap();
        let h = Handle::parse("code").unwrap();
        hub.mint_token(&h, AgentKind::Agent).await.unwrap();
        let r = hub.register(&h, Some("app-side".into())).await.unwrap();
        assert_eq!(r.handle, "code");
        let agents = hub.list_agents().await.unwrap();
        let me = agents.iter().find(|a| a.handle == "code").unwrap();
        assert!(me.online);
        assert_eq!(me.role.as_deref(), Some("app-side"));
    }
}
