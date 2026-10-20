use crate::Data;
use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde_json::{json, Value};
use crate::agent::{AgentConfig, McpServer, TicketInfo};
use crate::panels::Panel;
use tower_http::services::ServeDir;

const MANAGE_GUILD: i64 = 0x20;

#[derive(Clone)]
struct Web {
    data: std::sync::Arc<Data>,
    client_id: String,
    client_secret: String,
    base_url: String,
    bot_token: String,
    http: reqwest::Client,
    bot: std::sync::Arc<poise::serenity_prelude::Http>,
}

impl Web {
    async fn raw_session(&self, headers: &HeaderMap) -> Option<(i64, String, String)> {
        let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
        let token = cookie.split(';').find_map(|c| {
            let c = c.trim();
            c.strip_prefix("tk_session=").map(str::to_string)
        })?;
        self.data.db.session(&token).await.ok()?
    }

    async fn session(&self, headers: &HeaderMap) -> Option<(i64, String, Vec<i64>)> {
        let (user_id, name, guilds_json) = self.raw_session(headers).await?;
        let guilds = parse_guilds(&guilds_json).iter().filter_map(gid).collect();
        Some((user_id, name, guilds))
    }

    async fn discord_list(&self, path: &str) -> Vec<Value> {
        match self
            .http
            .get(format!("https://discord.com/api/v10/{path}"))
            .header(header::AUTHORIZATION, format!("Bot {}", self.bot_token))
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => vec![],
        }
    }

    /// id -> display name, via the bot token, cached for an hour.
    async fn user_names(&self, ids: &[String]) -> serde_json::Map<String, Value> {
        static CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (i64, String)>>> =
            std::sync::LazyLock::new(Default::default);
        let mut out = serde_json::Map::new();
        let mut missing = vec![];
        {
            let c = CACHE.lock().unwrap();
            for id in ids {
                match c.get(id) {
                    Some((at, name)) if now_secs() - at < 3600 => {
                        out.insert(id.clone(), json!(name));
                    }
                    _ => missing.push(id.clone()),
                }
            }
        }
        let fetched = futures::future::join_all(missing.iter().take(20).map(|id| async move {
            let r = self
                .http
                .get(format!("https://discord.com/api/v10/users/{id}"))
                .header(header::AUTHORIZATION, format!("Bot {}", self.bot_token))
                .send()
                .await
                .ok()?;
            let v: Value = r.json().await.ok()?;
            let name = v["global_name"].as_str().or(v["username"].as_str())?.to_string();
            Some((id.clone(), name))
        }))
        .await;
        let mut c = CACHE.lock().unwrap();
        for (id, name) in fetched.into_iter().flatten() {
            c.insert(id.clone(), (now_secs(), name.clone()));
            out.insert(id, json!(name));
        }
        out
    }

    /// Server config (settings, AI agent, panels) as JSON.
    async fn snapshot(&self, guild_id: u64) -> Result<Value> {
        let (s, a, p) = tokio::join!(self.data.db.settings(guild_id), self.data.db.agent_config(guild_id), self.data.db.panels(guild_id));
        Ok(json!({
            "settings": s?.map(|s| json!({
                "ticket_category_id": s.ticket_category_id.to_string(), "mod_role_id": s.mod_role_id.to_string(),
                "log_channel_id": s.log_channel_id.map(|v| v.to_string()), "ai_enabled": s.ai_enabled,
                "autoclose_hours": s.autoclose_hours, "close_on_leave": s.close_on_leave, "ratings": s.ratings,
            })),
            "agent": a?.map(|c| AgentConfig::parse(&c)),
            "panels": p?,
        }))
    }

    /// Old backups can point at images/docs that were cleaned up since; blank those instead of restoring dead links.
    async fn drop_missing(&self, guild_id: u64, store: &crate::archive::Store, mut v: Value) -> Value {
        let docs: std::collections::BTreeSet<String> =
            store.list(&format!("knowledge/{guild_id}/")).await.map(|l| l.into_iter().map(|(n, _)| n).collect()).unwrap_or_default();
        if let Some(k) = v["agent"]["knowledge"].as_array_mut() {
            k.retain(|d| d["key"].as_str().is_some_and(|key| docs.contains(key)));
        }
        let mut missing = std::collections::BTreeSet::new();
        for r in crate::gc::refs(&v.to_string()) {
            if let Some(h) = r.strip_prefix("m:") {
                if matches!(self.data.db.file(h).await, Ok(None)) {
                    missing.insert(h.to_string());
                }
            }
        }
        fn walk(v: &mut Value, missing: &std::collections::BTreeSet<String>) {
            match v {
                Value::String(s) if missing.iter().any(|h| s.contains(&format!("/media/{h}"))) => s.clear(),
                Value::Array(a) => a.iter_mut().for_each(|x| walk(x, missing)),
                Value::Object(o) => o.values_mut().for_each(|x| walk(x, missing)),
                _ => {}
            }
        }
        if !missing.is_empty() {
            walk(&mut v, &missing);
        }
        v
    }

    async fn refs(&self, guild_id: u64) -> Option<std::collections::BTreeSet<String>> {
        crate::gc::guild_refs(&self.data.db, guild_id).await.ok()
    }

    /// After a config change: deletes images/docs the guild dropped that nothing else uses.
    fn release(&self, guild_id: u64, before: Option<std::collections::BTreeSet<String>>) {
        let (Some(before), Some(store)) = (before, self.data.store.clone()) else { return };
        let db = self.data.db.clone();
        tokio::spawn(async move { crate::gc::release(&db, &store, guild_id, before).await });
    }

    /// Saves the current config before a change. Keeps the last 50 per server.
    async fn backup(&self, guild_id: u64, what: &str) {
        let Some(store) = self.data.store.clone() else { return };
        let run = async {
            let v = self.snapshot(guild_id).await?;
            let packed = zstd::bulk::compress(&serde_json::to_vec(&v)?, 19)?;
            let key = crate::archive::backup_key(guild_id, now_millis());
            let size = packed.len();
            store.put(&key, packed).await?;
            self.data.db.add_backup(guild_id, &key, what, size).await?;
            for (id, key, _, _, _) in self.data.db.backups(guild_id).await?.into_iter().skip(50) {
                if store.delete(&key).await.is_ok() {
                    self.data.db.delete_backup(id).await.ok();
                }
            }
            anyhow::Ok(())
        };
        if let Err(e) = run.await {
            tracing::warn!("backup for {guild_id} failed: {e:#}");
        }
    }

    async fn share_ok(&self, ticket_id: i64, token: &str) -> Option<(crate::db::Ticket, std::sync::Arc<crate::archive::Store>)> {
        if token.len() < 20 {
            return None;
        }
        let real = self.data.db.share_token(ticket_id).await.ok()?;
        if real.is_empty() || real.len() != token.len() || real.bytes().zip(token.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
            return None;
        }
        Some((self.data.db.by_id(ticket_id).await.ok()??, self.data.store.clone()?))
    }

    fn require_guild(&self, session: &Option<(i64, String, Vec<i64>)>, guild_id: u64) -> Result<(), StatusCode> {
        let Some((_, _, guilds)) = session else { return Err(StatusCode::UNAUTHORIZED) };
        if guilds.contains(&(guild_id as i64)) {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

pub fn router(data: std::sync::Arc<Data>) -> Result<Router> {
    let web = Web {
        data,
        client_id: std::env::var("DISCORD_CLIENT_ID")?,
        client_secret: std::env::var("DISCORD_CLIENT_SECRET")?,
        base_url: std::env::var("DASHBOARD_BASE_URL").unwrap_or_else(|_| "http://localhost:8420".into()),
        bot_token: std::env::var("DISCORD_TOKEN")?,
        http: reqwest::Client::new(),
        bot: std::sync::Arc::new(poise::serenity_prelude::Http::new(&std::env::var("DISCORD_TOKEN")?)),
    };

    Ok(Router::new()
        .route("/api/auth/discord", get(auth_start))
        .route("/api/auth/callback", get(auth_callback))
        .route("/api/logout", get(logout))
        .route("/api/me", get(me))
        .route("/api/guilds/{guild_id}", get(get_guild))
        .route("/api/guilds/{guild_id}/settings", put(put_settings))
        .route("/api/guilds/{guild_id}/tickets", get(list_tickets))
        .route("/api/guilds/{guild_id}/agent", get(get_agent).put(put_agent))
        .route("/api/guilds/{guild_id}/agent/test", post(test_agent))
        .route("/api/guilds/{guild_id}/agent/mcp-test", post(test_mcp))
        .route("/api/tickets/{ticket_id}/transcript", get(transcript))
        .route("/api/tickets/{ticket_id}", get(ticket_detail))
        .route("/api/tickets/{ticket_id}/files/{hash}", get(ticket_file))
        .route("/api/guilds/{guild_id}/panels", get(list_panels).post(save_panel))
        .route("/api/guilds/{guild_id}/panels/{panel_id}", put(save_panel_id).delete(delete_panel))
        .route("/api/guilds/{guild_id}/panels/{panel_id}/send", post(send_panel))
        .route("/api/guilds/{guild_id}/stats", get(stats))
        .route("/api/guilds/{guild_id}/media", post(upload_media).layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)))
        .route("/media/{file}", get(media))
        .route("/api/guilds/{guild_id}/knowledge", post(upload_knowledge).layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024)))
        .route("/api/guilds/{guild_id}/backups", get(list_backups))
        .route("/api/guilds/{guild_id}/backups/{backup_id}/restore", post(restore_backup))
        .route("/api/guilds/{guild_id}/export", get(export_status).post(start_export))
        .route("/share/{ticket_id}/{token}", get(share_page))
        .route("/share/{ticket_id}/{token}/files/{hash}", get(share_file))
        .fallback_service(ServeDir::new("dashboard/dist").fallback(axum::routing::get(spa_index)))
        .with_state(web))
}

async fn auth_start(State(w): State<Web>) -> Redirect {
    let url = format!(
        "https://discord.com/oauth2/authorize?client_id={}&redirect_uri={}&response_type=code&scope=identify%20guilds",
        w.client_id,
        urlencoding(&format!("{}/api/auth/callback", w.base_url))
    );
    Redirect::to(&url)
}

#[derive(serde::Deserialize)]
struct Cb {
    code: String,
}

async fn auth_callback(State(w): State<Web>, Query(q): Query<Cb>) -> Response {
    let token_resp = w
        .http
        .post("https://discord.com/api/v10/oauth2/token")
        .form(&[
            ("client_id", w.client_id.as_str()),
            ("client_secret", w.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", q.code.as_str()),
            ("redirect_uri", &format!("{}/api/auth/callback", w.base_url)),
        ])
        .send()
        .await;
    let Ok(tok) = token_resp else {
        return (StatusCode::BAD_GATEWAY, "discord token exchange failed").into_response();
    };
    let tok: Value = match tok.json().await {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_GATEWAY, "bad token response").into_response(),
    };
    let Some(access) = tok["access_token"].as_str() else {
        return (StatusCode::UNAUTHORIZED, "oauth denied").into_response();
    };

    let user: Value = match w
        .http
        .get("https://discord.com/api/v10/users/@me")
        .bearer_auth(access)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(_) => return (StatusCode::BAD_GATEWAY, "user fetch failed").into_response(),
    };
    let guilds: Vec<Value> = match w
        .http
        .get("https://discord.com/api/v10/users/@me/guilds")
        .bearer_auth(access)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(_) => vec![],
    };

    // keep guilds where user can Manage Server AND the bot is present
    let bot_guild_list = w.discord_list("users/@me/guilds?limit=200").await;
    let bot_guilds: std::collections::HashSet<i64> = bot_guild_list
        .iter()
        .filter_map(|g| g["id"].as_str()?.parse().ok())
        .collect();

    let admin_guilds: Vec<Value> = guilds
        .iter()
        .filter(|g| {
            g["permissions"].as_str().and_then(|p| p.parse::<i64>().ok()).map(|p| p & MANAGE_GUILD != 0).unwrap_or(false)
        })
        .filter(|g| gid(g).is_some_and(|id| bot_guilds.contains(&id)))
        .map(|g| json!({"id": g["id"], "name": g["name"], "icon": g["icon"]}))
        .collect();

    let user_id: i64 = user["id"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
    let user_name = user["username"].as_str().unwrap_or("").to_string();
    let session_token = random_token();
    let expires = now_secs() + 30 * 86400;
    if let Err(e) = w.data.db.save_session(&session_token, user_id as u64, &user_name, &serde_json::to_string(&admin_guilds).unwrap(), expires).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let cookie = format!("tk_session={session_token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={}", 30 * 86400);
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

async fn me(State(w): State<Web>, headers: HeaderMap) -> Response {
    match w.raw_session(&headers).await {
        Some((user_id, name, guilds_json)) => {
            Json(json!({"user": {"id": user_id.to_string(), "name": name}, "guilds": parse_guilds(&guilds_json)})).into_response()
        }
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn get_guild(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let meta = match w.raw_session(&headers).await {
        Some((_, _, j)) => parse_guilds(&j).into_iter().find(|g| gid(g) == Some(guild_id as i64)),
        None => None,
    }
    .unwrap_or_default();
    let (ch_path, role_path) = (format!("guilds/{guild_id}/channels"), format!("guilds/{guild_id}/roles"));
    let (s, channels, roles) = tokio::join!(w.data.db.settings(guild_id), w.discord_list(&ch_path), w.discord_list(&role_path));
    let s = s.ok().flatten();
    let mut channels: Vec<Value> = channels
        .iter()
        .filter(|c| matches!(c["type"].as_i64(), Some(0 | 4 | 5)))
        .map(|c| json!({"id": c["id"], "name": c["name"], "type": c["type"], "position": c["position"]}))
        .collect();
    channels.sort_by_key(|c| c["position"].as_i64().unwrap_or(0));
    let mut roles: Vec<Value> = roles
        .iter()
        .filter(|r| r["id"].as_str() != Some(&guild_id.to_string()) && !r["managed"].as_bool().unwrap_or(false))
        .map(|r| json!({"id": r["id"], "name": r["name"], "color": r["color"], "position": r["position"]}))
        .collect();
    roles.sort_by_key(|r| -r["position"].as_i64().unwrap_or(0));
    Json(json!({
        "guild_id": guild_id.to_string(),
        "name": meta["name"],
        "icon": meta["icon"],
        "channels": channels,
        "roles": roles,
        "settings": s.map(|s| json!({
            "ticket_category_id": s.ticket_category_id.to_string(),
            "mod_role_id": s.mod_role_id.to_string(),
            "log_channel_id": s.log_channel_id.map(|v| v.to_string()),
            "ai_enabled": s.ai_enabled,
            "autoclose_hours": s.autoclose_hours,
            "close_on_leave": s.close_on_leave,
            "ratings": s.ratings,
        })),
    }))
    .into_response()
}

async fn put_settings(
    State(w): State<Web>,
    headers: HeaderMap,
    Path(guild_id): Path<u64>,
    Json(body): Json<Value>,
) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let s = crate::db::Settings {
        guild_id: guild_id as i64,
        ticket_category_id: body["ticket_category_id"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
        mod_role_id: body["mod_role_id"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
        log_channel_id: body["log_channel_id"].as_str().and_then(|v| v.parse().ok()),
        ai_enabled: body["ai_enabled"].as_bool().unwrap_or(true),
        autoclose_hours: body["autoclose_hours"].as_i64().unwrap_or(0).clamp(0, 24 * 30),
        close_on_leave: body["close_on_leave"].as_bool().unwrap_or(true),
        ratings: body["ratings"].as_bool().unwrap_or(true),
    };
    w.backup(guild_id, "settings").await;
    match w.data.db.upsert_settings(&s).await {
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_agent(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match w.data.db.agent_config(guild_id).await {
        Ok(c) => Json(json!({"config": c.map(|c| AgentConfig::parse(&c)).unwrap_or_default()})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn put_agent(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, Json(mut cfg): Json<AgentConfig>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    if let Err(e) = cfg.validate() {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    w.backup(guild_id, "AI agent").await;
    let before = w.refs(guild_id).await;
    match w.data.db.save_agent_config(guild_id, &serde_json::to_string(&cfg).unwrap_or_default()).await {
        Ok(_) => {
            w.release(guild_id, before);
            Json(json!({"ok": true, "config": cfg})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct TestBody {
    config: AgentConfig,
    messages: Vec<TestMsg>,
}
#[derive(serde::Deserialize)]
struct TestMsg {
    role: String,
    content: String,
}

/// Dashboard playground: runs the agent with unsaved config; escalate/close are only reported.
async fn test_agent(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, Json(mut body): Json<TestBody>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some((user_id, user_name, _)) = session else { return StatusCode::UNAUTHORIZED.into_response() };
    if let Err(e) = body.config.validate() {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    let Some(last) = body.messages.pop().filter(|m| m.role == "user") else {
        return (StatusCode::BAD_REQUEST, "last message must be from the user").into_response();
    };
    let history: Vec<(String, String)> = body
        .messages
        .into_iter()
        .rev()
        .take(20)
        .rev()
        .map(|m| (if m.role == "assistant" { "assistant" } else { "user" }.to_string(), m.content))
        .collect();
    let guild_name = match w.raw_session(&headers).await {
        Some((_, _, j)) => parse_guilds(&j).into_iter().find(|g| gid(g) == Some(guild_id as i64)).and_then(|g| g["name"].as_str().map(str::to_string)),
        None => None,
    }
    .unwrap_or_default();
    let info = TicketInfo {
        guild_id,
        guild_name,
        user_id: user_id as u64,
        user_name,
        subject: "dashboard test".into(),
        opened_at: now_secs(),
        ..Default::default()
    };
    let knowledge = crate::archive::load_knowledge(w.data.store.as_deref(), &body.config.knowledge).await;
    match crate::agent::run(&w.data.ai, &body.config, &info, &knowledge, &history, &last.content).await {
        Ok(o) => Json(json!({"reply": o.text, "actions": o.actions, "tools": o.tools})).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("AI error: {e}")).into_response(),
    }
}

async fn test_mcp(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, Json(server): Json<McpServer>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match crate::agent::list_mcp_tools(&server).await {
        Ok(tools) => Json(json!({"tools": tools})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

async fn list_tickets(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match w.data.db.list_tickets(guild_id, 200).await {
        Ok(tickets) => Json(json!({
            "tickets": tickets.iter().map(ticket_json).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn transcript(State(w): State<Web>, headers: HeaderMap, Path(ticket_id): Path<i64>) -> Response {
    let Some((_, _, guilds)) = w.session(&headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // find the ticket's guild via a settings lookup through the channel? tickets have guild_id.
    let Ok(text) = w.data.db.transcript(ticket_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // authorize: check ticket's guild is in session guilds
    match w.data.db.ticket_guild(ticket_id).await {
        Ok(Some(gid)) if guilds.contains(&gid) => {}
        _ => return StatusCode::FORBIDDEN.into_response(),
    }
    ([(header::CONTENT_TYPE, "text/plain")], text).into_response()
}

fn ticket_json(t: &crate::db::Ticket) -> Value {
    json!({
        "id": t.id,
        "channel_id": t.channel_id.to_string(),
        "user_id": t.user_id.to_string(),
        "subject": t.subject,
        "status": t.status,
        "priority": t.priority,
        "claimed_by": (t.claimed_by != 0).then(|| t.claimed_by.to_string()),
        "closed_by": (t.closed_by != 0).then(|| t.closed_by.to_string()),
        "escalated": t.escalated,
        "rating": t.rating,
        "rating_comment": t.rating_comment,
        "created_at": t.created_at,
        "closed_at": t.closed_at,
    })
}

async fn ticket_detail(State(w): State<Web>, headers: HeaderMap, Path(ticket_id): Path<i64>) -> Response {
    let Some((_, _, guilds)) = w.session(&headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match w.data.db.by_id(ticket_id).await {
        Ok(Some(t)) if guilds.contains(&t.guild_id) => {
            let mut v = ticket_json(&t);
            let ids: Vec<String> = [t.user_id, t.claimed_by, t.closed_by].iter().filter(|x| **x != 0).map(|x| x.to_string()).collect();
            v["users"] = json!(w.user_names(&ids).await);
            v["transcript"] = json!(w.data.db.transcript(ticket_id).await.unwrap_or_default());
            let token = w.data.db.share_token(ticket_id).await.unwrap_or_default();
            v["share_url"] = json!((!token.is_empty()).then(|| format!("{}/share/{ticket_id}/{token}", w.base_url)));
            Json(v).into_response()
        }
        Ok(Some(_)) => StatusCode::FORBIDDEN.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Streams an archived attachment back in its original form (zstd is undone, lossless WebP is served as is).
async fn ticket_file(State(w): State<Web>, headers: HeaderMap, Path((ticket_id, hash)): Path<(i64, String)>) -> Response {
    let Some((_, _, guilds)) = w.session(&headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match w.data.db.by_id(ticket_id).await {
        Ok(Some(t)) if guilds.contains(&t.guild_id) => {}
        _ => return StatusCode::NOT_FOUND.into_response(),
    }
    if !w.data.db.ticket_has_file(ticket_id, &hash).await.unwrap_or(false) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = w.data.store.clone() else { return StatusCode::NOT_FOUND.into_response() };
    serve_archived(&store, &w, &hash).await
}

/// Dashboard image upload for embeds. PNG/BMP/TIFF become lossless WebP; JPEG/GIF/WebP stay as they are.
async fn upload_media(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, body: axum::body::Bytes) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some(store) = w.data.store.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Uploads are off right now").into_response();
    };
    let bytes = body.to_vec();
    if bytes.len() > 8 * 1024 * 1024 {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Images can be up to 8 MB").into_response();
    }
    let Ok(format) = image::guess_format(&bytes) else {
        return (StatusCode::BAD_REQUEST, "That isn't an image (PNG, JPEG, GIF or WebP)").into_response();
    };
    let (mime, ext) = match format {
        image::ImageFormat::Png => ("image/png", "png"),
        image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        image::ImageFormat::Gif => ("image/gif", "gif"),
        image::ImageFormat::WebP => ("image/webp", "webp"),
        image::ImageFormat::Bmp => ("image/bmp", "bmp"),
        image::ImageFormat::Tiff => ("image/tiff", "tiff"),
        _ => return (StatusCode::BAD_REQUEST, "Use PNG, JPEG, GIF or WebP").into_response(),
    };
    let hash = crate::archive::sha256(&bytes);
    let existing = w.data.db.file(&hash).await.ok().flatten().filter(|(k, _, _)| k.starts_with("media/"));
    let out_ext = match existing {
        Some((key, _, _)) => key.rsplit('.').next().unwrap_or(ext).to_string(),
        None => {
            let orig = bytes.len();
            let (packed, enc) = match tokio::task::spawn_blocking(move || {
                let (p, e) = crate::archive::compress(&bytes, &format!("x.{ext}"), mime);
                // embeds must stay directly loadable: only lossless WebP or the original bytes
                if e == crate::archive::Enc::Zstd { (bytes, crate::archive::Enc::Raw) } else { (p, e) }
            })
            .await
            {
                Ok(v) => v,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            };
            let (out_ext, out_mime) = if enc == crate::archive::Enc::Webp { ("webp", "image/webp") } else { (ext, mime) };
            let key = format!("media/{hash}.{out_ext}");
            let stored = packed.len();
            if let Err(e) = store.put(&key, packed).await {
                return (StatusCode::BAD_GATEWAY, e.to_string()).into_response();
            }
            if let Err(e) = w.data.db.add_file(&hash, &key, "raw", out_mime, orig, stored).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
            out_ext.to_string()
        }
    };
    Json(json!({"url": format!("{}/media/{hash}.{out_ext}", w.base_url)})).into_response()
}

static MEDIA_CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (String, axum::body::Bytes)>>> =
    std::sync::LazyLock::new(Default::default);

pub fn forget_media(hash: &str) {
    MEDIA_CACHE.lock().unwrap().remove(hash);
}

/// Public, immutable embed images (content-addressed), cached in memory and at Cloudflare.
async fn media(State(w): State<Web>, Path(file): Path<String>) -> Response {
    let Some((hash, _)) = file.split_once('.') else { return StatusCode::NOT_FOUND.into_response() };
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let cached = MEDIA_CACHE.lock().unwrap().get(hash).cloned();
    let (mime, bytes) = match cached {
        Some(v) => v,
        None => {
            let (Some(store), Ok(Some((key, _, mime)))) = (w.data.store.clone(), w.data.db.file(hash).await) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if !key.starts_with("media/") {
                return StatusCode::NOT_FOUND.into_response();
            }
            let Ok(b) = store.get(&key).await else { return StatusCode::BAD_GATEWAY.into_response() };
            let v = (mime, axum::body::Bytes::from(b));
            let mut c = MEDIA_CACHE.lock().unwrap();
            if c.values().map(|(_, b)| b.len()).sum::<usize>() > 64 * 1024 * 1024 {
                c.clear();
            }
            c.insert(hash.to_string(), v.clone());
            v
        }
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response()
}

async fn upload_knowledge(
    State(w): State<Web>,
    headers: HeaderMap,
    Path(guild_id): Path<u64>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some(store) = w.data.store.clone() else { return (StatusCode::SERVICE_UNAVAILABLE, "Uploads are off right now").into_response() };
    let name: String = q.get("name").map(|n| n.trim().chars().take(100).collect()).unwrap_or_default();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing file name").into_response();
    }
    if body.len() > crate::archive::MAX_DOC {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Documents can be up to 5 MB").into_response();
    }
    let (bytes, n) = (body.to_vec(), name.clone());
    let text = match tokio::task::spawn_blocking(move || crate::archive::extract_text(&bytes, &n)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let hash = crate::archive::sha256(text.as_bytes());
    let key = format!("knowledge/{guild_id}/{hash}.txt.zst");
    let packed = match zstd::bulk::compress(text.as_bytes(), 19) {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Err(e) = store.put(&key, packed).await {
        return (StatusCode::BAD_GATEWAY, e.to_string()).into_response();
    }
    let chars = text.chars().count() as i64;
    crate::archive::cache_doc(&key, text);
    Json(json!({"doc": {"name": name, "key": key, "chars": chars}})).into_response()
}

async fn list_backups(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match w.data.db.backups(guild_id).await {
        Ok(b) => Json(json!({"backups": b.iter().map(|(id, _, what, size, at)| json!({"id": id, "what": what, "size": size, "at": at})).collect::<Vec<_>>()}))
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn restore_backup(State(w): State<Web>, headers: HeaderMap, Path((guild_id, backup_id)): Path<(u64, i64)>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some(store) = w.data.store.clone() else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    let Some((_, key, _, _, _)) = w.data.db.backups(guild_id).await.unwrap_or_default().into_iter().find(|b| b.0 == backup_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let v: Value = match store.get(&key).await.and_then(|b| crate::archive::decompress(b, crate::archive::Enc::Zstd)).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    w.backup(guild_id, "before restore").await;
    let before = w.refs(guild_id).await;
    let v = w.drop_missing(guild_id, &store, v).await;
    let run = async {
        if let Some(s) = v["settings"].as_object() {
            let id = |k: &str| s.get(k).and_then(|x| x.as_str()).and_then(|x| x.parse::<i64>().ok());
            w.data
                .db
                .upsert_settings(&crate::db::Settings {
                    guild_id: guild_id as i64,
                    ticket_category_id: id("ticket_category_id").unwrap_or(0),
                    mod_role_id: id("mod_role_id").unwrap_or(0),
                    log_channel_id: id("log_channel_id"),
                    ai_enabled: s.get("ai_enabled").and_then(|x| x.as_bool()).unwrap_or(true),
                    autoclose_hours: s.get("autoclose_hours").and_then(|x| x.as_i64()).unwrap_or(0),
                    close_on_leave: s.get("close_on_leave").and_then(|x| x.as_bool()).unwrap_or(true),
                    ratings: s.get("ratings").and_then(|x| x.as_bool()).unwrap_or(true),
                })
                .await?;
        }
        if v["agent"].is_object() {
            let mut cfg: AgentConfig = serde_json::from_value(v["agent"].clone()).unwrap_or_default();
            cfg.validate().map_err(|e| anyhow::anyhow!(e))?;
            w.data.db.save_agent_config(guild_id, &serde_json::to_string(&cfg)?).await?;
        }
        let panels: Vec<Panel> = serde_json::from_value(v["panels"].clone()).unwrap_or_default();
        w.data.db.restore_panels(guild_id, &panels).await?;
        anyhow::Ok(())
    };
    match run.await {
        Ok(_) => {
            w.release(guild_id, before);
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// guild -> (state, url, at)
static EXPORTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, (String, String, i64)>>> =
    std::sync::LazyLock::new(Default::default);

async fn export_status(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let e = EXPORTS.lock().unwrap().get(&guild_id).cloned();
    Json(match e {
        Some((state, url, at)) => json!({"state": state, "url": url, "at": at}),
        None => json!({"state": "none"}),
    })
    .into_response()
}

async fn start_export(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some(store) = w.data.store.clone() else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    {
        let mut ex = EXPORTS.lock().unwrap();
        if ex.get(&guild_id).is_some_and(|e| e.0 == "working") {
            return Json(json!({"state": "working"})).into_response();
        }
        ex.insert(guild_id, ("working".into(), String::new(), now_secs()));
    }
    tokio::spawn(async move {
        let result = build_export(&w, &store, guild_id).await;
        let entry = match result {
            Ok(url) => ("done".to_string(), url, now_secs()),
            Err(e) => {
                tracing::warn!("export {guild_id} failed: {e:#}");
                (format!("failed: {e}"), String::new(), now_secs())
            }
        };
        EXPORTS.lock().unwrap().insert(guild_id, entry);
    });
    Json(json!({"state": "working"})).into_response()
}

/// Zip with config, every ticket + messages, and archived files (original form), uploaded to storage; returns a 7-day link.
async fn build_export(w: &Web, store: &crate::archive::Store, guild_id: u64) -> Result<String> {
    use std::io::Write;
    let tickets = w.data.db.all_tickets(guild_id).await?;
    let mut docs = vec![];
    let mut hashes = std::collections::BTreeSet::new();
    for t in &tickets {
        let msgs = w.data.db.messages_json(t.id).await?;
        for m in &msgs {
            let c = m["content"].as_str().unwrap_or("");
            for part in c.split("kst:").skip(1) {
                let h: String = part.chars().take(64).collect();
                if h.len() == 64 {
                    hashes.insert(h);
                }
            }
        }
        docs.push(crate::archive::ticket_doc(t, msgs));
    }
    let config = w.snapshot(guild_id).await?;
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    let mut total = 0usize;
    for h in hashes {
        let Some((key, enc, mime)) = w.data.db.file(&h).await? else { continue };
        let Ok(b) = store.get(&key).await.and_then(|b| crate::archive::decompress(b, crate::archive::Enc::parse(&enc))) else { continue };
        total += b.len();
        if total > 400 * 1024 * 1024 {
            break;
        }
        let ext = if enc == "webp" { "webp".to_string() } else { mime.rsplit('/').next().unwrap_or("bin").replace("jpeg", "jpg").replace("plain", "txt") };
        files.push((format!("files/{h}.{ext}"), b));
    }
    let zip_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(9));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zw.start_file("config.json", deflate)?;
        zw.write_all(&serde_json::to_vec_pretty(&config)?)?;
        zw.start_file("tickets.json", deflate)?;
        zw.write_all(&serde_json::to_vec_pretty(&docs)?)?;
        for (name, b) in files {
            let opt = if [".webp", ".jpg", ".png", ".gif", ".mp4", ".zip"].iter().any(|e| name.ends_with(e)) { stored } else { deflate };
            zw.start_file(name, opt)?;
            zw.write_all(&b)?;
        }
        Ok(zw.finish()?.into_inner())
    })
    .await??;
    let key = format!("exports/{guild_id}/{}.zip", now_secs());
    store.put(&key, zip_bytes).await?;
    store.link(&key, 168).await
}

/// Public, unguessable transcript page linked from the log channel.
async fn share_page(State(w): State<Web>, Path((ticket_id, token)): Path<(i64, String)>) -> Response {
    let Some((t, store)) = w.share_ok(ticket_id, &token).await else { return StatusCode::NOT_FOUND.into_response() };
    match store.get(&crate::archive::transcript_key(t.guild_id, t.id)).await.and_then(|b| crate::archive::decompress(b, crate::archive::Enc::Zstd)) {
        Ok(html) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CONTENT_SECURITY_POLICY, "default-src 'none'; img-src 'self' https:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"),
                (header::CACHE_CONTROL, "private, max-age=3600"),
                (header::HeaderName::from_static("x-robots-tag"), "noindex"),
                (header::REFERRER_POLICY, "no-referrer"),
            ],
            html,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn share_file(State(w): State<Web>, Path((ticket_id, token, hash)): Path<(i64, String, String)>) -> Response {
    let Some((_, store)) = w.share_ok(ticket_id, &token).await else { return StatusCode::NOT_FOUND.into_response() };
    if hash.len() != 64 || !w.data.db.ticket_has_file(ticket_id, &hash).await.unwrap_or(false) {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_archived(&store, &w, &hash).await
}

async fn serve_archived(store: &crate::archive::Store, w: &Web, hash: &str) -> Response {
    let Ok(Some((key, enc, mime))) = w.data.db.file(hash).await else { return StatusCode::NOT_FOUND.into_response() };
    let enc = crate::archive::Enc::parse(&enc);
    let bytes = match store.get(&key).await.and_then(|b| crate::archive::decompress(b, enc)) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let ctype = if enc == crate::archive::Enc::Webp { "image/webp".to_string() } else { mime };
    let safe = ctype.starts_with("image/") && ctype != "image/svg+xml" || ctype.starts_with("video/") || ctype.starts_with("audio/") || ctype == "application/pdf";
    let ctype = if safe { ctype } else if ctype.starts_with("text/") { "text/plain; charset=utf-8".into() } else { "application/octet-stream".into() };
    (
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "private, max-age=86400".into()),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff".into()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox".into()),
        ],
        bytes,
    )
        .into_response()
}

async fn list_panels(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match w.data.db.panels(guild_id).await {
        Ok(p) => Json(json!({"panels": p, "blank": Panel::default()})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn save_panel(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, Json(mut p): Json<Panel>) -> Response {
    p.id = 0;
    upsert_panel(w, headers, guild_id, p).await
}

async fn save_panel_id(State(w): State<Web>, headers: HeaderMap, Path((guild_id, panel_id)): Path<(u64, i64)>, Json(mut p): Json<Panel>) -> Response {
    p.id = panel_id;
    upsert_panel(w, headers, guild_id, p).await
}

async fn upsert_panel(w: Web, headers: HeaderMap, guild_id: u64, mut p: Panel) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    if let Err(e) = p.validate() {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    let existing = w.data.db.panels(guild_id).await.unwrap_or_default();
    if p.id == 0 && existing.len() >= 25 {
        return (StatusCode::BAD_REQUEST, "You can have up to 25 panels").into_response();
    }
    if p.id != 0 && !existing.iter().any(|x| x.id == p.id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if existing.iter().any(|x| x.id != p.id && x.name.eq_ignore_ascii_case(&p.name)) {
        return (StatusCode::BAD_REQUEST, "Another panel already has that name").into_response();
    }
    w.backup(guild_id, "panels").await;
    let before = w.refs(guild_id).await;
    match w.data.db.save_panel(guild_id, &p).await {
        Ok(id) => {
            w.release(guild_id, before);
            p.id = id;
            Json(json!({"panel": p})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn delete_panel(State(w): State<Web>, headers: HeaderMap, Path((guild_id, panel_id)): Path<(u64, i64)>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    w.backup(guild_id, "panels").await;
    let before = w.refs(guild_id).await;
    match w.data.db.delete_panel(guild_id, panel_id).await {
        Ok(_) => {
            w.release(guild_id, before);
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn send_panel(
    State(w): State<Web>,
    headers: HeaderMap,
    Path((guild_id, panel_id)): Path<(u64, i64)>,
    Json(body): Json<Value>,
) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let Some(channel) = body["channel_id"].as_str().and_then(|c| c.parse::<u64>().ok()) else {
        return (StatusCode::BAD_REQUEST, "Pick a channel").into_response();
    };
    let channels = w.discord_list(&format!("guilds/{guild_id}/channels")).await;
    if !channels.iter().any(|c| c["id"].as_str() == Some(&channel.to_string()) && matches!(c["type"].as_i64(), Some(0 | 5))) {
        return (StatusCode::BAD_REQUEST, "That channel isn't in this server").into_response();
    }
    let Ok(Some(panel)) = w.data.db.panel(guild_id, panel_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::tickets::post_panel(&w.bot, channel, &panel).await {
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(e) => {
            let raw = e.to_string();
            let msg = if raw.contains("Missing") || raw.contains("permissions") {
                "I can't post in that channel. Give my role View Channel, Send Messages and Embed Links there.".to_string()
            } else {
                format!("Couldn't post: {raw}")
            };
            (StatusCode::BAD_REQUEST, msg).into_response()
        }
    }
}

#[derive(serde::Deserialize)]
struct StatsQ {
    days: Option<i64>,
}

async fn stats(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>, Query(q): Query<StatsQ>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    let days = q.days.unwrap_or(30).clamp(1, 365);
    match w.data.db.stats(guild_id, now_secs() - days * 86400).await {
        Ok(mut v) => {
            let ids: Vec<String> = v["staff"].as_array().map(|a| a.iter().filter_map(|s| s["user_id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            v["users"] = json!(w.user_names(&ids).await);
            v["days"] = json!(days);
            Json(v).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn logout(State(w): State<Web>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(t) = cookie.split(';').find_map(|c| c.trim().strip_prefix("tk_session=")) {
            let _ = w.data.db.delete_session(t).await;
        }
    }
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(header::SET_COOKIE, "tk_session=; Path=/; Max-Age=0".parse().unwrap());
    resp
}

async fn spa_index() -> impl IntoResponse {
    match tokio::fs::read_to_string("dashboard/dist/index.html").await {
        Ok(s) => axum::response::Html(s).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "dashboard not built — run `npm run build` in dashboard/").into_response(),
    }
}

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || "-_.~".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn random_token() -> String {
    use std::io::Read;
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).expect("urandom");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_guilds(j: &str) -> Vec<Value> {
    let v: Vec<Value> = serde_json::from_str(j).unwrap_or_default();
    v.into_iter().map(|g| if g.is_object() { g } else { json!({"id": g.to_string()}) }).collect()
}

fn gid(g: &Value) -> Option<i64> {
    g["id"].as_str()?.parse().ok()
}
