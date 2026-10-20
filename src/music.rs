//! Music lobbies: join-to-create voice channels with a shared queue, songs from the tunes! library,
//! internet radio and uploaded files.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::{HttpRequest, Input};

use crate::{Context, Data, Error};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct MusicConfig {
    pub enabled: bool,
    pub hub_channel_id: String,
    pub lobby_name: String,
    pub user_limit: i64,
    pub dj_role_id: String,
    pub vote_percent: i64,
    pub max_queue: i64,
}

impl Default for MusicConfig {
    fn default() -> Self {
        Self { enabled: true, hub_channel_id: String::new(), lobby_name: "{user}'s lobby".into(), user_limit: 0, dj_role_id: String::new(), vote_percent: 50, max_queue: 100 }
    }
}

impl MusicConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        self.lobby_name = self.lobby_name.trim().chars().take(90).collect();
        if self.lobby_name.is_empty() {
            self.lobby_name = "{user}'s lobby".into();
        }
        self.user_limit = self.user_limit.clamp(0, 99);
        self.vote_percent = self.vote_percent.clamp(1, 100);
        self.max_queue = self.max_queue.clamp(1, 500);
        if self.hub_channel_id.parse::<u64>().is_err() {
            self.hub_channel_id.clear();
        }
        if self.dj_role_id.parse::<u64>().is_err() {
            self.dj_role_id.clear();
        }
        Ok(())
    }
}

static CACHE: LazyLock<Mutex<HashMap<u64, (Instant, Arc<MusicConfig>)>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CACHE.lock().unwrap().remove(&guild);
}

pub async fn config(data: &Data, guild: u64) -> Arc<MusicConfig> {
    if let Some((at, c)) = CACHE.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let raw = data.db.module_config(guild, "music").await.ok().flatten();
    let c = Arc::new(raw.and_then(|r| serde_json::from_str::<MusicConfig>(&r).ok()).unwrap_or_default());
    CACHE.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

#[derive(Clone, Default)]
struct Meta {
    title: String,
    by: String,
    thumb: String,
    duration: Option<f64>,
    live: bool,
    requester: u64,
}

struct Session {
    voice: u64,
    text: u64,
    metas: HashMap<String, Meta>,
    votes: HashSet<u64>,
    panel: Option<u64>,
    volume: f32,
    paused: bool,
}

static SESSIONS: LazyLock<Mutex<HashMap<u64, Session>>> = LazyLock::new(Default::default);
/// lobby channel -> (guild, owner)
static LOBBIES: LazyLock<Mutex<HashMap<u64, (u64, u64)>>> = LazyLock::new(Default::default);
static JOIN_COOLDOWN: LazyLock<Mutex<HashMap<u64, Instant>>> = LazyLock::new(Default::default);
static STREAM: LazyLock<reqwest12::Client> = LazyLock::new(|| reqwest12::Client::builder().user_agent("KevBot/1.0 (+https://kev.kreatix.dev)").build().unwrap_or_default());
static API: LazyLock<reqwest::Client> = LazyLock::new(|| reqwest::Client::builder().user_agent("KevBot/1.0 (+https://kev.kreatix.dev)").timeout(Duration::from_secs(10)).build().unwrap_or_default());

fn tunes_base() -> String {
    std::env::var("TUNES_URL").unwrap_or_else(|_| "http://127.0.0.1:8268".into())
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Humans in a voice channel, from the cache.
fn humans(ctx: &serenity::Context, guild: u64, channel: u64) -> Vec<u64> {
    let me = ctx.cache.current_user().id;
    let Some(g) = ctx.cache.guild(serenity::GuildId::new(guild)) else { return vec![] };
    g.voice_states
        .values()
        .filter(|v| v.channel_id.map(|c| c.get()) == Some(channel) && v.user_id != me && !v.member.as_ref().is_some_and(|m| m.user.bot))
        .map(|v| v.user_id.get())
        .collect()
}

fn voice_of(ctx: &serenity::Context, guild: u64, user: u64) -> Option<u64> {
    let g = ctx.cache.guild(serenity::GuildId::new(guild))?;
    g.voice_states.get(&serenity::UserId::new(user))?.channel_id.map(|c| c.get())
}

async fn manager(ctx: &serenity::Context) -> Option<Arc<songbird::Songbird>> {
    songbird::get(ctx).await
}

async fn end_session(ctx: &serenity::Context, guild: u64) {
    let panel = SESSIONS.lock().unwrap().remove(&guild).and_then(|s| s.panel.map(|p| (s.text, p)));
    if let Some(m) = manager(ctx).await {
        if let Some(call) = m.get(serenity::GuildId::new(guild)) {
            call.lock().await.queue().stop();
        }
        let _ = m.remove(serenity::GuildId::new(guild)).await;
    }
    if let Some((text, msg)) = panel {
        let _ = serenity::ChannelId::new(text).delete_message(&ctx.http, serenity::MessageId::new(msg)).await;
    }
}

fn panel(meta: &Meta, queued: usize, paused: bool) -> (serenity::CreateEmbed, Vec<serenity::CreateActionRow>) {
    let mut lines = vec![];
    if !meta.by.is_empty() {
        lines.push(meta.by.clone());
    }
    lines.push(if meta.live { "Live".to_string() } else { meta.duration.map(fmt_time).unwrap_or_default() });
    lines.push(format!("Requested by <@{}>", meta.requester));
    let mut e = serenity::CreateEmbed::new()
        .author(serenity::CreateEmbedAuthor::new(if paused { "Paused" } else { "Now playing" }))
        .title(meta.title.chars().take(256).collect::<String>())
        .description(lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n"))
        .color(0x66BB6A)
        .footer(serenity::CreateEmbedFooter::new(if queued == 0 { "Nothing queued after this".to_string() } else { format!("{queued} more in the queue") }));
    if meta.thumb.starts_with("http") {
        e = e.thumbnail(&meta.thumb);
    }
    let row = serenity::CreateActionRow::Buttons(vec![
        serenity::CreateButton::new("music:pause").label(if paused { "Resume" } else { "Pause" }).style(serenity::ButtonStyle::Secondary),
        serenity::CreateButton::new("music:skip").label("Skip").style(serenity::ButtonStyle::Primary),
        serenity::CreateButton::new("music:stop").label("Stop").style(serenity::ButtonStyle::Danger),
    ]);
    (e, vec![row])
}

async fn refresh_panel(ctx: &serenity::Context, guild: u64, repost: bool) {
    let Some(m) = manager(ctx).await else { return };
    let Some(call) = m.get(serenity::GuildId::new(guild)) else { return };
    let (current, queued) = {
        let c = call.lock().await;
        (c.queue().current().map(|h| h.uuid().to_string()), c.queue().len().saturating_sub(1))
    };
    let Some(uuid) = current else { return };
    let (text, old, meta, paused) = {
        let s = SESSIONS.lock().unwrap();
        let Some(s) = s.get(&guild) else { return };
        let Some(meta) = s.metas.get(&uuid).cloned() else { return };
        (s.text, s.panel, meta, s.paused)
    };
    let (embed, rows) = panel(&meta, queued, paused);
    let ch = serenity::ChannelId::new(text);
    if let (false, Some(old)) = (repost, old) {
        if ch.edit_message(&ctx.http, serenity::MessageId::new(old), serenity::EditMessage::new().embed(embed.clone()).components(rows.clone())).await.is_ok() {
            return;
        }
    }
    if let Some(old) = old {
        let _ = ch.delete_message(&ctx.http, serenity::MessageId::new(old)).await;
    }
    if let Ok(msg) = ch.send_message(&ctx.http, serenity::CreateMessage::new().embed(embed).components(rows)).await {
        if let Some(s) = SESSIONS.lock().unwrap().get_mut(&guild) {
            s.panel = Some(msg.id.get());
        }
    }
}

struct TrackWatch {
    ctx: serenity::Context,
    guild: u64,
    kind: &'static str,
}

#[async_trait::async_trait]
impl EventHandler for TrackWatch {
    async fn act(&self, ev: &EventContext<'_>) -> Option<Event> {
        let EventContext::Track(list) = ev else { return None };
        let ids: Vec<String> = list.iter().map(|(_, h)| h.uuid().to_string()).collect();
        let (ctx, guild) = (self.ctx.clone(), self.guild);
        match self.kind {
            "play" => {
                if let Some(s) = SESSIONS.lock().unwrap().get_mut(&guild) {
                    s.votes.clear();
                    s.paused = false;
                }
                tokio::spawn(async move { refresh_panel(&ctx, guild, true).await });
            }
            _ => {
                let failed = self.kind == "error";
                let titles: Vec<(u64, String)> = {
                    let mut s = SESSIONS.lock().unwrap();
                    let Some(s) = s.get_mut(&guild) else { return None };
                    ids.iter().filter_map(|id| s.metas.remove(id)).map(|m| (s.text, m.title)).collect()
                };
                tokio::spawn(async move {
                    if failed {
                        for (text, title) in titles {
                            let _ = serenity::ChannelId::new(text).say(&ctx.http, format!("Couldn't play **{title}**, skipping it.")).await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let empty = match manager(&ctx).await.and_then(|m| m.get(serenity::GuildId::new(guild))) {
                        Some(call) => call.lock().await.queue().is_empty(),
                        None => return,
                    };
                    if !empty {
                        return;
                    }
                    let panel = SESSIONS.lock().unwrap().get_mut(&guild).and_then(|s| s.panel.take().map(|p| (s.text, p)));
                    if let Some((text, msg)) = panel {
                        let ch = serenity::ChannelId::new(text);
                        let _ = ch.delete_message(&ctx.http, serenity::MessageId::new(msg)).await;
                        let _ = ch.say(&ctx.http, "Queue finished. Add more with `/play` or `/radio`, I'll leave in 3 minutes otherwise.").await;
                    }
                    tokio::time::sleep(Duration::from_secs(180)).await;
                    if let Some(call) = manager(&ctx).await.and_then(|m| m.get(serenity::GuildId::new(guild))) {
                        if call.lock().await.queue().is_empty() {
                            end_session(&ctx, guild).await;
                        }
                    }
                });
            }
        }
        None
    }
}

async fn reply(ctx: Context<'_>, text: impl Into<String>, private: bool) -> Result<(), Error> {
    ctx.send(poise::CreateReply::default().content(text).ephemeral(private).allowed_mentions(serenity::CreateAllowedMentions::new())).await?;
    Ok(())
}

/// Joins the member's channel if needed and queues a track. Returns the reply text.
async fn enqueue(ctx: Context<'_>, meta: Meta, url: String) -> Result<String, Error> {
    let sctx = ctx.serenity_context();
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    let voice = voice_of(sctx, guild, ctx.author().id.get()).unwrap_or(0);
    let cfg = config(ctx.data(), guild).await;
    let Some(m) = manager(sctx).await else { return Ok("Music isn't available right now.".into()) };
    let fresh = !SESSIONS.lock().unwrap().contains_key(&guild) || m.get(serenity::GuildId::new(guild)).is_none();
    let call = if fresh {
        let call = match m.join(serenity::GuildId::new(guild), serenity::ChannelId::new(voice)).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("music join: {e}");
                let _ = m.remove(serenity::GuildId::new(guild)).await;
                return Ok("I couldn't join your voice channel. I need Connect and Speak there.".into());
            }
        };
        {
            let mut c = call.lock().await;
            c.remove_all_global_events();
            c.add_global_event(Event::Track(TrackEvent::Play), TrackWatch { ctx: sctx.clone(), guild, kind: "play" });
            c.add_global_event(Event::Track(TrackEvent::End), TrackWatch { ctx: sctx.clone(), guild, kind: "end" });
            c.add_global_event(Event::Track(TrackEvent::Error), TrackWatch { ctx: sctx.clone(), guild, kind: "error" });
        }
        SESSIONS.lock().unwrap().insert(guild, Session { voice, text: ctx.channel_id().get(), metas: HashMap::new(), votes: HashSet::new(), panel: None, volume: 1.0, paused: false });
        call
    } else {
        m.get(serenity::GuildId::new(guild)).unwrap()
    };
    let mut c = call.lock().await;
    if c.queue().len() as i64 >= cfg.max_queue {
        return Ok(format!("The queue is full ({} max).", cfg.max_queue));
    }
    let input: Input = HttpRequest::new(STREAM.clone(), url).into();
    let handle = c.enqueue_input(input).await;
    let ahead = c.queue().len().saturating_sub(1);
    drop(c);
    let volume = {
        let mut s = SESSIONS.lock().unwrap();
        let s = s.get_mut(&guild);
        let v = s.as_ref().map(|s| s.volume).unwrap_or(1.0);
        if let Some(s) = s {
            s.metas.insert(handle.uuid().to_string(), meta.clone());
        }
        v
    };
    let _ = handle.set_volume(volume);
    Ok(if ahead == 0 { format!("Playing **{}**", meta.title) } else { format!("Added **{}**, {} ahead of it", meta.title, ahead) })
}

/// Checks the member is in voice and the bot isn't busy elsewhere. Some(error) when not.
fn voice_check(ctx: Context<'_>) -> Option<String> {
    let guild = ctx.guild_id()?.get();
    let Some(voice) = voice_of(ctx.serenity_context(), guild, ctx.author().id.get()) else {
        return Some("Join a voice channel first.".into());
    };
    let busy = SESSIONS.lock().unwrap().get(&guild).map(|s| s.voice).filter(|v| *v != voice);
    if let Some(other) = busy {
        if !humans(ctx.serenity_context(), guild, other).is_empty() {
            return Some(format!("I'm playing in <#{other}> right now. Join that channel, or wait until it's free."));
        }
        SESSIONS.lock().unwrap().remove(&guild);
    }
    None
}

fn blocked_link(url: &str) -> bool {
    ["youtube.com", "youtu.be", "spotify.com", "soundcloud.com", "music.apple.com", "deezer.com", "tidal.com"].iter().any(|d| url.contains(d))
}

/// Play a song, an audio link or a file
#[poise::command(slash_command, guild_only)]
pub async fn play(
    ctx: Context<'_>,
    #[description = "Song name or a link to an audio file"] song: Option<String>,
    #[description = "Upload an audio file"] file: Option<serenity::Attachment>,
) -> Result<(), Error> {
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    if !config(ctx.data(), guild).await.enabled {
        return reply(ctx, "Music is turned off in this server.", true).await;
    }
    if let Some(e) = voice_check(ctx) {
        return reply(ctx, e, true).await;
    }
    let me = ctx.author().id.get();
    let (meta, url) = if let Some(f) = file {
        let audio = f.content_type.as_deref().is_some_and(|t| t.starts_with("audio/") || t == "video/mp4" || t == "application/ogg")
            || [".mp3", ".ogg", ".opus", ".flac", ".wav", ".m4a", ".aac"].iter().any(|x| f.filename.to_lowercase().ends_with(x));
        if !audio {
            return reply(ctx, "That file isn't audio. MP3, OGG, FLAC, WAV and M4A work.", true).await;
        }
        (Meta { title: f.filename.clone(), requester: me, ..Default::default() }, f.url.clone())
    } else if let Some(q) = song.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        if q.starts_with("https://") || q.starts_with("http://") {
            if blocked_link(&q) {
                return reply(ctx, "Links from YouTube, Spotify and other streaming sites can't be played. Search by the song name instead, or upload the file.", true).await;
            }
            if crate::agent::guarded_client(&q).await.is_err() {
                return reply(ctx, "I can't play that link.", true).await;
            }
            let name = q.split(['?', '#']).next().unwrap_or(&q).rsplit('/').next().unwrap_or("Audio").to_string();
            (Meta { title: if name.is_empty() { "Audio".into() } else { name }, requester: me, ..Default::default() }, q)
        } else {
            ctx.defer().await?;
            let r = match reqwest::Url::parse_with_params(&format!("{}/api/tracks", tunes_base()), &[("q", q.as_str()), ("limit", "5"), ("sort", "top")]) {
                Ok(u) => API.get(u).send().await,
                Err(_) => return reply(ctx, "Search is down right now.", false).await,
            };
            let tracks: Vec<serde_json::Value> = match r {
                Ok(r) if r.status().is_success() => r.json::<serde_json::Value>().await.ok().and_then(|v| v["tracks"].as_array().cloned()).unwrap_or_default(),
                _ => vec![],
            };
            let Some(t) = tracks.into_iter().find(|t| t["id"].as_str().is_some()) else {
                return reply(ctx, format!("No song found for \"{q}\". Try `/radio` for stations, or upload a file."), false).await;
            };
            let id = t["id"].as_str().unwrap_or_default();
            (
                Meta {
                    title: t["title"].as_str().unwrap_or("Song").into(),
                    by: t["artist"].as_str().unwrap_or("").into(),
                    thumb: t["cover_url"].as_str().unwrap_or("").into(),
                    duration: t["duration"].as_f64(),
                    live: false,
                    requester: me,
                },
                format!("{}/api/tracks/{id}/stream", tunes_base()),
            )
        }
    } else {
        return reply(ctx, "Type a song name, paste an audio link or upload a file.", true).await;
    };
    let text = enqueue(ctx, meta, url).await?;
    reply(ctx, text, false).await
}

/// Play an internet radio station
#[poise::command(slash_command, guild_only)]
pub async fn radio(ctx: Context<'_>, #[description = "Station name, genre or country, like lofi or jazz"] station: String) -> Result<(), Error> {
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    if !config(ctx.data(), guild).await.enabled {
        return reply(ctx, "Music is turned off in this server.", true).await;
    }
    if let Some(e) = voice_check(ctx) {
        return reply(ctx, e, true).await;
    }
    ctx.defer().await?;
    let q = station.trim().to_string();
    let mut found: Option<serde_json::Value> = None;
    'hosts: for host in ["de1", "de2", "fi1"] {
        for field in ["name", "tag"] {
            let url = format!("https://{host}.api.radio-browser.info/json/stations/search");
            let Ok(u) = reqwest::Url::parse_with_params(&url, &[(field, q.as_str()), ("limit", "20"), ("hidebroken", "true"), ("order", "clickcount"), ("reverse", "true")]) else { continue 'hosts };
            let Ok(r) = API.get(u).send().await else { continue 'hosts };
            let list: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            found = list.into_iter().find(|s| {
                let codec = s["codec"].as_str().unwrap_or("").to_uppercase();
                s["hls"].as_i64() != Some(1) && (codec == "MP3" || codec.starts_with("AAC") || codec == "OGG") && s["url_resolved"].as_str().is_some_and(|u| u.starts_with("http"))
            });
            if found.is_some() {
                break 'hosts;
            }
        }
        break;
    }
    let Some(s) = found else {
        return reply(ctx, format!("No station found for \"{q}\"."), false).await;
    };
    let by = [s["country"].as_str().unwrap_or(""), s["tags"].as_str().unwrap_or("").split(',').take(3).collect::<Vec<_>>().join(", ").as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
    let meta = Meta { title: s["name"].as_str().unwrap_or("Radio").trim().into(), by, thumb: s["favicon"].as_str().unwrap_or("").into(), duration: None, live: true, requester: ctx.author().id.get() };
    let text = enqueue(ctx, meta, s["url_resolved"].as_str().unwrap_or_default().to_string()).await?;
    reply(ctx, text, false).await
}

/// Whether this member can control playback without a vote.
fn controls(ctx: &serenity::Context, guild: u64, user: u64, member: Option<&serenity::Member>, cfg: &MusicConfig) -> bool {
    if member.and_then(|m| m.permissions).is_some_and(|p| p.administrator() || p.manage_channels()) {
        return true;
    }
    if let Ok(dj) = cfg.dj_role_id.parse::<u64>() {
        if member.is_some_and(|m| m.roles.contains(&serenity::RoleId::new(dj))) {
            return true;
        }
    }
    let voice = SESSIONS.lock().unwrap().get(&guild).map(|s| s.voice);
    let Some(voice) = voice else { return false };
    if LOBBIES.lock().unwrap().get(&voice).is_some_and(|(_, owner)| *owner == user) {
        return true;
    }
    let others = humans(ctx, guild, voice);
    others.len() <= 1 || LOBBIES.lock().unwrap().get(&voice).is_none() && others.first() == Some(&user) && others.len() == 1
}

/// Runs a playback action for a member. Returns (reply, private).
async fn action(ctx: &serenity::Context, data: &Data, guild: u64, user: u64, member: Option<&serenity::Member>, what: &str) -> (String, bool) {
    let voice = SESSIONS.lock().unwrap().get(&guild).map(|s| s.voice);
    let Some(voice) = voice else { return ("Nothing is playing.".into(), true) };
    let Some(call) = manager(ctx).await.and_then(|m| m.get(serenity::GuildId::new(guild))) else { return ("Nothing is playing.".into(), true) };
    if voice_of(ctx, guild, user) != Some(voice) {
        return (format!("Join <#{voice}> first."), true);
    }
    let cfg = config(data, guild).await;
    let boss = controls(ctx, guild, user, member, &cfg);
    let current = call.lock().await.queue().current();
    match what {
        "skip" => {
            if current.is_none() {
                return ("Nothing is playing.".into(), true);
            }
            if !boss {
                let listeners = humans(ctx, guild, voice).len().max(1);
                let need = ((listeners as f64) * cfg.vote_percent as f64 / 100.0).ceil().max(1.0) as usize;
                let votes = {
                    let mut s = SESSIONS.lock().unwrap();
                    let Some(s) = s.get_mut(&guild) else { return ("Nothing is playing.".into(), true) };
                    s.votes.insert(user);
                    s.votes.len()
                };
                if votes < need {
                    return (format!("<@{user}> voted to skip, {votes}/{need}."), false);
                }
            }
            let _ = call.lock().await.queue().skip();
            ("Skipped.".into(), false)
        }
        "pause" => {
            if !boss {
                return ("Only the lobby owner or a DJ can pause.".into(), true);
            }
            let Some(h) = current else { return ("Nothing is playing.".into(), true) };
            let paused = {
                let mut s = SESSIONS.lock().unwrap();
                let Some(s) = s.get_mut(&guild) else { return ("Nothing is playing.".into(), true) };
                s.paused = !s.paused;
                s.paused
            };
            let _ = if paused { h.pause() } else { h.play() };
            refresh_panel(ctx, guild, false).await;
            (if paused { "Paused." } else { "Resumed." }.into(), false)
        }
        "stop" => {
            if !boss {
                return ("Only the lobby owner or a DJ can stop the music.".into(), true);
            }
            end_session(ctx, guild).await;
            ("Stopped and left.".into(), false)
        }
        _ => (String::new(), true),
    }
}

async fn run(ctx: Context<'_>, what: &str) -> Result<(), Error> {
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    let member = ctx.author_member().await.map(|m| m.into_owned());
    let (text, private) = action(ctx.serenity_context(), ctx.data(), guild, ctx.author().id.get(), member.as_ref(), what).await;
    reply(ctx, text, private).await
}

/// Skip the song, or vote to skip it
#[poise::command(slash_command, guild_only)]
pub async fn skip(ctx: Context<'_>) -> Result<(), Error> {
    run(ctx, "skip").await
}

/// Pause or resume the music
#[poise::command(slash_command, guild_only)]
pub async fn pause(ctx: Context<'_>) -> Result<(), Error> {
    run(ctx, "pause").await
}

/// Stop the music and leave the voice channel
#[poise::command(slash_command, guild_only)]
pub async fn stop(ctx: Context<'_>) -> Result<(), Error> {
    run(ctx, "stop").await
}

/// See what's playing and what's next
#[poise::command(slash_command, guild_only)]
pub async fn queue(ctx: Context<'_>) -> Result<(), Error> {
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    let Some(call) = manager(ctx.serenity_context()).await.and_then(|m| m.get(serenity::GuildId::new(guild))) else {
        return reply(ctx, "Nothing is playing.", true).await;
    };
    let ids: Vec<String> = call.lock().await.queue().current_queue().iter().map(|h| h.uuid().to_string()).collect();
    let lines: Vec<String> = {
        let s = SESSIONS.lock().unwrap();
        s.get(&guild).map(|s| ids.iter()
            .enumerate()
            .filter_map(|(i, id)| s.metas.get(id).map(|m| (i, m)))
            .take(15)
            .map(|(i, m)| {
                let len = if m.live { "live".to_string() } else { m.duration.map(fmt_time).unwrap_or_default() };
                if i == 0 { format!("**Now:** {} {}", m.title, len) } else { format!("`{i}` {} {}", m.title, len) }
            })
            .collect()).unwrap_or_default()
    };
    if lines.is_empty() {
        return reply(ctx, "Nothing is playing.", true).await;
    }
    let more = ids.len().saturating_sub(15);
    let text = if more > 0 { format!("{}\nand {more} more", lines.join("\n")) } else { lines.join("\n") };
    reply(ctx, text, true).await
}

/// Change the volume
#[poise::command(slash_command, guild_only)]
pub async fn volume(
    ctx: Context<'_>,
    #[description = "1 to 150"]
    #[min = 1]
    #[max = 150]
    level: u8,
) -> Result<(), Error> {
    let guild = ctx.guild_id().map(|g| g.get()).unwrap_or(0);
    let member = ctx.author_member().await.map(|m| m.into_owned());
    let cfg = config(ctx.data(), guild).await;
    if !SESSIONS.lock().unwrap().contains_key(&guild) {
        return reply(ctx, "Nothing is playing.", true).await;
    }
    if !controls(ctx.serenity_context(), guild, ctx.author().id.get(), member.as_ref(), &cfg) {
        return reply(ctx, "Only the lobby owner or a DJ can change the volume.", true).await;
    }
    let v = level as f32 / 100.0;
    if let Some(s) = SESSIONS.lock().unwrap().get_mut(&guild) {
        s.volume = v;
    }
    if let Some(call) = manager(ctx.serenity_context()).await.and_then(|m| m.get(serenity::GuildId::new(guild))) {
        for h in call.lock().await.queue().current_queue() {
            let _ = h.set_volume(v);
        }
    }
    reply(ctx, format!("Volume set to {level}%."), false).await
}

pub async fn on_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<(), Error> {
    let what = c.data.custom_id.trim_start_matches("music:");
    let guild = c.guild_id.map(|g| g.get()).unwrap_or(0);
    let (text, private) = action(ctx, data, guild, c.user.id.get(), c.member.as_ref(), what).await;
    let msg = serenity::CreateInteractionResponseMessage::new().content(text).ephemeral(private).allowed_mentions(serenity::CreateAllowedMentions::new());
    c.create_response(&ctx.http, serenity::CreateInteractionResponse::Message(msg)).await?;
    Ok(())
}

async fn make_lobby(ctx: &serenity::Context, data: &Data, guild: u64, user: u64, hub: u64, cfg: &MusicConfig) {
    {
        let mut cd = JOIN_COOLDOWN.lock().unwrap();
        if cd.get(&user).is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
            return;
        }
        cd.insert(user, Instant::now());
    }
    let (parent, name) = {
        let Some(g) = ctx.cache.guild(serenity::GuildId::new(guild)) else { return };
        let parent = g.channels.get(&serenity::ChannelId::new(hub)).and_then(|c| c.parent_id);
        let display = g.members.get(&serenity::UserId::new(user)).map(|m| m.display_name().to_string()).unwrap_or_else(|| "Someone".into());
        (parent, cfg.lobby_name.replace("{user}", &display).chars().take(100).collect::<String>())
    };
    let owner = serenity::PermissionOverwrite {
        allow: serenity::Permissions::MANAGE_CHANNELS | serenity::Permissions::MOVE_MEMBERS | serenity::Permissions::MUTE_MEMBERS | serenity::Permissions::CONNECT | serenity::Permissions::SPEAK,
        deny: serenity::Permissions::empty(),
        kind: serenity::PermissionOverwriteType::Member(serenity::UserId::new(user)),
    };
    let mut b = serenity::CreateChannel::new(name).kind(serenity::ChannelType::Voice).permissions(vec![owner]);
    if let Some(p) = parent {
        b = b.category(p);
    }
    if cfg.user_limit > 0 {
        b = b.user_limit(cfg.user_limit as u32);
    }
    let ch = match serenity::GuildId::new(guild).create_channel(&ctx.http, b).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("music lobby create in {guild}: {e}");
            return;
        }
    };
    LOBBIES.lock().unwrap().insert(ch.id.get(), (guild, user));
    let _ = data.db.add_lobby(ch.id.get(), guild, user).await;
    if serenity::GuildId::new(guild).move_member(&ctx.http, serenity::UserId::new(user), ch.id).await.is_err() {
        let _ = ch.id.delete(&ctx.http).await;
        LOBBIES.lock().unwrap().remove(&ch.id.get());
        let _ = data.db.delete_lobby(ch.id.get()).await;
        return;
    }
    let _ = ch.id.say(&ctx.http, format!("<@{user}> this is your lobby. Play music with `/play` or `/radio`, everyone here shares the queue and you control it. The channel is deleted when everyone leaves.")).await;
}

async fn left(ctx: &serenity::Context, data: &Data, guild: u64, channel: u64, user: u64) {
    let rest = humans(ctx, guild, channel);
    let lobby = LOBBIES.lock().unwrap().get(&channel).copied();
    let playing_here = SESSIONS.lock().unwrap().get(&guild).is_some_and(|s| s.voice == channel);
    if rest.is_empty() {
        if playing_here {
            end_session(ctx, guild).await;
        }
        if lobby.is_some() {
            let _ = serenity::ChannelId::new(channel).delete(&ctx.http).await;
            LOBBIES.lock().unwrap().remove(&channel);
            let _ = data.db.delete_lobby(channel).await;
        }
        return;
    }
    if let Some((_, owner)) = lobby.filter(|(_, o)| *o == user) {
        let next = rest[0];
        let _ = owner;
        LOBBIES.lock().unwrap().insert(channel, (guild, next));
        let _ = data.db.set_lobby_owner(channel, next).await;
        let ch = serenity::ChannelId::new(channel);
        let ow = serenity::PermissionOverwrite {
            allow: serenity::Permissions::MANAGE_CHANNELS | serenity::Permissions::MOVE_MEMBERS | serenity::Permissions::MUTE_MEMBERS | serenity::Permissions::CONNECT | serenity::Permissions::SPEAK,
            deny: serenity::Permissions::empty(),
            kind: serenity::PermissionOverwriteType::Member(serenity::UserId::new(next)),
        };
        let _ = ch.create_permission(&ctx.http, ow).await;
        let _ = ch.delete_permission(&ctx.http, serenity::PermissionOverwriteType::Member(serenity::UserId::new(user))).await;
        let _ = ch.say(&ctx.http, format!("<@{next}> owns this lobby now.")).await;
    }
}

pub async fn on_voice(ctx: &serenity::Context, data: &Data, old: Option<&serenity::VoiceState>, new: &serenity::VoiceState) {
    let Some(guild) = new.guild_id.map(|g| g.get()) else { return };
    let user = new.user_id.get();
    let before = old.and_then(|o| o.channel_id).map(|c| c.get());
    let after = new.channel_id.map(|c| c.get());
    if before == after {
        return;
    }
    if user == ctx.cache.current_user().id.get() {
        match after {
            None => {
                SESSIONS.lock().unwrap().remove(&guild);
            }
            Some(ch) => {
                if let Some(s) = SESSIONS.lock().unwrap().get_mut(&guild) {
                    s.voice = ch;
                }
            }
        }
        return;
    }
    if new.member.as_ref().is_some_and(|m| m.user.bot) {
        return;
    }
    if let Some(ch) = before {
        left(ctx, data, guild, ch, user).await;
    }
    if let Some(ch) = after {
        let cfg = config(data, guild).await;
        if cfg.enabled && cfg.hub_channel_id == ch.to_string() {
            make_lobby(ctx, data, guild, user, ch, &cfg).await;
        }
    }
}

/// Loads lobbies after a restart and removes the ones that emptied while Kev was offline.
pub async fn startup(ctx: serenity::Context, data: Data) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    for (ch, guild, owner) in data.db.lobbies().await.unwrap_or_default() {
        if ctx.cache.guild(serenity::GuildId::new(guild)).is_none() {
            continue;
        }
        if humans(&ctx, guild, ch).is_empty() {
            let _ = serenity::ChannelId::new(ch).delete(&ctx.http).await;
            let _ = data.db.delete_lobby(ch).await;
        } else {
            LOBBIES.lock().unwrap().insert(ch, (guild, owner));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn radio_stream_decodes() {
        let url = "http://usa9.fastcast4u.com/proxy/jamz?mp=/1".to_string();
        let input: Input = HttpRequest::new(STREAM.clone(), url).into();
        let Input::Live(live, _) = input.make_live_async().await.expect("live") else { panic!("not live") };
        let parsed = tokio::task::spawn_blocking(move || live.promote(songbird::input::codecs::get_codec_registry(), songbird::input::codecs::get_probe())).await.unwrap().expect("probe");
        assert!(parsed.is_playable());
    }
}
