use crate::error::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, SqliteConnection, SqlitePool};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

static MEM_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn wrap_migrate(e: &sqlx::migrate::MigrateError) -> sqlx::Error {
    sqlx::Error::Configuration(e.to_string().into())
}

fn base_opts(opts: SqliteConnectOptions) -> SqliteConnectOptions {
    opts.foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5))
}

pub struct Store {
    writer: Mutex<SqliteConnection>,
    reader: SqlitePool,
    temp_path: Option<PathBuf>,
}

impl Store {
    pub async fn open(path: &str) -> Result<Self> {
        let opts = base_opts(
            SqliteConnectOptions::from_str(&format!("sqlite://{path}"))
                .map_err(|e| sqlx::Error::Configuration(e.to_string().into()))?
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal),
        );
        let store = Self::from_file_opts(opts, None).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let db_path = std::path::Path::new(path);
            let meta = std::fs::metadata(db_path).map_err(sqlx::Error::Io)?;
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(db_path, perms).map_err(sqlx::Error::Io)?;
            if let Some(parent) = db_path.parent() {
                if let Ok(dir_meta) = std::fs::metadata(parent) {
                    let mut dir_perms = dir_meta.permissions();
                    dir_perms.set_mode(0o700);
                    let _ = std::fs::set_permissions(parent, dir_perms);
                }
            }
            for ext in &["db-wal", "db-shm"] {
                let sidecar = db_path.with_extension(ext);
                if let Ok(sc_meta) = std::fs::metadata(&sidecar) {
                    let mut sc_perms = sc_meta.permissions();
                    sc_perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&sidecar, sc_perms);
                }
            }
        }
        Ok(store)
    }

    pub async fn open_in_memory() -> Result<Self> {
        let id = MEM_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("babylon_mem_{}_{id}.db", std::process::id()));
        let opts = base_opts(
            SqliteConnectOptions::from_str(&format!("sqlite://{}", path.to_string_lossy()))
                .map_err(|e| sqlx::Error::Configuration(e.to_string().into()))?
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal),
        );
        Self::from_file_opts(opts, Some(path)).await
    }

    async fn from_file_opts(
        opts: SqliteConnectOptions,
        temp_path: Option<PathBuf>,
    ) -> Result<Self> {
        let reader = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts.clone())
            .await?;
        sqlx::migrate!()
            .run(&reader)
            .await
            .map_err(|e| wrap_migrate(&e))?;
        let writer = opts.connect().await?;
        Ok(Self {
            writer: Mutex::new(writer),
            reader,
            temp_path,
        })
    }

    #[must_use]
    pub const fn reader(&self) -> &SqlitePool {
        &self.reader
    }

    pub async fn with_writer<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(
            &'a mut SqliteConnection,
        ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
    {
        let mut conn = self.writer.lock().await;
        f(&mut conn).await
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Some(path) = &self.temp_path {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn store_open_sets_restrictive_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "babylon_perm_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        let path_str = path.to_str().unwrap().to_string();
        let _store = Store::open(&path_str).await.unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        assert_eq!(mode, 0o600, "db file must be mode 0600, got {mode:o}");
    }

    #[tokio::test]
    async fn opens_migrates_and_roundtrips_a_write() {
        let store = Store::open_in_memory().await.unwrap();
        let now = 1_000i64;
        store
            .with_writer(|conn| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO agents(handle, token_hash, created_at) VALUES (?,?,?)",
                    )
                    .bind("code")
                    .bind(vec![1u8; 32])
                    .bind(now)
                    .execute(conn)
                    .await?;
                    Ok(())
                })
            })
            .await
            .unwrap();
        let cnt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents")
            .fetch_one(store.reader())
            .await
            .unwrap();
        assert_eq!(cnt, 1);
    }
}
