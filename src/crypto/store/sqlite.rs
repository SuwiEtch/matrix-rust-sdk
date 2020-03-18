use std::path::Path;
use std::sync::Arc;
use url::Url;

use async_trait::async_trait;
use olm_rs::PicklingMode;
use sqlx::{query, query_as, sqlite::SqliteQueryAs, Connect, Executor, SqliteConnection};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use super::{Account, CryptoStore, Result};

pub struct SqliteStore {
    user_id: Arc<String>,
    device_id: Arc<String>,
    connection: Arc<Mutex<SqliteConnection>>,
    pickle_passphrase: Option<Zeroizing<String>>,
}

static DATABASE_NAME: &str = "matrix-sdk-crypto.db";

impl SqliteStore {
    async fn open<P: AsRef<Path>>(user_id: &str, device_id: &str, path: P) -> Result<SqliteStore> {
        SqliteStore::open_helper(user_id, device_id, path, None).await
    }

    async fn open_with_passphrase<P: AsRef<Path>>(
        user_id: &str,
        device_id: &str,
        path: P,
        passphrase: String,
    ) -> Result<SqliteStore> {
        SqliteStore::open_helper(user_id, device_id, path, Some(Zeroizing::new(passphrase))).await
    }

    async fn open_helper<P: AsRef<Path>>(
        user_id: &str,
        device_id: &str,
        path: P,
        passphrase: Option<Zeroizing<String>>,
    ) -> Result<SqliteStore> {
        let url = Url::from_directory_path(path.as_ref()).unwrap();
        let url = url.join(DATABASE_NAME).unwrap();

        let connection = SqliteConnection::connect(url.as_ref()).await.unwrap();
        let store = SqliteStore {
            user_id: Arc::new(user_id.to_owned()),
            device_id: Arc::new(device_id.to_owned()),
            connection: Arc::new(Mutex::new(connection)),
            pickle_passphrase: passphrase,
        };
        store.create_tables().await?;
        Ok(store)
    }

    async fn create_tables(&self) -> Result<()> {
        let mut connection = self.connection.lock().await;
        connection
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS account (
                "id" INTEGER NOT NULL PRIMARY KEY,
                "user_id" TEXT NOT NULL,
                "device_id" TEXT NOT NULL,
                "pickle" BLOB NOT NULL,
                "shared" INTEGER NOT NULL,
                UNIQUE(user_id,device_id)
            );
        "#,
            )
            .await
            .unwrap();

        Ok(())
    }

    fn get_pickle_mode(&self) -> PicklingMode {
        match &self.pickle_passphrase {
            Some(p) => PicklingMode::Encrypted {
                key: p.as_bytes().to_vec(),
            },
            None => PicklingMode::Unencrypted,
        }
    }
}

#[async_trait]
impl CryptoStore for SqliteStore {
    async fn load_account(&self) -> Result<Account> {
        let mut connection = self.connection.lock().await;

        let (pickle, shared): (String, bool) = query_as(
            "SELECT pickle, shared FROM account
                      WHERE user_id = ? and device_id = ?",
        )
        .bind(&*self.user_id)
        .bind(&*self.device_id)
        .fetch_one(&mut *connection)
        .await
        .unwrap();

        Ok(Account::from_pickle(pickle, self.get_pickle_mode(), shared).unwrap())
    }

    async fn save_account(&self, account: Arc<Mutex<Account>>) -> Result<()> {
        let acc = account.lock().await;
        let pickle = acc.pickle(self.get_pickle_mode());
        let mut connection = self.connection.lock().await;

        query(
            "INSERT OR IGNORE INTO account (
                user_id, device_id, pickle, shared
             ) VALUES (?, ?, ?, ?)",
        )
        .bind(&*self.user_id)
        .bind(&*self.device_id)
        .bind(pickle)
        .bind(true)
        .execute(&mut *connection)
        .await
        .unwrap();

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    use super::{Account, CryptoStore, SqliteStore};

    async fn get_store() -> SqliteStore {
        let tmpdir = tempdir().unwrap();
        let tmpdir_path = tmpdir.path().to_str().unwrap();
        SqliteStore::open("@example:localhost", "DEVICEID", tmpdir_path)
            .await
            .expect("Can't create store")
    }

    fn get_account() -> Arc<Mutex<Account>> {
        let account = Account::new();
        Arc::new(Mutex::new(account))
    }

    #[tokio::test]
    async fn create_store() {
        let tmpdir = tempdir().unwrap();
        let tmpdir_path = tmpdir.path().to_str().unwrap();
        let _ = SqliteStore::open("@example:localhost", "DEVICEID", tmpdir_path)
            .await
            .expect("Can't create store");
    }

    #[tokio::test]
    async fn save_account() {
        let store = get_store().await;
        let account = get_account();

        store
            .save_account(account)
            .await
            .expect("Can't save account");
    }

    #[tokio::test]
    async fn load_account() {
        let store = get_store().await;
        let account = get_account();

        store
            .save_account(account.clone())
            .await
            .expect("Can't save account");

        let acc = account.lock().await;
        let loaded_account = store.load_account().await.expect("Can't load account");

        assert_eq!(*acc, loaded_account);
    }
}
