use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use maple_protocol::Secret;
use maple_transport::Error;
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug)]
pub struct Identity {
    pub id: String,
    pub username: String,
}
#[derive(Clone)]
pub struct Accounts {
    path: PathBuf,
    hash_slots: Arc<tokio::sync::Semaphore>,
    dummy: String,
}
pub fn valid_username(s: &str) -> bool {
    (3..=32).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
fn hasher() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 1, None).unwrap(),
    )
}
fn random_hex(bytes: usize) -> String {
    let mut data = vec![0u8; bytes];
    OsRng.fill_bytes(&mut data);
    data.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl Accounts {
    pub fn open(directory: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(directory)?;
        let db = Self {
            path: directory.join("accounts.sqlite"),
            hash_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            dummy: hasher()
                .hash_password(
                    b"dummy password, not an account",
                    &SaltString::generate(&mut OsRng),
                )
                .map_err(|_| "Password hashing unavailable")?
                .to_string(),
        };
        let c = db.connection()?;
        let version: i64 = c.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 1 {
            return Err("Unsupported account database schema version".into());
        }
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA max_page_count=16384; PRAGMA journal_size_limit=1048576;
          CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, hash TEXT NOT NULL, created INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS sessions(digest BLOB PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id), expires INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS snapshots(id INTEGER PRIMARY KEY CHECK(id=1), body BLOB NOT NULL);
          CREATE TRIGGER IF NOT EXISTS user_capacity BEFORE INSERT ON users WHEN (SELECT COUNT(*) FROM users)>=1024 BEGIN SELECT RAISE(ABORT,'account capacity reached'); END;
          PRAGMA user_version=1;")?;
        Ok(db)
    }
    fn connection(&self) -> Result<Connection, Error> {
        let c = Connection::open(&self.path)?;
        c.busy_timeout(std::time::Duration::from_secs(3))?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")?;
        Ok(c)
    }
    pub async fn password(
        &self,
        username: String,
        password: Secret,
        register: bool,
    ) -> Result<(Identity, Secret), String> {
        if !valid_username(&username) || password.0.len() > 256 || password.0.is_empty() {
            return Err(
                "Invalid credentials; usernames use 3–32 lowercase letters, digits, or underscores"
                    .into(),
            );
        }
        if register && password.0.chars().count() < 12 {
            return Err("Use a password of at least 12 characters".into());
        }
        let _slot = self
            .hash_slots
            .acquire()
            .await
            .map_err(|_| "Authentication unavailable")?;
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.password_sync(username, password, register))
            .await
            .map_err(|_| "Authentication unavailable".to_string())?
    }
    fn password_sync(
        &self,
        username: String,
        password: Secret,
        register: bool,
    ) -> Result<(Identity, Secret), String> {
        let c = self
            .connection()
            .map_err(|_| "Account storage unavailable")?;
        let existing = c
            .query_row(
                "SELECT id,hash FROM users WHERE username=?1",
                [&username],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| "Account storage unavailable")?;
        let id = if register {
            if existing.is_some() {
                return Err("Account cannot be registered".into());
            }
            let count: i64 = c
                .query_row("SELECT count(*) FROM users", [], |r| r.get(0))
                .map_err(|_| "Account storage unavailable")?;
            if count >= 1024 {
                return Err("Account capacity reached".into());
            }
            let hash = hasher()
                .hash_password(password.0.as_bytes(), &SaltString::generate(&mut OsRng))
                .map_err(|_| "Authentication unavailable")?
                .to_string();
            let id = random_hex(16);
            c.execute(
                "INSERT INTO users(id,username,hash,created) VALUES(?1,?2,?3,?4)",
                params![id, username, hash, now()],
            )
            .map_err(|_| "Account cannot be registered")?;
            id
        } else {
            let hash = existing.as_ref().map(|(_, h)| h).unwrap_or(&self.dummy);
            let parsed = PasswordHash::new(hash).map_err(|_| "Invalid username or password")?;
            let verified = hasher()
                .verify_password(password.0.as_bytes(), &parsed)
                .is_ok();
            if !verified || existing.is_none() {
                return Err("Invalid username or password".into());
            }
            existing.unwrap().0
        };
        self.issue(&Identity {
            id: id.clone(),
            username: username.clone(),
        })
        .map(|token| (Identity { id, username }, token))
        .map_err(|_| "Session storage unavailable".into())
    }
    fn issue(&self, user: &Identity) -> Result<Secret, Error> {
        let mut c = self.connection()?;
        let transaction = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM sessions WHERE expires<=?1", [now()])?;
        // Bound remembered sessions per account, including those abandoned by a crash.
        let count: i64 = transaction.query_row(
            "SELECT count(*) FROM sessions WHERE user_id=?1",
            [&user.id],
            |r| r.get(0),
        )?;
        if count >= 8 {
            transaction.execute("DELETE FROM sessions WHERE digest IN (SELECT digest FROM sessions WHERE user_id=?1 ORDER BY expires LIMIT 1)",[&user.id])?;
        }
        let token = Secret(random_hex(32));
        transaction.execute(
            "INSERT INTO sessions(digest,user_id,expires) VALUES(?1,?2,?3)",
            params![digest(&token.0), user.id, now() + 3600],
        )?;
        transaction.commit()?;
        Ok(token)
    }
    pub async fn resume(&self, token: Secret) -> Result<(Identity, Secret), String> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            if token.0.len()!=64{return Err("Session expired or revoked".to_string());}
            let mut c=db.connection().map_err(|_|"Session unavailable".to_string())?;
            let transaction=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|"Session unavailable".to_string())?;
            let user=transaction.query_row("SELECT u.id,u.username FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.digest=?1 AND s.expires>?2",params![digest(&token.0),now()],|r|Ok(Identity{id:r.get(0)?,username:r.get(1)?})).map_err(|_|"Session expired or revoked".to_string())?;
            transaction.execute("DELETE FROM sessions WHERE digest=?1",[digest(&token.0)]).map_err(|_|"Session unavailable".to_string())?;
            let new=Secret(random_hex(32));transaction.execute("INSERT INTO sessions(digest,user_id,expires) VALUES(?1,?2,?3)",params![digest(&new.0),user.id,now()+3600]).map_err(|_|"Session unavailable".to_string())?;
            transaction.commit().map_err(|_|"Session unavailable".to_string())?;
            Ok((user, new))
        })
        .await
        .map_err(|_| "Session unavailable".to_string())?
    }
    fn session_sync(&self, token: &Secret) -> Result<Identity, Error> {
        if token.0.len() != 64 {
            return Err("Invalid session".into());
        }
        let c = self.connection()?;
        Ok(c.query_row("SELECT u.id,u.username FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.digest=?1 AND s.expires>?2",params![digest(&token.0),now()],|r|Ok(Identity{id:r.get(0)?,username:r.get(1)?}))?)
    }
    pub async fn valid(&self, token: Secret) -> bool {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.session_sync(&token).is_ok())
            .await
            .unwrap_or(false)
    }
    fn revoke_sync(&self, token: &Secret) -> Result<(), Error> {
        self.connection()?
            .execute("DELETE FROM sessions WHERE digest=?1", [digest(&token.0)])?;
        Ok(())
    }
    pub async fn revoke(&self, token: Secret) -> Result<(), String> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            db.revoke_sync(&token)
                .map_err(|_| "Session revocation storage unavailable".to_string())
        })
        .await
        .map_err(|_| "Session revocation unavailable".to_string())?
    }
    pub fn names(&self) -> Result<std::collections::BTreeMap<String, String>, Error> {
        let c = self.connection()?;
        let mut query = c.prepare("SELECT id,username FROM users")?;
        let rows = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    pub fn snapshot(&self) -> Result<Option<Vec<u8>>, Error> {
        Ok(self
            .connection()?
            .query_row("SELECT body FROM snapshots WHERE id=1", [], |r| r.get(0))
            .optional()?)
    }
    pub fn save_snapshot(&self, bytes: Vec<u8>) -> Result<(), Error> {
        if bytes.len() > 32_000_000 {
            return Err("Chat storage capacity reached".into());
        }
        let c = self.connection()?;
        c.execute("INSERT INTO snapshots(id,body) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",[bytes])?;
        // The upsert is already committed. Checkpoint contention must not make the
        // caller roll back memory after durable storage has accepted the mutation.
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        Ok(())
    }
}
