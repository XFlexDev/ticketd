use anyhow::Result;
use serde_json::{json, Value};

/// KreatixDB HTTP client — POST {sql, params} to {url}/query, bearer key.
#[derive(Clone)]
pub struct Kdb {
    http: reqwest::Client,
    url: String,
    key: String,
}

pub struct Settings {
    pub guild_id: i64,
    pub ticket_category_id: i64,
    pub mod_role_id: i64,
    pub log_channel_id: Option<i64>,
    pub ai_enabled: bool,
}

pub struct Ticket {
    pub id: i64,
    pub channel_id: i64,
    pub user_id: i64,
    pub subject: String,
    pub status: String,
}

impl Kdb {
    pub fn from_env() -> Result<Self> {
        let url = std::env::var("KDB_URL")?.trim_end_matches('/').to_string();
        let key = std::env::var("KDB_KEY")?;
        Ok(Self { http: reqwest::Client::new(), url, key })
    }

    async fn exec(&self, sql: &str, params: &[Value]) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}/query", self.url))
            .bearer_auth(&self.key)
            .json(&json!({"sql": sql, "params": params}))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("database error {status}: {}", body.chars().take(300).collect::<String>());
        }
        let mut v: Value = resp.json().await?;
        unwrap_big_ints(&mut v);
        Ok(v["results"][0].clone())
    }

    async fn rows(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
        Ok(self.exec(sql, params).await?["rows"].as_array().cloned().unwrap_or_default())
    }

    pub async fn init(&self) -> Result<()> {
        self.exec(
            "CREATE TABLE IF NOT EXISTS tickets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                guild_id INTEGER NOT NULL,
                channel_id INTEGER NOT NULL UNIQUE,
                user_id INTEGER NOT NULL,
                subject TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'open',
                created_at INTEGER NOT NULL,
                closed_at INTEGER
            )",
            &[],
        )
        .await?;
        self.exec(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ticket_id INTEGER NOT NULL,
                author TEXT NOT NULL,
                author_id INTEGER NOT NULL,
                content TEXT NOT NULL,
                is_staff INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            )",
            &[],
        )
        .await?;
        self.exec("CREATE INDEX IF NOT EXISTS idx_messages_ticket ON messages(ticket_id)", &[]).await?;
        self.exec(
            "CREATE TABLE IF NOT EXISTS sessions (
                token TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                user_name TEXT NOT NULL,
                guilds TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            )",
            &[],
        )
        .await?;
        self.exec(
            "CREATE TABLE IF NOT EXISTS settings (
                guild_id INTEGER PRIMARY KEY,
                ticket_category_id INTEGER NOT NULL,
                mod_role_id INTEGER NOT NULL,
                log_channel_id INTEGER,
                ai_enabled INTEGER NOT NULL DEFAULT 1
            )",
            &[],
        )
        .await?;
        Ok(())
    }

    pub async fn create_ticket(&self, guild_id: u64, channel_id: u64, user_id: u64, subject: &str) -> Result<i64> {
        let r = self
            .exec(
                "INSERT INTO tickets (guild_id, channel_id, user_id, subject, status, created_at) VALUES (?,?,?,?,?,?)",
                &[json!(guild_id), json!(channel_id), json!(user_id), json!(subject), json!("open"), json!(now())],
            )
            .await?;
        Ok(r["lastInsertRowid"].as_i64().unwrap_or(0))
    }

    fn ticket(v: &Value) -> Ticket {
        Ticket {
            id: v[0].as_i64().unwrap_or(0),
            channel_id: v[1].as_i64().unwrap_or(0),
            user_id: v[2].as_i64().unwrap_or(0),
            subject: v[3].as_str().unwrap_or("").into(),
            status: v[4].as_str().unwrap_or("").into(),
        }
    }

    pub async fn by_channel(&self, channel_id: u64) -> Result<Option<Ticket>> {
        let rows = self
            .rows("SELECT id, channel_id, user_id, subject, status FROM tickets WHERE channel_id = ?", &[json!(channel_id)])
            .await?;
        Ok(rows.first().map(Self::ticket))
    }

    pub async fn open_ticket_for_user(&self, guild_id: u64, user_id: u64) -> Result<Option<Ticket>> {
        let rows = self
            .rows(
                "SELECT id, channel_id, user_id, subject, status FROM tickets WHERE guild_id = ? AND user_id = ? AND status = 'open' ORDER BY id DESC LIMIT 1",
                &[json!(guild_id), json!(user_id)],
            )
            .await?;
        Ok(rows.first().map(Self::ticket))
    }

    pub async fn close_ticket(&self, ticket_id: i64) -> Result<()> {
        self.exec("UPDATE tickets SET status = 'closed', closed_at = ? WHERE id = ?", &[json!(now()), json!(ticket_id)]).await?;
        Ok(())
    }

    pub async fn settings(&self, guild_id: u64) -> Result<Option<Settings>> {
        let rows = self
            .rows(
                "SELECT guild_id, ticket_category_id, mod_role_id, log_channel_id, ai_enabled FROM settings WHERE guild_id = ?",
                &[json!(guild_id)],
            )
            .await?;
        Ok(rows.first().map(|r| Settings {
            guild_id: r[0].as_i64().unwrap_or(0),
            ticket_category_id: r[1].as_i64().unwrap_or(0),
            mod_role_id: r[2].as_i64().unwrap_or(0),
            log_channel_id: r[3].as_i64(),
            ai_enabled: r[4].as_i64() != Some(0),
        }))
    }

    pub async fn upsert_settings(&self, s: &Settings) -> Result<()> {
        self.exec(
            "INSERT INTO settings (guild_id, ticket_category_id, mod_role_id, log_channel_id, ai_enabled)
             VALUES (?,?,?,?,?)
             ON CONFLICT(guild_id) DO UPDATE SET ticket_category_id=excluded.ticket_category_id,
                mod_role_id=excluded.mod_role_id, log_channel_id=excluded.log_channel_id, ai_enabled=excluded.ai_enabled",
            &[
                json!(s.guild_id),
                json!(s.ticket_category_id),
                json!(s.mod_role_id),
                match s.log_channel_id { Some(v) => json!(v), None => Value::Null },
                json!(s.ai_enabled as i64),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_tickets(&self, guild_id: u64, limit: i64) -> Result<Vec<Ticket>> {
        let rows = self
            .rows(
                "SELECT id, channel_id, user_id, subject, status FROM tickets WHERE guild_id = ? ORDER BY id DESC LIMIT ?",
                &[json!(guild_id), json!(limit)],
            )
            .await?;
        Ok(rows.iter().map(Self::ticket).collect())
    }

    pub async fn ticket_guild(&self, ticket_id: i64) -> Result<Option<i64>> {
        let rows = self.rows("SELECT guild_id FROM tickets WHERE id = ?", &[json!(ticket_id)]).await?;
        Ok(rows.first().and_then(|r| r[0].as_i64()))
    }

    pub async fn save_session(&self, token: &str, user_id: u64, user_name: &str, guilds: &str, expires_at: i64) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO sessions (token, user_id, user_name, guilds, expires_at) VALUES (?,?,?,?,?)",
            &[json!(token), json!(user_id), json!(user_name), json!(guilds), json!(expires_at)],
        )
        .await?;
        Ok(())
    }

    /// -> (user_id, user_name, guilds_json)
    pub async fn session(&self, token: &str) -> Result<Option<(i64, String, String)>> {
        let rows = self
            .rows(
                "SELECT user_id, user_name, guilds FROM sessions WHERE token = ? AND expires_at > ?",
                &[json!(token), json!(now())],
            )
            .await?;
        Ok(rows.first().map(|r| {
            (r[0].as_i64().unwrap_or(0), r[1].as_str().unwrap_or("").into(), r[2].as_str().unwrap_or("[]").into())
        }))
    }

    pub async fn delete_session(&self, token: &str) -> Result<()> {
        self.exec("DELETE FROM sessions WHERE token = ?", &[json!(token)]).await?;
        Ok(())
    }

    pub async fn count_tickets(&self, guild_id: u64) -> Result<i64> {
        let rows = self.rows("SELECT COUNT(*) FROM tickets WHERE guild_id = ?", &[json!(guild_id)]).await?;
        Ok(rows.first().and_then(|r| r[0].as_i64()).unwrap_or(0))
    }

    pub async fn log_message(&self, ticket_id: i64, author: &str, author_id: u64, content: &str, is_staff: bool) -> Result<()> {
        self.exec(
            "INSERT INTO messages (ticket_id, author, author_id, content, is_staff, created_at) VALUES (?,?,?,?,?,?)",
            &[json!(ticket_id), json!(author), json!(author_id), json!(content), json!(is_staff as i64), json!(now())],
        )
        .await?;
        Ok(())
    }

    /// (role, content) pairs for the AI; bot replies are `assistant`.
    /// A human staff member (not the bot) has written in this ticket.
    pub async fn staff_replied(&self, ticket_id: i64, bot_id: u64) -> Result<bool> {
        let rows = self
            .rows(
                "SELECT 1 FROM messages WHERE ticket_id = ? AND is_staff = 1 AND author_id != ? LIMIT 1",
                &[json!(ticket_id), json!(bot_id)],
            )
            .await?;
        Ok(!rows.is_empty())
    }
    pub async fn history(&self, ticket_id: i64, limit: i64) -> Result<Vec<(String, String)>> {
        let rows = self
            .rows(
                "SELECT content, is_staff FROM (SELECT id, content, is_staff FROM messages WHERE ticket_id = ? ORDER BY id DESC LIMIT ?) ORDER BY id",
                &[json!(ticket_id), json!(limit)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let role = if r[1].as_i64() == Some(1) { "assistant" } else { "user" };
                (role.to_string(), r[0].as_str().unwrap_or("").to_string())
            })
            .collect())
    }

    pub async fn transcript(&self, ticket_id: i64) -> Result<String> {
        let rows = self
            .rows("SELECT author, content, created_at FROM messages WHERE ticket_id = ? ORDER BY id", &[json!(ticket_id)])
            .await?;
        let mut s = String::new();
        for r in rows {
            s.push_str(&format!(
                "[{}] {}: {}\n",
                r[2].as_i64().unwrap_or(0),
                r[0].as_str().unwrap_or("?"),
                r[1].as_str().unwrap_or("")
            ));
        }
        Ok(s)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// KreatixDB encodes integers beyond JS safe range as {"$integer": "123"}.
fn unwrap_big_ints(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if m.len() == 1 {
                if let Some(n) = m.get("$integer").and_then(|s| s.as_str()).and_then(|s| s.parse::<i64>().ok()) {
                    *v = Value::from(n);
                    return;
                }
            }
            m.values_mut().for_each(unwrap_big_ints);
        }
        Value::Array(a) => a.iter_mut().for_each(unwrap_big_ints),
        _ => {}
    }
}
