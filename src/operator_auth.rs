//! Local remembered browser sessions. Only token hashes and finite lifecycle/scope data persist.
use crate::{ApiError, Result, store::internal};
use axum::http::{HeaderMap, header};
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
pub(crate) const TTL_MS: i64 = 400 * 24 * 60 * 60 * 1000;
const RENEW_MS: i64 = 24 * 60 * 60 * 1000;
pub(crate) struct Auth {
    pub cookie_name: String,
    authority: String,
    secret: [u8; 32],
    binding: String,
}
struct Row {
    origin: String,
    scope: String,
    issued: i64,
    renewed: i64,
    expires: i64,
}
pub(crate) struct Session {
    pub csrf: String,
    pub expires: i64,
}
fn mac(secret: &[u8], data: &str) -> String {
    let mut h = Hmac::<Sha256>::new_from_slice(secret).expect("fixed HMAC key");
    h.update(data.as_bytes());
    format!("{:x}", h.finalize().into_bytes())
}
pub(crate) fn hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
impl Auth {
    pub fn open(db: &mut Connection, path: &Path, secret: [u8; 32], port: u16) -> Result<Self> {
        let path = path.canonicalize().map_err(internal)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS operator_auth(singleton INTEGER PRIMARY KEY CHECK(singleton=1),database_id TEXT NOT NULL,binding TEXT NOT NULL); CREATE TABLE IF NOT EXISTS operator_sessions(token_hash TEXT PRIMARY KEY,origin TEXT NOT NULL,scope TEXT NOT NULL,issued_ms INTEGER NOT NULL,renewed_ms INTEGER NOT NULL,expires_ms INTEGER NOT NULL);").map_err(internal)?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT database_id,binding FROM operator_auth WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(internal)?;
        let id = prior
            .as_ref()
            .map(|p| p.0.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let binding = mac(
            &secret,
            &format!("workboard-browser-authority-v1\0{id}\0{}", path.display()),
        );
        if prior.as_ref().is_none_or(|p| p.1 != binding) {
            // Deletion is durable before startup accepts requests, including rotation back to an older key.
            tx.execute("DELETE FROM operator_sessions", [])
                .map_err(internal)?;
            tx.execute("INSERT INTO operator_auth VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET binding=excluded.binding",params![id,binding]).map_err(internal)?;
        }
        tx.commit().map_err(internal)?;
        Ok(Self {
            cookie_name: format!("workboard_rs_session_{port}"),
            authority: format!("127.0.0.1:{port}"),
            secret,
            binding,
        })
    }
    fn check(&self, db: &Connection) -> Result<()> {
        let binding: String = db
            .query_row(
                "SELECT binding FROM operator_auth WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(internal)?;
        if binding != self.binding {
            return Err(ApiError::unavailable("operator_session_store_unavailable"));
        }
        Ok(())
    }
    fn scope(&self, origin: &str) -> String {
        mac(
            &self.secret,
            &format!("workboard-browser-scope-v1\0{}\0{origin}", self.binding),
        )
    }
    pub fn origin(&self, headers: &HeaderMap) -> Option<String> {
        let host = headers.get(header::HOST)?.to_str().ok()?;
        if host != self.authority && host != self.authority.replacen("127.0.0.1", "localhost", 1) {
            return None;
        }
        let origin = format!("http://{host}");
        if headers
            .get(header::ORIGIN)
            .is_some_and(|v| v.to_str().ok() != Some(origin.as_str()))
        {
            return None;
        }
        Some(origin)
    }
    pub fn token(&self, headers: &HeaderMap) -> Option<String> {
        let prefix = format!("{}=", self.cookie_name);
        let mut found = None;
        for header in headers.get_all(header::COOKIE) {
            for part in header.to_str().ok()?.split(';') {
                if let Some(value) = part.trim().strip_prefix(&prefix) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(value.to_string());
                }
            }
        }
        found.filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    }
    fn row(&self, db: &Connection, id: &str, now: i64) -> Result<Option<Row>> {
        self.check(db)?;
        let row:Option<Row>=db.query_row("SELECT origin,scope,issued_ms,renewed_ms,expires_ms FROM operator_sessions WHERE token_hash=?1",[id],|r|Ok(Row{origin:r.get(0)?,scope:r.get(1)?,issued:r.get(2)?,renewed:r.get(3)?,expires:r.get(4)?})).optional().map_err(internal)?;
        let Some(row) = row else { return Ok(None) };
        if now < 0
            || row.issued < 0
            || row.issued > row.renewed
            || row.renewed >= row.expires
            || row.expires - row.renewed > TTL_MS
        {
            return Err(ApiError::unavailable("operator_session_store_unavailable"));
        }
        if row.scope != self.scope(&row.origin) || row.renewed > now || row.expires <= now {
            return Ok(None);
        }
        Ok(Some(row))
    }
    pub fn session(&self, db: &Connection, id: &str, now: i64) -> Result<Option<Session>> {
        Ok(self.row(db, id, now)?.map(|r| Session {
            csrf: mac(
                &self.secret,
                &format!(
                    "workboard-browser-csrf-v1\0{}\0{id}\0{}",
                    self.binding, r.origin
                ),
            ),
            expires: r.expires,
        }))
    }
    pub fn read(&self, db: &Connection, headers: &HeaderMap, now: i64) -> Result<Option<String>> {
        let (Some(token), Some(origin)) = (self.token(headers), self.origin(headers)) else {
            return Ok(None);
        };
        let id = hash(&token);
        Ok(self
            .row(db, &id, now)?
            .filter(|r| r.origin == origin)
            .map(|_| id))
    }
    pub fn issue(
        &self,
        db: &mut Connection,
        origin: &str,
        prior: Option<&str>,
        now: i64,
    ) -> Result<String> {
        if now < 0 {
            return Err(ApiError::internal());
        }
        let expires = now.checked_add(TTL_MS).ok_or_else(ApiError::internal)?;
        let token = hash(&format!(
            "{}{}{}",
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4()
        ));
        let id = hash(&token);
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        self.check(&tx)?;
        tx.execute("DELETE FROM operator_sessions WHERE expires_ms<=?1", [now])
            .map_err(internal)?;
        if let Some(prior) = prior {
            tx.execute(
                "DELETE FROM operator_sessions WHERE token_hash=?1 AND origin=?2",
                params![prior, origin],
            )
            .map_err(internal)?;
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM operator_sessions", [], |r| r.get(0))
            .map_err(internal)?;
        if count >= 256 {
            return Err(ApiError::new(429, "session_limit_reached"));
        }
        tx.execute(
            "INSERT INTO operator_sessions VALUES(?1,?2,?3,?4,?4,?5)",
            params![id, origin, self.scope(origin), now, expires],
        )
        .map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok(token)
    }
    pub fn renew(
        &self,
        db: &mut Connection,
        headers: &HeaderMap,
        now: i64,
    ) -> Result<Option<String>> {
        let Some(id) = self.read(db, headers, now)? else {
            return Ok(None);
        };
        let row = self.row(db, &id, now)?.ok_or_else(ApiError::internal)?;
        if now - row.renewed < RENEW_MS {
            return Ok(None);
        }
        let expires = now.checked_add(TTL_MS).ok_or_else(ApiError::internal)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(internal)?;
        self.check(&tx)?;
        let changed=tx.execute("UPDATE operator_sessions SET renewed_ms=?1,expires_ms=?2 WHERE token_hash=?3 AND renewed_ms=?4 AND expires_ms>?1",params![now,expires,id,row.renewed]).map_err(internal)?;
        tx.commit().map_err(internal)?;
        Ok((changed == 1).then(|| self.token(headers).unwrap()))
    }
    pub fn revoke(&self, db: &Connection, id: &str) -> Result<()> {
        self.check(db)?;
        db.execute("DELETE FROM operator_sessions WHERE token_hash=?1", [id])
            .map_err(internal)?;
        Ok(())
    }
    pub fn cookie(&self, token: &str) -> String {
        format!(
            "{}={token}; HttpOnly; SameSite=Strict; Path=/api/workboard/v1; Max-Age={}",
            self.cookie_name,
            if token.is_empty() { 0 } else { TTL_MS / 1000 }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(port: u16, token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, format!("127.0.0.1:{port}").parse().unwrap());
        h.insert(
            header::COOKIE,
            format!("workboard_rs_session_{port}={token}")
                .parse()
                .unwrap(),
        );
        h
    }
    #[test]
    fn remembered_clock_renewal_expiry_hashes_and_permanent_key_rotation() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        let a = Auth::open(&mut db, file.path(), [1; 32], 4311).unwrap();
        let now = 1000;
        let token = a
            .issue(&mut db, "http://127.0.0.1:4311", None, now)
            .unwrap();
        let id = hash(&token);
        let h = headers(4311, &token);
        let csrf = a.session(&db, &id, now).unwrap().unwrap().csrf;
        let raw: String = db
            .query_row("SELECT token_hash FROM operator_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw, id);
        assert_ne!(raw, token);
        assert_ne!(csrf, token);
        assert_ne!(csrf, id);
        assert!(a.renew(&mut db, &h, now + RENEW_MS - 1).unwrap().is_none());
        let renewed = now + RENEW_MS;
        assert_eq!(a.renew(&mut db, &h, renewed).unwrap(), Some(token.clone()));
        assert!(a.cookie(&token).contains("Max-Age=34560000"));
        assert_eq!(
            a.session(&db, &id, renewed).unwrap().unwrap().expires,
            renewed + TTL_MS
        );
        assert!(a.read(&db, &h, now + TTL_MS + 1).unwrap().is_some());
        assert!(a.read(&db, &h, renewed + TTL_MS).unwrap().is_none());
        let reopened = Auth::open(&mut db, file.path(), [1; 32], 4311).unwrap();
        assert_eq!(
            reopened.session(&db, &id, renewed).unwrap().unwrap().csrf,
            csrf
        );
        let b = Auth::open(&mut db, file.path(), [2; 32], 4311).unwrap();
        assert!(b.read(&db, &h, renewed).unwrap().is_none());
        let again = Auth::open(&mut db, file.path(), [1; 32], 4311).unwrap();
        assert!(again.read(&db, &h, renewed).unwrap().is_none());
    }
    #[test]
    fn cookies_are_unambiguous_and_scope_origin_port_and_database_copy() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        let a = Auth::open(&mut db, file.path(), [1; 32], 4311).unwrap();
        let token = a
            .issue(&mut db, "http://127.0.0.1:4311", None, 1000)
            .unwrap();
        let h = headers(4311, &token);
        assert!(a.read(&db, &h, 1000).unwrap().is_some());
        let b = Auth::open(&mut db, file.path(), [1; 32], 4315).unwrap();
        assert_ne!(a.cookie_name, b.cookie_name);
        assert!(b.read(&db, &headers(4315, &token), 1000).unwrap().is_none());
        let mut duplicate = h.clone();
        duplicate.append(header::COOKIE, h[header::COOKIE].clone());
        assert!(a.read(&db, &duplicate, 1000).unwrap().is_none());
        duplicate = h.clone();
        duplicate.insert(
            header::COOKIE,
            format!("{}={token}; {}={token}", a.cookie_name, a.cookie_name)
                .parse()
                .unwrap(),
        );
        assert!(a.read(&db, &duplicate, 1000).unwrap().is_none());
        let mut alias = h.clone();
        alias.insert(header::HOST, "localhost:4311".parse().unwrap());
        assert!(a.read(&db, &alias, 1000).unwrap().is_none());
        let mut foreign = h.clone();
        foreign.insert(header::ORIGIN, "http://127.0.0.1:4315".parse().unwrap());
        assert!(a.read(&db, &foreign, 1000).unwrap().is_none());
        let other = tempfile::NamedTempFile::new().unwrap();
        let copied = Auth::open(&mut db, other.path(), [1; 32], 4311).unwrap();
        assert!(copied.read(&db, &h, 1000).unwrap().is_none());
    }
    #[test]
    fn storage_failures_never_claim_issuance_renewal_or_revocation() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        let a = Auth::open(&mut db, file.path(), [1; 32], 4311).unwrap();
        db.execute_batch("CREATE TRIGGER fail_issue BEFORE INSERT ON operator_sessions BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        assert!(
            a.issue(&mut db, "http://127.0.0.1:4311", None, 1000)
                .is_err()
        );
        db.execute_batch("DROP TRIGGER fail_issue;").unwrap();
        let token = a
            .issue(&mut db, "http://127.0.0.1:4311", None, 1000)
            .unwrap();
        let id = hash(&token);
        let h = headers(4311, &token);
        db.execute_batch("CREATE TRIGGER fail_renew BEFORE UPDATE ON operator_sessions BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        assert!(a.renew(&mut db, &h, 1000 + RENEW_MS).is_err());
        assert_eq!(
            a.session(&db, &id, 1000).unwrap().unwrap().expires,
            1000 + TTL_MS
        );
        db.execute_batch("DROP TRIGGER fail_renew;").unwrap();
        db.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON operator_sessions BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        assert!(a.revoke(&db, &id).is_err());
        assert!(Auth::open(&mut db, file.path(), [2; 32], 4311).is_err());
        assert!(a.read(&db, &h, 1000).unwrap().is_some());
    }
}
