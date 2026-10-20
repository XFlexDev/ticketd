//! Rule-based automod and anti-raid. No AI, so it costs nothing per message.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::moderation::{self, Act};
use crate::Data;

#[derive(Serialize, Deserialize, Clone)]
pub struct Rule {
    #[serde(default)]
    pub on: bool,
    /// delete | warn | timeout | kick | ban
    #[serde(default = "del")]
    pub action: String,
    #[serde(default)]
    pub minutes: i64,
    #[serde(default)]
    pub limit: i64,
}

fn del() -> String {
    "delete".into()
}

fn rule(on: bool, action: &str, minutes: i64, limit: i64) -> Rule {
    Rule { on, action: action.into(), minutes, limit }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AutomodConfig {
    pub enabled: bool,
    pub ignored_roles: Vec<String>,
    pub ignored_channels: Vec<String>,
    /// limit = messages within 5 seconds
    pub spam: Rule,
    /// limit = identical messages within 60 seconds
    pub duplicates: Rule,
    /// limit = user and role mentions in one message
    pub mentions: Rule,
    pub invites: Rule,
    pub links: Rule,
    pub allowed_domains: Vec<String>,
    pub scam: Rule,
    pub words: Rule,
    pub word_list: Vec<String>,
    /// limit = percent of capital letters
    pub caps: Rule,
    /// limit = emojis in one message
    pub emoji: Rule,
    pub zalgo: Rule,
    pub raid: Raid,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Raid {
    pub on: bool,
    pub joins: i64,
    pub seconds: i64,
    /// Minutes until the lockdown lifts by itself.
    pub lockdown_minutes: i64,
    /// Kick everyone who joins during a lockdown.
    pub kick_during: bool,
    /// 0 = off
    pub min_account_days: i64,
    /// kick | timeout | ban
    pub young_action: String,
    pub alert_channel_id: String,
}

impl Default for Raid {
    fn default() -> Self {
        Self { on: true, joins: 10, seconds: 10, lockdown_minutes: 15, kick_during: false, min_account_days: 0, young_action: "kick".into(), alert_channel_id: String::new() }
    }
}

impl Default for AutomodConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ignored_roles: vec![],
            ignored_channels: vec![],
            spam: rule(true, "timeout", 10, 6),
            duplicates: rule(true, "delete", 0, 4),
            mentions: rule(true, "timeout", 60, 6),
            invites: rule(false, "delete", 0, 0),
            links: rule(false, "delete", 0, 0),
            allowed_domains: ["tenor.com", "giphy.com", "youtube.com", "youtu.be", "github.com"].map(String::from).to_vec(),
            scam: rule(true, "timeout", 1440, 0),
            words: rule(false, "delete", 0, 0),
            word_list: vec![],
            caps: rule(false, "delete", 0, 75),
            emoji: rule(false, "delete", 0, 12),
            zalgo: rule(true, "delete", 0, 0),
            raid: Raid::default(),
        }
    }
}

const ACTIONS: [&str; 5] = ["delete", "warn", "timeout", "kick", "ban"];

impl AutomodConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        let id = |s: &String| s.parse::<u64>().is_ok();
        self.ignored_roles.retain(id);
        self.ignored_channels.retain(id);
        self.ignored_roles.truncate(50);
        self.ignored_channels.truncate(100);
        for (r, lo, hi) in [
            (&mut self.spam, 3, 30),
            (&mut self.duplicates, 2, 20),
            (&mut self.mentions, 2, 50),
            (&mut self.invites, 0, 0),
            (&mut self.links, 0, 0),
            (&mut self.scam, 0, 0),
            (&mut self.words, 0, 0),
            (&mut self.caps, 50, 100),
            (&mut self.emoji, 3, 100),
            (&mut self.zalgo, 0, 0),
        ] {
            if !ACTIONS.contains(&r.action.as_str()) {
                return Err("Automod actions are delete, warn, timeout, kick or ban".into());
            }
            r.limit = r.limit.clamp(lo, hi);
            r.minutes = r.minutes.clamp(0, 40320);
            if r.action == "timeout" && r.minutes == 0 {
                r.minutes = 10;
            }
        }
        let clean = |v: &mut Vec<String>, max_len: usize, max: usize| {
            *v = v.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty() && w.len() <= max_len).collect::<Vec<_>>();
            v.dedup();
            v.truncate(max);
        };
        clean(&mut self.word_list, 60, 500);
        clean(&mut self.allowed_domains, 100, 100);
        for d in &mut self.allowed_domains {
            *d = d.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.").trim_end_matches('/').to_string();
        }
        let r = &mut self.raid;
        r.joins = r.joins.clamp(3, 100);
        r.seconds = r.seconds.clamp(5, 300);
        r.lockdown_minutes = r.lockdown_minutes.clamp(1, 1440);
        r.min_account_days = r.min_account_days.clamp(0, 365);
        if !matches!(r.young_action.as_str(), "kick" | "timeout" | "ban") {
            r.young_action = "kick".into();
        }
        if !r.alert_channel_id.is_empty() && !id(&r.alert_channel_id) {
            r.alert_channel_id.clear();
        }
        Ok(())
    }
}

static CONFIGS: LazyLock<Mutex<HashMap<u64, (Instant, Arc<AutomodConfig>)>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CONFIGS.lock().unwrap().remove(&guild);
}

pub async fn config(data: &Data, guild: u64) -> Arc<AutomodConfig> {
    if let Some((at, c)) = CONFIGS.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let c: AutomodConfig = data.db.module_config(guild, "automod").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default();
    let c = Arc::new(c);
    CONFIGS.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

// ---------- scam domains ----------

static SCAM: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(Default::default);

const SCAM_LISTS: [&str; 2] = [
    "https://raw.githubusercontent.com/nikolaischunk/discord-phishing-links/main/domain-list.json",
    "https://raw.githubusercontent.com/Discord-AntiScam/scam-links/main/list.txt",
];

async fn load_scam_lists() -> usize {
    let http = reqwest::Client::new();
    let mut set = HashSet::new();
    for url in SCAM_LISTS {
        let Ok(r) = http.get(url).timeout(Duration::from_secs(30)).send().await else { continue };
        let Ok(body) = r.text().await else { continue };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
            for d in v["domains"].as_array().into_iter().flatten().filter_map(|d| d.as_str()) {
                set.insert(d.trim().to_lowercase());
            }
        } else {
            for line in body.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                set.insert(line.to_lowercase());
            }
        }
    }
    for safe in OFFICIAL {
        set.remove(*safe);
    }
    let n = set.len();
    if n > 0 {
        *SCAM.write().unwrap() = set;
    }
    n
}

pub async fn scam_refresher() {
    loop {
        let n = load_scam_lists().await;
        tracing::info!("automod: {n} scam domains loaded");
        tokio::time::sleep(Duration::from_secs(if n > 0 { 12 * 3600 } else { 600 })).await;
    }
}

const OFFICIAL: &[&str] = &[
    "discord.com", "discord.gg", "discordapp.com", "discordapp.net", "discord.media", "discord.new", "discord.gift", "discord.co", "discordstatus.com", "dis.gd",
    "steamcommunity.com", "steampowered.com",
];

fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(host), |h| h.split_once('.').map(|(_, rest)| rest)).filter(|h| h.contains('.'))
}

fn is_scam(host: &str) -> bool {
    if suffixes(host).any(|h| OFFICIAL.contains(&h)) {
        return false;
    }
    if (host.contains("nitro") || host.contains("disc0rd") || host.contains("dlscord") || host.contains("discrod")) && !host.ends_with(".gg") {
        return true;
    }
    let set = SCAM.read().unwrap();
    suffixes(host).any(|h| set.contains(h))
}

// ---------- text checks ----------

/// Hosts of every link in the text, lowercased, without www.
pub fn hosts(text: &str) -> Vec<String> {
    let mut out = vec![];
    for tok in text.split(|c: char| c.is_whitespace() || "<>()[]\"'`|".contains(c)) {
        let lower = tok.to_lowercase();
        let rest = if let Some(i) = lower.find("://") {
            if !lower[..i].ends_with("http") && !lower[..i].ends_with("https") {
                continue;
            }
            &lower[i + 3..]
        } else if lower.starts_with("www.") || lower.starts_with("discord.gg/") {
            &lower[..]
        } else {
            continue;
        };
        let host = rest.split(['/', '?', '#', ':']).next().unwrap_or("").trim_start_matches("www.").trim_end_matches('.');
        if host.contains('.') && host.len() < 254 {
            out.push(host.to_string());
        }
    }
    out
}

fn invite_codes(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out = vec![];
    for pat in ["discord.gg/", "discord.com/invite/", "discordapp.com/invite/", "discord.me/", "dsc.gg/"] {
        let mut s = &lower[..];
        while let Some(i) = s.find(pat) {
            let code: String = s[i + pat.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            if !code.is_empty() {
                out.push(if pat == "discord.me/" || pat == "dsc.gg/" { format!("{pat}{code}") } else { code });
            }
            s = &s[i + pat.len()..];
        }
    }
    out
}

fn glob(pat: &str, word: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == word;
    }
    let mut rest = word;
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if i == 0 {
            if !rest.starts_with(p) {
                return false;
            }
            rest = &rest[p.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else if let Some(j) = rest.find(p) {
            rest = &rest[j + p.len()..];
        } else {
            return false;
        }
    }
    true
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | '!' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            _ => c,
        })
        .collect()
}

fn bad_word(text: &str, list: &[String]) -> Option<String> {
    let norm = normalize(text);
    let lower = text.to_lowercase();
    let words: Vec<&str> = norm.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for w in list {
        if w.contains(' ') {
            if lower.contains(w.as_str()) || norm.contains(w.as_str()) {
                return Some(w.clone());
            }
        } else if words.iter().any(|x| glob(w, x)) || lower.split(|c: char| !c.is_alphanumeric()).any(|x| glob(w, x)) {
            return Some(w.clone());
        }
    }
    None
}

fn caps_percent(text: &str) -> Option<i64> {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 12 {
        return None;
    }
    Some(letters.iter().filter(|c| c.is_uppercase()).count() as i64 * 100 / letters.len() as i64)
}

fn emoji_count(text: &str) -> i64 {
    let unicode = text.chars().filter(|c| matches!(*c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x1F000..=0x1F2FF)).count();
    let custom = text.matches("<:").count() + text.matches("<a:").count();
    (unicode + custom) as i64
}

fn is_zalgo(text: &str) -> bool {
    let marks = text.chars().filter(|c| matches!(*c as u32, 0x0300..=0x036F | 0x0489 | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)).count();
    marks >= 12
}

// ---------- per-user state ----------

struct Recent {
    at: Instant,
    id: serenity::MessageId,
    channel: serenity::ChannelId,
    hash: u64,
}

static HISTORY: LazyLock<Mutex<HashMap<(u64, u64), VecDeque<Recent>>>> = LazyLock::new(Default::default);
static COOLDOWN: LazyLock<Mutex<HashMap<(u64, u64), Instant>>> = LazyLock::new(Default::default);
static INVITES: LazyLock<Mutex<HashMap<String, Option<u64>>>> = LazyLock::new(Default::default);

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.trim().to_lowercase().hash(&mut h);
    h.finish()
}

fn exempt(ctx: &serenity::Context, guild: serenity::GuildId, msg: &serenity::Message, cfg: &AutomodConfig) -> bool {
    if cfg.ignored_channels.iter().any(|c| c == &msg.channel_id.to_string()) {
        return true;
    }
    let Some(m) = msg.member.as_ref() else { return true };
    if m.roles.iter().any(|r| cfg.ignored_roles.contains(&r.to_string())) {
        return true;
    }
    let Some(g) = ctx.cache.guild(guild) else { return true };
    if g.owner_id == msg.author.id {
        return true;
    }
    let staff = serenity::Permissions::ADMINISTRATOR | serenity::Permissions::MANAGE_GUILD | serenity::Permissions::MANAGE_MESSAGES | serenity::Permissions::MODERATE_MEMBERS;
    m.roles.iter().filter_map(|r| g.roles.get(r)).any(|r| r.permissions.intersects(staff))
}

async fn own_invite(ctx: &serenity::Context, guild: serenity::GuildId, code: &str) -> bool {
    if code.contains('/') {
        return false;
    }
    if let Some(g) = INVITES.lock().unwrap().get(code) {
        return *g == Some(guild.get());
    }
    let g = ctx.http.get_invite(code, false, false, None).await.ok().and_then(|i| i.guild.map(|g| g.id.get()));
    let mut map = INVITES.lock().unwrap();
    if map.len() > 5000 {
        map.clear();
    }
    map.insert(code.to_string(), g);
    g == Some(guild.get())
}

/// Returns true when the message was removed, so nothing else should handle it.
pub async fn on_message(ctx: &serenity::Context, msg: &serenity::Message, data: &Data) -> bool {
    if msg.author.bot || msg.webhook_id.is_some() {
        return false;
    }
    let Some(guild) = msg.guild_id else { return false };
    let cfg = config(data, guild.get()).await;
    if !cfg.enabled || exempt(ctx, guild, msg, &cfg) {
        return false;
    }
    let key = (guild.get(), msg.author.id.get());
    let text = &msg.content;

    // spam and duplicates use a short per-user history
    let (spam_hit, dup_hit, burst) = {
        let mut h = HISTORY.lock().unwrap();
        if h.len() > 20000 {
            h.retain(|_, q| q.back().is_some_and(|r| r.at.elapsed() < Duration::from_secs(60)));
        }
        let q = h.entry(key).or_default();
        q.push_back(Recent { at: Instant::now(), id: msg.id, channel: msg.channel_id, hash: hash(text) });
        while q.front().is_some_and(|r| r.at.elapsed() > Duration::from_secs(60)) {
            q.pop_front();
        }
        let recent = q.iter().filter(|r| r.at.elapsed() <= Duration::from_secs(5)).count() as i64;
        let h0 = hash(text);
        let dups = if text.trim().len() >= 3 { q.iter().filter(|r| r.hash == h0).count() as i64 } else { 0 };
        let spam_hit = cfg.spam.on && recent >= cfg.spam.limit;
        let dup_hit = cfg.duplicates.on && dups >= cfg.duplicates.limit;
        let burst: Vec<(serenity::ChannelId, serenity::MessageId)> = if spam_hit || dup_hit {
            let b = q.iter().filter(|r| r.at.elapsed() <= Duration::from_secs(if spam_hit { 5 } else { 60 }) && (spam_hit || r.hash == h0)).map(|r| (r.channel, r.id)).collect();
            q.clear();
            b
        } else {
            vec![]
        };
        (spam_hit, dup_hit, burst)
    };

    let hit: Option<(&Rule, String)> = if spam_hit {
        Some((&cfg.spam, "Sending messages too fast".into()))
    } else if dup_hit {
        Some((&cfg.duplicates, "Repeating the same message".into()))
    } else if cfg.scam.on && hosts(text).iter().any(|h| is_scam(h)) {
        Some((&cfg.scam, "Scam or phishing link".into()))
    } else if cfg.mentions.on && (msg.mentions.len() + msg.mention_roles.len()) as i64 >= cfg.mentions.limit {
        Some((&cfg.mentions, "Mass mentions".into()))
    } else if let Some(w) = cfg.words.on.then(|| bad_word(text, &cfg.word_list)).flatten() {
        let _ = w;
        Some((&cfg.words, "Blocked word".into()))
    } else if cfg.invites.on && {
        let codes = invite_codes(text);
        let mut foreign = false;
        for c in &codes {
            if !own_invite(ctx, guild, c).await {
                foreign = true;
                break;
            }
        }
        foreign
    } {
        Some((&cfg.invites, "Invite links aren't allowed".into()))
    } else if cfg.links.on
        && hosts(text).iter().any(|h| !suffixes(h).any(|s| cfg.allowed_domains.iter().any(|d| d == s)) && !suffixes(h).any(|s| OFFICIAL.contains(&s)))
    {
        Some((&cfg.links, "Links aren't allowed here".into()))
    } else if cfg.caps.on && caps_percent(text).is_some_and(|p| p >= cfg.caps.limit) {
        Some((&cfg.caps, "Too many capital letters".into()))
    } else if cfg.emoji.on && emoji_count(text) >= cfg.emoji.limit {
        Some((&cfg.emoji, "Too many emojis".into()))
    } else if cfg.zalgo.on && is_zalgo(text) {
        Some((&cfg.zalgo, "Glitched text".into()))
    } else {
        None
    };
    let Some((rule, why)) = hit else { return false };

    // delete the message (and the burst for spam)
    let mut by_channel: HashMap<serenity::ChannelId, Vec<serenity::MessageId>> = HashMap::new();
    by_channel.entry(msg.channel_id).or_default().push(msg.id);
    for (c, id) in burst {
        let v = by_channel.entry(c).or_default();
        if !v.contains(&id) {
            v.push(id);
        }
    }
    for (c, ids) in by_channel {
        let _ = if ids.len() == 1 { c.delete_message(&ctx.http, ids[0]).await } else { c.delete_messages(&ctx.http, &ids).await };
    }

    let cooled = {
        let mut cd = COOLDOWN.lock().unwrap();
        let fresh = cd.get(&key).is_none_or(|t| t.elapsed() > Duration::from_secs(15));
        if fresh {
            cd.insert(key, Instant::now());
        }
        if cd.len() > 20000 {
            cd.retain(|_, t| t.elapsed() < Duration::from_secs(60));
        }
        fresh
    };
    if !cooled {
        return true;
    }

    let notice = format!("<@{}> your message was removed: {why}.", msg.author.id);
    if let Ok(m) = msg
        .channel_id
        .send_message(&ctx.http, serenity::CreateMessage::new().content(notice).allowed_mentions(serenity::CreateAllowedMentions::new().users(vec![msg.author.id])))
        .await
    {
        let http = ctx.http.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(6)).await;
            let _ = m.delete(&http).await;
        });
    }

    let act = match rule.action.as_str() {
        "warn" => Some(Act::Warn),
        "timeout" => Some(Act::Timeout(rule.minutes.max(1) * 60)),
        "kick" => Some(Act::Kick),
        "ban" => Some(Act::Ban { secs: 0, delete_days: 1 }),
        _ => None,
    };
    if let Some(act) = act {
        let me: serenity::User = ctx.cache.current_user().clone().into();
        if let Err(e) = moderation::punish(ctx, data, guild, &msg.author, &me, act, &format!("Automod: {why}")).await {
            tracing::warn!("automod punish failed in {guild}: {e}");
        }
    }
    true
}

// ---------- anti-raid ----------

static JOINS: LazyLock<Mutex<HashMap<u64, VecDeque<Instant>>>> = LazyLock::new(Default::default);
/// guild -> (lockdown ends, verification level to restore)
static LOCKDOWN: LazyLock<Mutex<HashMap<u64, (Instant, u8)>>> = LazyLock::new(Default::default);

fn level(n: u8) -> serenity::VerificationLevel {
    match n {
        0 => serenity::VerificationLevel::None,
        1 => serenity::VerificationLevel::Low,
        2 => serenity::VerificationLevel::Medium,
        3 => serenity::VerificationLevel::High,
        _ => serenity::VerificationLevel::Higher,
    }
}

async fn alert(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, cfg: &AutomodConfig, embed: serenity::CreateEmbed, buttons: Vec<serenity::CreateButton>) {
    let ch = if cfg.raid.alert_channel_id.is_empty() { moderation::config(data, guild.get()).await.log_channel_id } else { cfg.raid.alert_channel_id.clone() };
    let Ok(ch) = ch.parse::<u64>() else { return };
    let mut m = serenity::CreateMessage::new().embed(embed);
    if !buttons.is_empty() {
        m = m.components(vec![serenity::CreateActionRow::Buttons(buttons)]);
    }
    let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, m).await;
}

/// Returns true when the member was removed or held back, so no welcome is sent.
pub async fn on_join(ctx: &serenity::Context, data: &Data, member: &serenity::Member) -> Result<bool> {
    if member.user.bot {
        return Ok(false);
    }
    let guild = member.guild_id;
    let cfg = config(data, guild.get()).await;
    if !cfg.enabled {
        return Ok(false);
    }
    let me: serenity::User = ctx.cache.current_user().clone().into();

    let locked = LOCKDOWN.lock().unwrap().get(&guild.get()).is_some_and(|(until, _)| *until > Instant::now());
    if locked && cfg.raid.kick_during {
        let _ = moderation::punish(ctx, data, guild, &member.user, &me, Act::Kick, "Anti-raid: joined during a lockdown").await;
        return Ok(true);
    }

    if cfg.raid.min_account_days > 0 {
        let age_days = (crate::db::now() - member.user.id.created_at().unix_timestamp()) / 86400;
        if age_days < cfg.raid.min_account_days {
            let why = format!("Account is younger than {} days", cfg.raid.min_account_days);
            let act = match cfg.raid.young_action.as_str() {
                "ban" => Act::Ban { secs: 0, delete_days: 0 },
                "timeout" => Act::Timeout(((cfg.raid.min_account_days - age_days) * 86400).clamp(60, 28 * 86400)),
                _ => Act::Kick,
            };
            let _ = moderation::punish(ctx, data, guild, &member.user, &me, act, &why).await;
            return Ok(true);
        }
    }

    if !cfg.raid.on || locked {
        return Ok(false);
    }
    let spike = {
        let mut j = JOINS.lock().unwrap();
        let q = j.entry(guild.get()).or_default();
        q.push_back(Instant::now());
        let window = Duration::from_secs(cfg.raid.seconds as u64);
        while q.front().is_some_and(|t| t.elapsed() > window) {
            q.pop_front();
        }
        let hit = q.len() as i64 >= cfg.raid.joins;
        if hit {
            q.clear();
        }
        hit
    };
    if spike {
        start_lockdown(ctx, data, guild, &cfg).await;
    }
    Ok(false)
}

async fn start_lockdown(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, cfg: &AutomodConfig) {
    let prev = ctx.cache.guild(guild).map(|g| u8::from(g.verification_level)).unwrap_or(1);
    let mins = cfg.raid.lockdown_minutes as u64;
    LOCKDOWN.lock().unwrap().insert(guild.get(), (Instant::now() + Duration::from_secs(mins * 60), prev));
    let res = guild.edit(&ctx.http, serenity::EditGuild::new().verification_level(serenity::VerificationLevel::Higher).audit_log_reason("Kev anti-raid lockdown")).await;
    let mut text = format!("{} members joined within {} seconds. Verification is set to the highest level for {} minutes.", cfg.raid.joins, cfg.raid.seconds, mins);
    if res.is_err() {
        text = format!("{} members joined within {} seconds, but I couldn't raise the verification level. Give Kev the Manage Server permission.", cfg.raid.joins, cfg.raid.seconds);
    }
    if cfg.raid.kick_during {
        text.push_str(" Anyone who joins now gets kicked.");
    }
    let embed = serenity::CreateEmbed::new().title("Raid detected").description(text).color(0xEF5350).timestamp(serenity::Timestamp::now());
    alert(ctx, data, guild, cfg, embed, vec![serenity::CreateButton::new("automod:unlock").label("End lockdown").style(serenity::ButtonStyle::Secondary)]).await;
    let (ctx2, data2) = (ctx.clone(), data.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(mins * 60)).await;
        let due = LOCKDOWN.lock().unwrap().get(&guild.get()).is_some_and(|(until, _)| *until <= Instant::now());
        if due {
            end_lockdown(&ctx2, &data2, guild, None).await;
        }
    });
}

async fn end_lockdown(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, by: Option<serenity::UserId>) -> bool {
    let Some((_, prev)) = LOCKDOWN.lock().unwrap().remove(&guild.get()) else { return false };
    let _ = guild.edit(&ctx.http, serenity::EditGuild::new().verification_level(level(prev)).audit_log_reason("Kev lockdown ended")).await;
    let cfg = config(data, guild.get()).await;
    let text = match by {
        Some(u) => format!("<@{u}> ended the lockdown. Verification is back to normal."),
        None => "The lockdown ended. Verification is back to normal.".into(),
    };
    alert(ctx, data, guild, &cfg, serenity::CreateEmbed::new().description(text).color(0x66BB6A), vec![]).await;
    true
}

pub async fn on_unlock_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<()> {
    let reply = |t: &str| serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(t).ephemeral(true));
    let Some(guild) = c.guild_id else { return Ok(()) };
    let allowed = c.member.as_ref().and_then(|m| m.permissions).is_some_and(|p| p.intersects(serenity::Permissions::ADMINISTRATOR | serenity::Permissions::MANAGE_GUILD));
    if !allowed {
        c.create_response(&ctx.http, reply("Only people with Manage Server can end a lockdown.")).await?;
        return Ok(());
    }
    let ended = end_lockdown(ctx, data, guild, Some(c.user.id)).await;
    c.create_response(&ctx.http, reply(if ended { "Lockdown ended." } else { "There's no lockdown running." })).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_words() {
        assert_eq!(hosts("check https://www.Example.com/x?y and http://a.b.co:80/"), vec!["example.com", "a.b.co"]);
        assert_eq!(hosts("ftp://nope.com not.a.link"), Vec::<String>::new());
        assert_eq!(invite_codes("join discord.gg/abc-1 or https://discord.com/invite/XyZ"), vec!["abc-1", "xyz"]);
        assert!(is_scam("free-nitro-gift.com"));
        assert!(!is_scam("discord.com"));
        assert!(!is_scam("cdn.discordapp.com"));
        assert!(glob("bad*", "badword") && glob("*bad", "verybad") && glob("b*d", "bread") && !glob("bad", "badly"));
        assert_eq!(bad_word("you are B4D", &["bad".into()]), Some("bad".into()));
        assert_eq!(bad_word("nothing here", &["bad".into()]), None);
        assert_eq!(caps_percent("HELLO EVERYONE HERE"), Some(100));
        assert_eq!(emoji_count("hi \u{1F600}\u{1F600} <:x:1>"), 3);
    }
}
