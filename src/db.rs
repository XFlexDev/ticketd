use anyhow::Result;
use serde_json::{json, Value};

/// KreatixDB HTTP client — POST {sql, params} to {url}/query, bearer key.
#[derive(Clone)]
pub struct Kdb {
    http: reqwest::Client,
    url: String,
    key: String,
}

#[derive(Clone, Default)]
pub struct Settings {
    pub guild_id: i64,
    pub ticket_category_id: i64,
    pub mod_role_id: i64,
    pub log_channel_id: Option<i64>,
    pub ai_enabled: bool,
    /// 0 = off
    pub autoclose_hours: i64,
    pub close_on_leave: bool,
    pub ratings: bool,
}

#[derive(Clone, Default)]
pub struct Ticket {
    pub id: i64,
    pub channel_id: i64,
    pub user_id: i64,
    pub subject: String,
    pub status: String,
    pub guild_id: i64,
    pub created_at: i64,
    pub closed_at: i64,
    pub staff_roles: Vec<u64>,
    pub ping_roles: Vec<u64>,
    pub instructions: String,
    pub claimed_by: i64,
    pub priority: String,
    pub escalated: bool,
    pub rating: i64,
    pub rating_comment: String,
    pub last_activity: i64,
    pub warned: bool,
    pub closed_by: i64,
}

const TCOLS: &str = "id, channel_id, user_id, subject, status, guild_id, created_at, closed_at, staff_roles, ping_roles, instructions, claimed_by, priority, escalated, rating, rating_comment, last_activity, warned, closed_by";

fn ids(v: &Value) -> Vec<u64> {
    v.as_str().unwrap_or("").split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

pub fn join_ids(v: &[u64]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
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
        self.exec("CREATE TABLE IF NOT EXISTS agents (guild_id INTEGER PRIMARY KEY, config TEXT NOT NULL)", &[]).await?;
        self.exec("CREATE TABLE IF NOT EXISTS panels (id INTEGER PRIMARY KEY AUTOINCREMENT, guild_id INTEGER NOT NULL, config TEXT NOT NULL)", &[]).await?;
        self.add_columns(
            "tickets",
            &[
                ("escalated", "INTEGER NOT NULL DEFAULT 0"),
                ("staff_roles", "TEXT NOT NULL DEFAULT ''"),
                ("ping_roles", "TEXT NOT NULL DEFAULT ''"),
                ("instructions", "TEXT NOT NULL DEFAULT ''"),
                ("claimed_by", "INTEGER NOT NULL DEFAULT 0"),
                ("priority", "TEXT NOT NULL DEFAULT ''"),
                ("rating", "INTEGER NOT NULL DEFAULT 0"),
                ("rating_comment", "TEXT NOT NULL DEFAULT ''"),
                ("last_activity", "INTEGER NOT NULL DEFAULT 0"),
                ("warned", "INTEGER NOT NULL DEFAULT 0"),
                ("first_reply_at", "INTEGER"),
                ("human", "INTEGER NOT NULL DEFAULT 0"),
                ("closed_by", "INTEGER NOT NULL DEFAULT 0"),
                ("close_reason", "TEXT NOT NULL DEFAULT ''"),
            ],
        )
        .await?;
        self.add_columns(
            "settings",
            &[
                ("autoclose_hours", "INTEGER NOT NULL DEFAULT 0"),
                ("close_on_leave", "INTEGER NOT NULL DEFAULT 1"),
                ("ratings", "INTEGER NOT NULL DEFAULT 1"),
            ],
        )
        .await?;
        self.exec("CREATE INDEX IF NOT EXISTS idx_tickets_guild ON tickets(guild_id, status)", &[]).await?;
        Ok(())
    }

    async fn add_columns(&self, table: &str, cols: &[(&str, &str)]) -> Result<()> {
        let have = self.rows(&format!("PRAGMA table_info({table})"), &[]).await?;
        for (name, def) in cols {
            if !have.iter().any(|c| c[1].as_str() == Some(name)) {
                self.exec(&format!("ALTER TABLE {table} ADD COLUMN {name} {def}"), &[]).await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_ticket(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        subject: &str,
        staff_roles: &[u64],
        ping_roles: &[u64],
        instructions: &str,
    ) -> Result<i64> {
        let t = now();
        let r = self
            .exec(
                "INSERT INTO tickets (guild_id, channel_id, user_id, subject, status, created_at, last_activity, staff_roles, ping_roles, instructions) VALUES (?,?,?,?,?,?,?,?,?,?)",
                &[
                    json!(guild_id),
                    json!(channel_id),
                    json!(user_id),
                    json!(subject),
                    json!("open"),
                    json!(t),
                    json!(t),
                    json!(join_ids(staff_roles)),
                    json!(join_ids(ping_roles)),
                    json!(instructions),
                ],
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
            guild_id: v[5].as_i64().unwrap_or(0),
            created_at: v[6].as_i64().unwrap_or(0),
            closed_at: v[7].as_i64().unwrap_or(0),
            staff_roles: ids(&v[8]),
            ping_roles: ids(&v[9]),
            instructions: v[10].as_str().unwrap_or("").into(),
            claimed_by: v[11].as_i64().unwrap_or(0),
            priority: v[12].as_str().unwrap_or("").into(),
            escalated: v[13].as_i64() == Some(1),
            rating: v[14].as_i64().unwrap_or(0),
            rating_comment: v[15].as_str().unwrap_or("").into(),
            last_activity: v[16].as_i64().unwrap_or(0),
            warned: v[17].as_i64() == Some(1),
            closed_by: v[18].as_i64().unwrap_or(0),
        }
    }

    async fn tickets_where(&self, cond: &str, params: &[Value]) -> Result<Vec<Ticket>> {
        let rows = self.rows(&format!("SELECT {TCOLS} FROM tickets WHERE {cond}"), params).await?;
        Ok(rows.iter().map(Self::ticket).collect())
    }

    pub async fn by_channel(&self, channel_id: u64) -> Result<Option<Ticket>> {
        Ok(self.tickets_where("channel_id = ?", &[json!(channel_id)]).await?.into_iter().next())
    }

    pub async fn by_id(&self, id: i64) -> Result<Option<Ticket>> {
        Ok(self.tickets_where("id = ?", &[json!(id)]).await?.into_iter().next())
    }

    pub async fn open_ticket_for_user(&self, guild_id: u64, user_id: u64) -> Result<Option<Ticket>> {
        Ok(self
            .tickets_where("guild_id = ? AND user_id = ? AND status = 'open' ORDER BY id DESC LIMIT 1", &[json!(guild_id), json!(user_id)])
            .await?
            .into_iter()
            .next())
    }

    pub async fn all_open(&self) -> Result<Vec<Ticket>> {
        self.tickets_where("status = 'open' ORDER BY guild_id LIMIT 5000", &[]).await
    }

    pub async fn close_ticket(&self, ticket_id: i64) -> Result<()> {
        self.close_ticket_by(ticket_id, 0, "").await
    }

    pub async fn close_ticket_by(&self, ticket_id: i64, by: u64, reason: &str) -> Result<()> {
        self.exec(
            "UPDATE tickets SET status = 'closed', closed_at = ?, closed_by = ?, close_reason = ? WHERE id = ?",
            &[json!(now()), json!(by), json!(reason), json!(ticket_id)],
        )
        .await?;
        Ok(())
    }

    pub async fn set_claim(&self, ticket_id: i64, user: u64) -> Result<()> {
        self.exec("UPDATE tickets SET claimed_by = ? WHERE id = ?", &[json!(user), json!(ticket_id)]).await?;
        Ok(())
    }

    pub async fn set_priority(&self, ticket_id: i64, p: &str) -> Result<()> {
        self.exec("UPDATE tickets SET priority = ? WHERE id = ?", &[json!(p), json!(ticket_id)]).await?;
        Ok(())
    }

    /// New member/staff activity: resets the inactivity timer; `reply` marks the first real answer.
    pub async fn touch(&self, ticket_id: i64, reply: bool, human_staff: bool) -> Result<()> {
        let t = now();
        let mut sql = String::from("UPDATE tickets SET last_activity = ?, warned = 0");
        if reply {
            sql.push_str(", first_reply_at = COALESCE(first_reply_at, ?)");
        }
        if human_staff {
            sql.push_str(", human = 1");
        }
        sql.push_str(" WHERE id = ?");
        let params: Vec<Value> = if reply { vec![json!(t), json!(t), json!(ticket_id)] } else { vec![json!(t), json!(ticket_id)] };
        self.exec(&sql, &params).await?;
        Ok(())
    }

    pub async fn set_warned(&self, ticket_id: i64) -> Result<()> {
        self.exec("UPDATE tickets SET warned = 1 WHERE id = ?", &[json!(ticket_id)]).await?;
        Ok(())
    }

    /// Only the first rating counts. Returns false if already rated.
    pub async fn rate(&self, ticket_id: i64, stars: i64) -> Result<bool> {
        let r = self
            .exec("UPDATE tickets SET rating = ? WHERE id = ? AND rating = 0", &[json!(stars), json!(ticket_id)])
            .await?;
        Ok(r["changes"].as_i64().unwrap_or(1) > 0)
    }

    pub async fn rate_comment(&self, ticket_id: i64, comment: &str) -> Result<()> {
        self.exec("UPDATE tickets SET rating_comment = ? WHERE id = ?", &[json!(comment), json!(ticket_id)]).await?;
        Ok(())
    }

    pub async fn panels(&self, guild_id: u64) -> Result<Vec<crate::panels::Panel>> {
        let rows = self.rows("SELECT id, config FROM panels WHERE guild_id = ? ORDER BY id", &[json!(guild_id)]).await?;
        Ok(rows
            .iter()
            .map(|r| {
                let mut p = crate::panels::Panel::parse(r[1].as_str().unwrap_or("{}"));
                p.id = r[0].as_i64().unwrap_or(0);
                p
            })
            .collect())
    }

    pub async fn panel(&self, guild_id: u64, id: i64) -> Result<Option<crate::panels::Panel>> {
        let rows = self.rows("SELECT id, config FROM panels WHERE guild_id = ? AND id = ?", &[json!(guild_id), json!(id)]).await?;
        Ok(rows.first().map(|r| {
            let mut p = crate::panels::Panel::parse(r[1].as_str().unwrap_or("{}"));
            p.id = r[0].as_i64().unwrap_or(0);
            p
        }))
    }

    /// Insert when `p.id == 0`, else update. Returns the id.
    pub async fn save_panel(&self, guild_id: u64, p: &crate::panels::Panel) -> Result<i64> {
        let cfg = serde_json::to_string(p)?;
        if p.id == 0 {
            let r = self.exec("INSERT INTO panels (guild_id, config) VALUES (?,?)", &[json!(guild_id), json!(cfg)]).await?;
            Ok(r["lastInsertRowid"].as_i64().unwrap_or(0))
        } else {
            self.exec("UPDATE panels SET config = ? WHERE id = ? AND guild_id = ?", &[json!(cfg), json!(p.id), json!(guild_id)]).await?;
            Ok(p.id)
        }
    }

    pub async fn delete_panel(&self, guild_id: u64, id: i64) -> Result<()> {
        self.exec("DELETE FROM panels WHERE id = ? AND guild_id = ?", &[json!(id), json!(guild_id)]).await?;
        Ok(())
    }

    pub async fn stats(&self, guild_id: u64, since: i64) -> Result<Value> {
        let g = json!(guild_id);
        let sn = json!(since);
        let p0: Vec<Value> = vec![g.clone(), sn.clone()];
        let p1: Vec<Value> = vec![g.clone(), sn.clone()];
        let p2: Vec<Value> = vec![g.clone(), sn.clone(), g.clone(), sn.clone()];
        let p3: Vec<Value> = vec![g.clone(), sn.clone()];
        let p4: Vec<Value> = vec![g.clone()];
        let (daily, totals, staff, ratings, open) = tokio::join!(
            self.rows(
                "SELECT created_at / 86400 AS d, COUNT(*), SUM(status = 'closed') FROM tickets WHERE guild_id = ? AND created_at >= ? GROUP BY d ORDER BY d",
                &p0
            ),
            self.rows(
                "SELECT COUNT(*), SUM(status = 'closed'),
                    AVG(CASE WHEN first_reply_at IS NOT NULL THEN first_reply_at - created_at END),
                    AVG(CASE WHEN status = 'closed' AND closed_at IS NOT NULL THEN closed_at - created_at END),
                    SUM(status = 'closed' AND escalated = 0 AND human = 0 AND claimed_by = 0),
                    SUM(escalated = 1)
                 FROM tickets WHERE guild_id = ? AND created_at >= ?",
                &p1
            ),
            self.rows(
                "SELECT u, SUM(c), SUM(cl) FROM (
                    SELECT claimed_by AS u, 1 AS c, 0 AS cl FROM tickets WHERE guild_id = ? AND created_at >= ? AND claimed_by != 0
                    UNION ALL
                    SELECT closed_by AS u, 0 AS c, 1 AS cl FROM tickets WHERE guild_id = ? AND created_at >= ? AND closed_by != 0 AND status = 'closed'
                 ) GROUP BY u ORDER BY SUM(c) + SUM(cl) DESC LIMIT 15",
                &p2
            ),
            self.rows(
                "SELECT rating, COUNT(*) FROM tickets WHERE guild_id = ? AND created_at >= ? AND rating > 0 GROUP BY rating",
                &p3
            ),
            self.rows("SELECT priority, COUNT(*) FROM tickets WHERE guild_id = ? AND status = 'open' GROUP BY priority", &p4),
        );
        let t = totals?;
        let t = t.first().cloned().unwrap_or_default();
        Ok(json!({
            "daily": daily?.iter().map(|r| json!({"day": r[0], "opened": r[1], "closed": r[2]})).collect::<Vec<_>>(),
            "opened": t[0], "closed": t[1],
            "first_reply_secs": t[2], "resolve_secs": t[3],
            "ai_solved": t[4], "escalated": t[5],
            "staff": staff?.iter().map(|r| json!({"user_id": r[0].as_i64().unwrap_or(0).to_string(), "claimed": r[1], "closed": r[2]})).collect::<Vec<_>>(),
            "ratings": ratings?.iter().map(|r| json!({"stars": r[0], "count": r[1]})).collect::<Vec<_>>(),
            "open_by_priority": open?.iter().map(|r| json!({"priority": r[0], "count": r[1]})).collect::<Vec<_>>(),
        }))
    }

    pub async fn settings(&self, guild_id: u64) -> Result<Option<Settings>> {
        let rows = self
            .rows(
                "SELECT guild_id, ticket_category_id, mod_role_id, log_channel_id, ai_enabled, autoclose_hours, close_on_leave, ratings FROM settings WHERE guild_id = ?",
                &[json!(guild_id)],
            )
            .await?;
        Ok(rows.first().map(|r| Settings {
            guild_id: r[0].as_i64().unwrap_or(0),
            ticket_category_id: r[1].as_i64().unwrap_or(0),
            mod_role_id: r[2].as_i64().unwrap_or(0),
            log_channel_id: r[3].as_i64(),
            ai_enabled: r[4].as_i64() != Some(0),
            autoclose_hours: r[5].as_i64().unwrap_or(0),
            close_on_leave: r[6].as_i64() != Some(0),
            ratings: r[7].as_i64() != Some(0),
        }))
    }

    pub async fn upsert_settings(&self, s: &Settings) -> Result<()> {
        self.exec(
            "INSERT INTO settings (guild_id, ticket_category_id, mod_role_id, log_channel_id, ai_enabled, autoclose_hours, close_on_leave, ratings)
             VALUES (?,?,?,?,?,?,?,?)
             ON CONFLICT(guild_id) DO UPDATE SET ticket_category_id=excluded.ticket_category_id,
                mod_role_id=excluded.mod_role_id, log_channel_id=excluded.log_channel_id, ai_enabled=excluded.ai_enabled,
                autoclose_hours=excluded.autoclose_hours, close_on_leave=excluded.close_on_leave, ratings=excluded.ratings",
            &[
                json!(s.guild_id),
                json!(s.ticket_category_id),
                json!(s.mod_role_id),
                match s.log_channel_id { Some(v) => json!(v), None => Value::Null },
                json!(s.ai_enabled as i64),
                json!(s.autoclose_hours),
                json!(s.close_on_leave as i64),
                json!(s.ratings as i64),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_tickets(&self, guild_id: u64, limit: i64) -> Result<Vec<Ticket>> {
        self.tickets_where("guild_id = ? ORDER BY id DESC LIMIT ?", &[json!(guild_id), json!(limit)]).await
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

    pub async fn log_message(&self, ticket_id: i64, author: &str, author_id: u64, content: &str, is_staff: bool) -> Result<i64> {
        self.log_kind(ticket_id, author, author_id, content, is_staff as i64).await
    }

    /// is_staff: 0 member, 1 staff/bot, 2 private staff note (hidden from the AI history and members).
    pub async fn log_kind(&self, ticket_id: i64, author: &str, author_id: u64, content: &str, kind: i64) -> Result<i64> {
        let r = self
            .exec(
                "INSERT INTO messages (ticket_id, author, author_id, content, is_staff, created_at) VALUES (?,?,?,?,?,?)",
                &[json!(ticket_id), json!(author), json!(author_id), json!(content), json!(kind), json!(now())],
            )
            .await?;
        Ok(r["lastInsertRowid"].as_i64().unwrap_or(0))
    }

    /// Id of the newest member message in a ticket.
    pub async fn last_member_msg(&self, ticket_id: i64) -> Result<i64> {
        let rows = self.rows("SELECT MAX(id) FROM messages WHERE ticket_id = ? AND is_staff = 0", &[json!(ticket_id)]).await?;
        Ok(rows.first().and_then(|r| r[0].as_i64()).unwrap_or(0))
    }

    /// One line per earlier ticket of this member.
    pub async fn user_tickets(&self, guild_id: u64, user_id: u64, except: i64) -> Result<Vec<String>> {
        let rows = self
            .rows(
                "SELECT id, subject, status, created_at, rating, close_reason FROM tickets WHERE guild_id = ? AND user_id = ? AND id != ? ORDER BY id DESC LIMIT 10",
                &[json!(guild_id), json!(user_id), json!(except)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let mut line = format!(
                    "#{} {} ({}, opened <t:{}:D>)",
                    r[0].as_i64().unwrap_or(0),
                    r[1].as_str().unwrap_or(""),
                    r[2].as_str().unwrap_or(""),
                    r[3].as_i64().unwrap_or(0)
                );
                if let Some(n) = r[4].as_i64().filter(|n| *n > 0) {
                    line.push_str(&format!(", rated {n}/5"));
                }
                if let Some(c) = r[5].as_str().filter(|c| !c.is_empty()) {
                    line.push_str(&format!(", closed: {c}"));
                }
                line
            })
            .collect())
    }

    pub async fn set_subject(&self, ticket_id: i64, subject: &str) -> Result<()> {
        self.exec("UPDATE tickets SET subject = ? WHERE id = ?", &[json!(subject), json!(ticket_id)]).await?;
        Ok(())
    }

    /// (role, content) pairs for the AI; bot replies are `assistant`.
    pub async fn agent_config(&self, guild_id: u64) -> Result<Option<String>> {
        let rows = self.rows("SELECT config FROM agents WHERE guild_id = ?", &[json!(guild_id)]).await?;
        Ok(rows.first().and_then(|r| r[0].as_str().map(str::to_string)))
    }
    pub async fn save_agent_config(&self, guild_id: u64, config: &str) -> Result<()> {
        self.exec(
            "INSERT INTO agents (guild_id, config) VALUES (?,?) ON CONFLICT(guild_id) DO UPDATE SET config=excluded.config",
            &[json!(guild_id), json!(config)],
        )
        .await?;
        Ok(())
    }
    pub async fn set_escalated(&self, ticket_id: i64) -> Result<()> {
        self.exec("UPDATE tickets SET escalated = 1 WHERE id = ?", &[json!(ticket_id)]).await?;
        Ok(())
    }
    pub async fn is_escalated(&self, ticket_id: i64) -> Result<bool> {
        let rows = self.rows("SELECT escalated FROM tickets WHERE id = ?", &[json!(ticket_id)]).await?;
        Ok(rows.first().and_then(|r| r[0].as_i64()) == Some(1))
    }
    /// A human staff member (not the bot, not the ticket opener) has written in this ticket.
    pub async fn staff_replied(&self, ticket_id: i64, bot_id: u64, opener_id: u64) -> Result<bool> {
        let rows = self
            .rows(
                "SELECT 1 FROM messages WHERE ticket_id = ? AND is_staff = 1 AND author_id != ? AND author_id != ? LIMIT 1",
                &[json!(ticket_id), json!(bot_id), json!(opener_id)],
            )
            .await?;
        Ok(!rows.is_empty())
    }
    pub async fn history(&self, ticket_id: i64, limit: i64) -> Result<Vec<(String, String)>> {
        let rows = self
            .rows(
                "SELECT content, is_staff FROM (SELECT id, content, is_staff FROM messages WHERE ticket_id = ? AND is_staff != 2 ORDER BY id DESC LIMIT ?) ORDER BY id",
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
        self.transcript_for(ticket_id, true).await
    }

    pub async fn transcript_for(&self, ticket_id: i64, include_notes: bool) -> Result<String> {
        let sql = if include_notes {
            "SELECT author, content, created_at FROM messages WHERE ticket_id = ? ORDER BY id"
        } else {
            "SELECT author, content, created_at FROM messages WHERE ticket_id = ? AND is_staff != 2 ORDER BY id"
        };
        let rows = self.rows(sql, &[json!(ticket_id)]).await?;
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
