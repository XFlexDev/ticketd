//! XP from chatting, levels, reward roles, /rank and /leaderboard.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::{Context, Data, Error};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Reward {
    pub level: i64,
    pub role_id: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct LevelConfig {
    pub enabled: bool,
    pub min_xp: i64,
    pub max_xp: i64,
    pub cooldown: i64,
    /// "current", "channel", "dm" or "off"
    pub announce: String,
    pub announce_channel_id: String,
    pub message: String,
    pub rewards: Vec<Reward>,
    pub stack: bool,
    pub no_xp_roles: Vec<String>,
    pub no_xp_channels: Vec<String>,
}

impl Default for LevelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_xp: 15,
            max_xp: 25,
            cooldown: 60,
            announce: "current".into(),
            announce_channel_id: String::new(),
            message: "GG {mention}, you reached **level {level}**!".into(),
            rewards: vec![],
            stack: true,
            no_xp_roles: vec![],
            no_xp_channels: vec![],
        }
    }
}

impl LevelConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        self.min_xp = self.min_xp.clamp(0, 1000);
        self.max_xp = self.max_xp.clamp(self.min_xp, 1000);
        self.cooldown = self.cooldown.clamp(0, 3600);
        if !matches!(self.announce.as_str(), "current" | "channel" | "dm" | "off") {
            self.announce = "current".into();
        }
        if self.announce_channel_id.parse::<u64>().is_err() {
            self.announce_channel_id.clear();
        }
        self.message = self.message.trim().chars().take(1000).collect();
        self.rewards.retain(|r| r.role_id.parse::<u64>().is_ok() && r.level > 0);
        self.rewards.sort_by_key(|r| r.level);
        self.rewards.truncate(50);
        self.no_xp_roles.retain(|r| r.parse::<u64>().is_ok());
        self.no_xp_channels.retain(|r| r.parse::<u64>().is_ok());
        Ok(())
    }
}

/// XP needed to go from `level` to `level + 1`.
pub fn need(level: i64) -> i64 {
    5 * level * level + 50 * level + 100
}

/// (level, xp into that level) for a total.
pub fn level_of(mut xp: i64) -> (i64, i64) {
    let mut l = 0;
    while xp >= need(l) {
        xp -= need(l);
        l += 1;
    }
    (l, xp)
}

static CONFIGS: LazyLock<Mutex<HashMap<u64, (Instant, Arc<LevelConfig>)>>> = LazyLock::new(Default::default);
static COOLDOWN: LazyLock<Mutex<HashMap<(u64, u64), Instant>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CONFIGS.lock().unwrap().remove(&guild);
}

pub async fn config(data: &Data, guild: u64) -> Arc<LevelConfig> {
    if let Some((at, c)) = CONFIGS.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let c: LevelConfig = data.db.module_config(guild, "leveling").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default();
    let c = Arc::new(c);
    CONFIGS.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

fn roll(min: i64, max: i64) -> i64 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as i64;
    min + n % (max - min + 1)
}

pub async fn on_message(ctx: &serenity::Context, msg: &serenity::Message, data: &Data) {
    let Some(guild) = msg.guild_id else { return };
    if msg.author.bot || msg.webhook_id.is_some() {
        return;
    }
    let cfg = config(data, guild.get()).await;
    if !cfg.enabled || cfg.no_xp_channels.contains(&msg.channel_id.to_string()) {
        return;
    }
    if let Some(m) = &msg.member {
        if m.roles.iter().any(|r| cfg.no_xp_roles.contains(&r.to_string())) {
            return;
        }
    }
    let key = (guild.get(), msg.author.id.get());
    {
        let mut cd = COOLDOWN.lock().unwrap();
        if cd.get(&key).is_some_and(|t| t.elapsed() < Duration::from_secs(cfg.cooldown as u64)) {
            return;
        }
        cd.insert(key, Instant::now());
        if cd.len() > 50_000 {
            cd.retain(|_, t| t.elapsed() < Duration::from_secs(3600));
        }
    }
    let gain = roll(cfg.min_xp, cfg.max_xp);
    let (old, _) = data.db.xp(guild.get(), msg.author.id.get()).await.unwrap_or((0, 0));
    let total = old + gain;
    let name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
    if let Err(e) = data.db.add_xp(guild.get(), msg.author.id.get(), &name, &msg.author.face(), gain).await {
        tracing::warn!("xp: {e}");
        return;
    }
    let (before, _) = level_of(old);
    let (after, _) = level_of(total);
    if after > before {
        level_up(ctx, &cfg, guild, &msg.author, msg.channel_id, after).await;
    }
}

async fn level_up(ctx: &serenity::Context, cfg: &LevelConfig, guild: serenity::GuildId, user: &serenity::User, channel: serenity::ChannelId, level: i64) {
    sync_rewards(ctx, cfg, guild, user.id, level).await;
    if cfg.announce == "off" || cfg.message.is_empty() {
        return;
    }
    let text = cfg
        .message
        .replace("{mention}", &format!("<@{}>", user.id))
        .replace("{user}", user.global_name.as_deref().unwrap_or(&user.name))
        .replace("{level}", &level.to_string());
    let m = serenity::CreateMessage::new().content(text).allowed_mentions(serenity::CreateAllowedMentions::new().users(vec![user.id]));
    let _ = match cfg.announce.as_str() {
        "dm" => user.direct_message(&ctx.http, m).await.map(|_| ()),
        "channel" => match cfg.announce_channel_id.parse::<u64>() {
            Ok(c) => serenity::ChannelId::new(c).send_message(&ctx.http, m).await.map(|_| ()),
            Err(_) => Ok(()),
        },
        _ => channel.send_message(&ctx.http, m).await.map(|_| ()),
    };
}

async fn sync_rewards(ctx: &serenity::Context, cfg: &LevelConfig, guild: serenity::GuildId, user: serenity::UserId, level: i64) {
    let earned: Vec<&Reward> = cfg.rewards.iter().filter(|r| r.level <= level).collect();
    let keep: Vec<&Reward> = if cfg.stack { earned.clone() } else { earned.last().copied().into_iter().collect() };
    for r in &cfg.rewards {
        let Ok(id) = r.role_id.parse::<u64>() else { continue };
        let role = serenity::RoleId::new(id);
        if keep.iter().any(|k| k.role_id == r.role_id) {
            let _ = ctx.http.add_member_role(guild, user, role, Some("Kev level reward")).await;
        } else if !cfg.stack && earned.iter().any(|k| k.role_id == r.role_id) {
            let _ = ctx.http.remove_member_role(guild, user, role, Some("Kev level reward")).await;
        }
    }
}

fn bar(into: i64, need: i64) -> String {
    let filled = ((into as f64 / need as f64) * 16.0).round() as usize;
    format!("`{}{}`", "█".repeat(filled.min(16)), "░".repeat(16 - filled.min(16)))
}

/// Show your level and XP
#[poise::command(slash_command, guild_only)]
pub async fn rank(ctx: Context<'_>, #[description = "Whose rank"] user: Option<serenity::User>) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let cfg = config(ctx.data(), guild.get()).await;
    if !cfg.enabled {
        ctx.send(poise::CreateReply::default().content("Leveling is off in this server. An admin can turn it on in the dashboard.").ephemeral(true)).await?;
        return Ok(());
    }
    let (xp, msgs) = ctx.data().db.xp(guild.get(), user.id.get()).await?;
    let place = ctx.data().db.xp_place(guild.get(), xp).await?;
    let (level, into) = level_of(xp);
    let e = serenity::CreateEmbed::new()
        .author(serenity::CreateEmbedAuthor::new(user.global_name.as_deref().unwrap_or(&user.name)).icon_url(user.face()))
        .color(0x66BB6A)
        .field("Level", level.to_string(), true)
        .field("Rank", format!("#{place}"), true)
        .field("Messages", msgs.to_string(), true)
        .description(format!("{} {into} / {} XP", bar(into, need(level)), need(level)));
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Top members by XP
#[poise::command(slash_command, guild_only)]
pub async fn leaderboard(ctx: Context<'_>) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let rows = ctx.data().db.leaderboard(guild.get(), 10, 0).await?;
    if rows.is_empty() {
        ctx.send(poise::CreateReply::default().content("Nobody has XP yet.").ephemeral(true)).await?;
        return Ok(());
    }
    let lines: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let xp = r["xp"].as_i64().unwrap_or(0);
            format!("`{:>2}` <@{}> level {} · {} XP", i + 1, r["user_id"].as_str().unwrap_or("0"), level_of(xp).0, xp)
        })
        .collect();
    let link = format!("https://kev.kreatix.dev/leaderboard/{}", guild.get());
    let e = serenity::CreateEmbed::new().title("Leaderboard").color(0x66BB6A).description(format!("{}\n\n[Full leaderboard]({link})", lines.join("\n")));
    ctx.send(poise::CreateReply::default().embed(e).allowed_mentions(serenity::CreateAllowedMentions::new())).await?;
    Ok(())
}

/// Give or take XP
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_GUILD", required_permissions = "MANAGE_GUILD")]
pub async fn xp(
    ctx: Context<'_>,
    #[description = "Who"] user: serenity::User,
    #[description = "XP to add, negative takes it away"] amount: i64,
) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let data = ctx.data();
    let (old, _) = data.db.xp(guild.get(), user.id.get()).await?;
    let amount = amount.max(-old).min(10_000_000);
    let name = user.global_name.clone().unwrap_or_else(|| user.name.clone());
    data.db.give_xp(guild.get(), user.id.get(), &name, &user.face(), amount).await?;
    let total = old + amount;
    let cfg = config(data, guild.get()).await;
    let (level, _) = level_of(total);
    if level != level_of(old).0 {
        sync_rewards(ctx.serenity_context(), &cfg, guild, user.id, level).await;
    }
    ctx.send(poise::CreateReply::default().content(format!("**{name}** now has {total} XP (level {level}).")).ephemeral(true)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(level_of(0), (0, 0));
        assert_eq!(level_of(99), (0, 99));
        assert_eq!(level_of(100), (1, 0));
        assert_eq!(level_of(100 + 155), (2, 0));
    }
}
