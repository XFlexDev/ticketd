use crate::Data;
use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, put},
    Json, Router,
};
use serde_json::{json, Value};
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
    };

    Ok(Router::new()
        .route("/api/auth/discord", get(auth_start))
        .route("/api/auth/callback", get(auth_callback))
        .route("/api/logout", get(logout))
        .route("/api/me", get(me))
        .route("/api/guilds/{guild_id}", get(get_guild))
        .route("/api/guilds/{guild_id}/settings", put(put_settings))
        .route("/api/guilds/{guild_id}/tickets", get(list_tickets))
        .route("/api/tickets/{ticket_id}/transcript", get(transcript))
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
    };
    match w.data.db.upsert_settings(&s).await {
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_tickets(State(w): State<Web>, headers: HeaderMap, Path(guild_id): Path<u64>) -> Response {
    let session = w.session(&headers).await;
    if let Err(s) = w.require_guild(&session, guild_id) {
        return s.into_response();
    }
    match w.data.db.list_tickets(guild_id, 200).await {
        Ok(tickets) => Json(json!({
            "tickets": tickets.iter().map(|t| json!({
                "id": t.id,
                "channel_id": t.channel_id.to_string(),
                "user_id": t.user_id.to_string(),
                "subject": t.subject,
                "status": t.status,
            })).collect::<Vec<_>>()
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
