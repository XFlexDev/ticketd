//! Moderation: ban, kick, timeout, warn, purge, lock. Every action is a numbered case in the mod log.

use anyhow::Result;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::{Context, Data, Error};

#[derive(Serialize, Deserialize, Clone)]
pub struct ModConfig {
    #[serde(default)]
    pub log_channel_id: String,
    /// DM the member the reason before acting.
    #[serde(default = "yes")]
    pub dm: bool,
    /// Automatic punishment when a member reaches N active warnings.
    #[serde(default)]
    pub escalation: Vec<Step>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Step {
    pub warns: i64,
    pub action: String,
    #[serde(default)]
    pub minutes: i64,
}

fn yes() -> bool {
    true
}

impl Default for ModConfig {
    fn default() -> Self {
        Self {
            log_channel_id: String::new(),
            dm: true,
            escalation: vec![
                Step { warns: 3, action: "timeout".into(), minutes: 60 },
                Step { warns: 5, action: "kick".into(), minutes: 0 },
            ],
        }
    }
}

impl ModConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        if !self.log_channel_id.is_empty() && self.log_channel_id.parse::<u64>().is_err() {
            return Err("Bad log channel".into());
        }
        self.escalation.retain(|s| s.warns > 0);
        self.escalation.truncate(10);
        for s in &mut self.escalation {
            if !matches!(s.action.as_str(), "timeout" | "kick" | "ban") {
                return Err("Escalation actions are timeout, kick or ban".into());
            }
            s.warns = s.warns.min(100);
            s.minutes = s.minutes.clamp(0, 40320);
            if s.action == "timeout" && s.minutes == 0 {
                s.minutes = 60;
            }
        }
        self.escalation.sort_by_key(|s| s.warns);
        self.escalation.dedup_by_key(|s| s.warns);
        Ok(())
    }
}

pub async fn config(data: &Data, guild: u64) -> ModConfig {
    data.db.module_config(guild, "moderation").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default()
}

/// "10m", "2h", "7d", "1w", "90s", plain number = minutes.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim().to_lowercase().replace(' ', "");
    if s.is_empty() {
        return None;
    }
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: i64 = n.parse().ok()?;
    let mul = match unit {
        "" | "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86400,
        "w" | "week" | "weeks" => 604800,
        _ => return None,
    };
    (n > 0).then(|| n.saturating_mul(mul))
}

pub fn fmt_duration(secs: i64) -> String {
    let (n, u) = if secs % 604800 == 0 {
        (secs / 604800, "week")
    } else if secs % 86400 == 0 {
        (secs / 86400, "day")
    } else if secs % 3600 == 0 {
        (secs / 3600, "hour")
    } else if secs % 60 == 0 {
        (secs / 60, "minute")
    } else {
        (secs, "second")
    };
    format!("{n} {u}{}", if n == 1 { "" } else { "s" })
}

#[derive(Clone, Copy, PartialEq)]
pub enum Act {
    Warn,
    Timeout(i64),
    Untimeout,
    Kick,
    Ban { secs: i64, delete_days: u8 },
    Unban,
}

impl Act {
    fn name(&self) -> &'static str {
        match self {
            Act::Warn => "warn",
            Act::Timeout(_) => "timeout",
            Act::Untimeout => "untimeout",
            Act::Kick => "kick",
            Act::Ban { .. } => "ban",
            Act::Unban => "unban",
        }
    }
    fn past(&self) -> &'static str {
        match self {
            Act::Warn => "warned",
            Act::Timeout(_) => "timed out",
            Act::Untimeout => "no longer timed out",
            Act::Kick => "kicked",
            Act::Ban { .. } => "banned",
            Act::Unban => "unbanned",
        }
    }
    fn secs(&self) -> i64 {
        match self {
            Act::Timeout(s) => *s,
            Act::Ban { secs, .. } => *secs,
            _ => 0,
        }
    }
    fn color(&self) -> u32 {
        match self {
            Act::Warn => 0xFFCA28,
            Act::Timeout(_) => 0xFFA726,
            Act::Kick => 0xFF7043,
            Act::Ban { .. } => 0xEF5350,
            Act::Untimeout | Act::Unban => 0x66BB6A,
        }
    }
}

const MAX_TIMEOUT: i64 = 28 * 86400;

fn top_position(g: &serenity::Guild, roles: &[serenity::RoleId]) -> u16 {
    roles.iter().filter_map(|r| g.roles.get(r)).map(|r| r.position).max().unwrap_or(0)
}

/// Can `actor` act on `target`? Checks owner, self, and role order for both the actor and Kev.
pub async fn check_target(ctx: &serenity::Context, guild: serenity::GuildId, actor: serenity::UserId, target: serenity::UserId) -> Result<(), String> {
    let me = ctx.cache.current_user().id;
    if target == actor {
        return Err("You can't do that to yourself.".into());
    }
    if target == me {
        return Err("I can't do that to myself.".into());
    }
    let target_member = guild.member(ctx, target).await.ok();
    let actor_member = guild.member(ctx, actor).await.ok();
    let my_member = guild.member(ctx, me).await.ok();
    let Some(g) = ctx.cache.guild(guild).map(|g| g.clone()) else { return Ok(()) };
    if target == g.owner_id {
        return Err("That's the server owner.".into());
    }
    let Some(t) = target_member else { return Ok(()) };
    let tp = top_position(&g, &t.roles);
    if actor != g.owner_id && actor != me {
        let ap = actor_member.map(|m| top_position(&g, &m.roles)).unwrap_or(0);
        if ap <= tp {
            return Err("Their highest role is the same as or above yours.".into());
        }
    }
    let mp = my_member.map(|m| top_position(&g, &m.roles)).unwrap_or(0);
    if mp <= tp {
        return Err("Their highest role is above mine. Move Kev's role higher in Server Settings, Roles.".into());
    }
    Ok(())
}

fn friendly(e: &serenity::Error) -> String {
    let raw = e.to_string();
    if raw.contains("Missing Permissions") || raw.contains("Missing Access") || raw.contains("Invalid permissions") {
        "I'm missing permissions for that. Give Kev's role Ban Members, Kick Members, Moderate Members and Manage Messages, and keep it above the people it moderates.".into()
    } else if raw.contains("Unknown Ban") {
        "That user isn't banned.".into()
    } else if raw.contains("Unknown Member") {
        "That user isn't in the server.".into()
    } else {
        raw
    }
}

/// Run a moderation action, record the case, DM and log it. Returns the case number.
pub async fn punish(
    ctx: &serenity::Context,
    data: &Data,
    guild: serenity::GuildId,
    target: &serenity::User,
    actor: &serenity::User,
    act: Act,
    reason: &str,
) -> Result<i64, String> {
    let cfg = config(data, guild.get()).await;
    let reason = if reason.trim().is_empty() { "No reason given".to_string() } else { reason.trim().chars().take(500).collect() };
    let guild_name = guild.name(&ctx.cache).unwrap_or_else(|| "the server".into());

    if cfg.dm && matches!(act, Act::Warn | Act::Timeout(_) | Act::Kick | Act::Ban { .. }) {
        let mut text = format!("You were {} in **{guild_name}**.", act.past());
        if act.secs() > 0 {
            text.push_str(&format!("\nDuration: {}", fmt_duration(act.secs())));
        }
        text.push_str(&format!("\nReason: {reason}"));
        let embed = serenity::CreateEmbed::new().description(text).color(act.color());
        let _ = target.direct_message(&ctx.http, serenity::CreateMessage::new().embed(embed)).await;
    }

    let audit = format!("{}: {}", actor.name, reason).chars().take(500).collect::<String>();
    let res = match act {
        Act::Warn => Ok(()),
        Act::Timeout(secs) => {
            let until = serenity::Timestamp::from_unix_timestamp(crate::db::now() + secs.min(MAX_TIMEOUT)).map_err(|e| e.to_string())?;
            guild
                .edit_member(&ctx.http, target.id, serenity::EditMember::new().disable_communication_until_datetime(until).audit_log_reason(&audit))
                .await
                .map(|_| ())
        }
        Act::Untimeout => guild.edit_member(&ctx.http, target.id, serenity::EditMember::new().enable_communication().audit_log_reason(&audit)).await.map(|_| ()),
        Act::Kick => guild.kick_with_reason(&ctx.http, target.id, &audit).await,
        Act::Ban { delete_days, .. } => guild.ban_with_reason(&ctx.http, target.id, delete_days.min(7), &audit).await,
        Act::Unban => guild.unban(&ctx.http, target.id).await,
    };
    res.map_err(|e| friendly(&e))?;

    if act == Act::Unban || matches!(act, Act::Ban { .. }) {
        let _ = data.db.end_temp_bans(guild.get(), target.id.get()).await;
    }
    let num = data
        .db
        .add_case(guild.get(), target.id.get(), &target.name, actor.id.get(), &actor.name, act.name(), &reason, act.secs())
        .await
        .map_err(|e| e.to_string())?;
    log_case(ctx, &cfg, num, target, actor, act, &reason).await;

    if act == Act::Warn {
        let count = data.db.active_warns(guild.get(), target.id.get()).await.unwrap_or(0);
        if let Some(step) = cfg.escalation.iter().find(|s| s.warns == count) {
            let next = match step.action.as_str() {
                "timeout" => Act::Timeout(step.minutes.max(1) * 60),
                "kick" => Act::Kick,
                _ => Act::Ban { secs: step.minutes * 60, delete_days: 0 },
            };
            let me = ctx.cache.current_user().clone();
            let why = format!("Reached {count} warnings");
            let _ = Box::pin(punish(ctx, data, guild, target, &me.into(), next, &why)).await;
        }
    }
    Ok(num)
}

async fn log_case(ctx: &serenity::Context, cfg: &ModConfig, num: i64, target: &serenity::User, actor: &serenity::User, act: Act, reason: &str) {
    let Ok(ch) = cfg.log_channel_id.parse::<u64>() else { return };
    let title = match act {
        Act::Ban { secs, .. } if secs > 0 => "Temp ban".to_string(),
        _ => {
            let n = act.name();
            n[..1].to_uppercase() + &n[1..]
        }
    };
    let mut e = serenity::CreateEmbed::new()
        .title(format!("Case {num} | {title}"))
        .color(act.color())
        .field("User", format!("<@{}> {}", target.id, target.name), true)
        .field("Moderator", format!("<@{}>", actor.id), true)
        .field("Reason", reason, false)
        .timestamp(serenity::Timestamp::now());
    if act.secs() > 0 {
        e = e.field("Duration", fmt_duration(act.secs()), true);
    }
    let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, serenity::CreateMessage::new().embed(e)).await;
}

/// Lift temp bans that ran out. Runs every minute.
pub async fn sweeper(ctx: serenity::Context, data: Data) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tick.tick().await;
        let Ok(due) = data.db.expired_bans().await else { continue };
        for (guild, num, user) in due {
            let _ = data.db.deactivate_case(guild, num).await;
            let g = serenity::GuildId::new(guild);
            let Ok(u) = serenity::UserId::new(user).to_user(&ctx).await else { continue };
            let me: serenity::User = ctx.cache.current_user().clone().into();
            if let Err(e) = punish(&ctx, &data, g, &u, &me, Act::Unban, &format!("Temp ban from case {num} ended")).await {
                tracing::warn!("temp unban {guild}/{user}: {e}");
            }
        }
    }
}

async fn private(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(poise::CreateReply::default().content(text).ephemeral(true)).await?;
    Ok(())
}

async fn done(ctx: Context<'_>, act: Act, user: &serenity::User, num: i64, reason: &str) -> Result<(), Error> {
    let mut text = format!("**{}** was {}", user.name, act.past());
    if act.secs() > 0 {
        text.push_str(&format!(" for {}", fmt_duration(act.secs())));
    }
    text.push_str(&format!(". Case {num}"));
    if !reason.trim().is_empty() {
        text.push_str(&format!("\nReason: {}", reason.trim()));
    }
    let embed = serenity::CreateEmbed::new().description(text).color(act.color());
    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

async fn run(ctx: Context<'_>, user: &serenity::User, act: Act, reason: Option<String>) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    if !matches!(act, Act::Unban | Act::Untimeout) {
        if let Err(e) = check_target(ctx.serenity_context(), guild, ctx.author().id, user.id).await {
            return private(ctx, e).await;
        }
    }
    ctx.defer().await?;
    let reason = reason.unwrap_or_default();
    match punish(ctx.serenity_context(), ctx.data(), guild, user, ctx.author(), act, &reason).await {
        Ok(num) => done(ctx, act, user, num, &reason).await,
        Err(e) => private(ctx, e).await,
    }
}

/// Ban someone, optionally for a while
#[poise::command(slash_command, guild_only, default_member_permissions = "BAN_MEMBERS", required_permissions = "BAN_MEMBERS")]
pub async fn ban(
    ctx: Context<'_>,
    #[description = "Who to ban"] user: serenity::User,
    #[description = "Why"] reason: Option<String>,
    #[description = "How long, like 1h, 7d or 2w (leave empty for permanent)"] duration: Option<String>,
    #[description = "Delete their messages from the last N days (0 to 7)"]
    #[min = 0]
    #[max = 7]
    delete_days: Option<u8>,
) -> Result<(), Error> {
    let secs = match duration.as_deref().map(parse_duration) {
        Some(None) => return private(ctx, "Couldn't read that duration. Try 30m, 12h, 7d or 2w.").await,
        Some(Some(s)) => s,
        None => 0,
    };
    run(ctx, &user, Act::Ban { secs, delete_days: delete_days.unwrap_or(0) }, reason).await
}

/// Unban someone (paste their user ID)
#[poise::command(slash_command, guild_only, default_member_permissions = "BAN_MEMBERS", required_permissions = "BAN_MEMBERS")]
pub async fn unban(ctx: Context<'_>, #[description = "User or user ID"] user: serenity::User, #[description = "Why"] reason: Option<String>) -> Result<(), Error> {
    run(ctx, &user, Act::Unban, reason).await
}

/// Kick someone from the server
#[poise::command(slash_command, guild_only, default_member_permissions = "KICK_MEMBERS", required_permissions = "KICK_MEMBERS")]
pub async fn kick(ctx: Context<'_>, #[description = "Who to kick"] user: serenity::User, #[description = "Why"] reason: Option<String>) -> Result<(), Error> {
    run(ctx, &user, Act::Kick, reason).await
}

/// Time someone out so they can't talk
#[poise::command(slash_command, guild_only, default_member_permissions = "MODERATE_MEMBERS", required_permissions = "MODERATE_MEMBERS")]
pub async fn timeout(
    ctx: Context<'_>,
    #[description = "Who to time out"] user: serenity::User,
    #[description = "How long, like 10m, 1h or 3d (max 28d)"] duration: String,
    #[description = "Why"] reason: Option<String>,
) -> Result<(), Error> {
    let Some(secs) = parse_duration(&duration) else {
        return private(ctx, "Couldn't read that duration. Try 10m, 1h or 3d.").await;
    };
    if secs > MAX_TIMEOUT {
        return private(ctx, "Discord allows 28 days at most.").await;
    }
    run(ctx, &user, Act::Timeout(secs), reason).await
}

/// Remove someone's timeout
#[poise::command(slash_command, guild_only, default_member_permissions = "MODERATE_MEMBERS", required_permissions = "MODERATE_MEMBERS")]
pub async fn untimeout(ctx: Context<'_>, #[description = "Who"] user: serenity::User, #[description = "Why"] reason: Option<String>) -> Result<(), Error> {
    run(ctx, &user, Act::Untimeout, reason).await
}

/// Warn someone. Warnings can trigger automatic punishments
#[poise::command(slash_command, guild_only, default_member_permissions = "MODERATE_MEMBERS", required_permissions = "MODERATE_MEMBERS")]
pub async fn warn(ctx: Context<'_>, #[description = "Who to warn"] user: serenity::User, #[description = "Why"] reason: String) -> Result<(), Error> {
    run(ctx, &user, Act::Warn, Some(reason)).await
}

/// Remove a warning by its case number
#[poise::command(slash_command, guild_only, default_member_permissions = "MODERATE_MEMBERS", required_permissions = "MODERATE_MEMBERS")]
pub async fn unwarn(ctx: Context<'_>, #[description = "Case number"] case: i64) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let Some(c) = ctx.data().db.case(guild.get(), case).await? else { return private(ctx, "No case with that number.").await };
    if c["action"] != "warn" {
        return private(ctx, "That case isn't a warning.").await;
    }
    ctx.data().db.deactivate_case(guild.get(), case).await?;
    private(ctx, format!("Removed the warning in case {case} from **{}**.", c["user_name"].as_str().unwrap_or(""))).await
}

/// See someone's moderation history
#[poise::command(slash_command, guild_only, default_member_permissions = "MODERATE_MEMBERS", required_permissions = "MODERATE_MEMBERS")]
pub async fn warnings(ctx: Context<'_>, #[description = "Who"] user: serenity::User) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let cases = ctx.data().db.cases(guild.get(), Some(user.id.get()), 15).await?;
    if cases.is_empty() {
        return private(ctx, format!("**{}** has a clean record.", user.name)).await;
    }
    let warns = cases.iter().filter(|c| c["action"] == "warn" && c["active"] == true).count();
    let lines: Vec<String> = cases
        .iter()
        .map(|c| {
            let struck = c["action"] == "warn" && c["active"] == false;
            let line = format!("`{}` **{}** <t:{}:R> {}", c["num"], c["action"].as_str().unwrap_or(""), c["created_at"], c["reason"].as_str().unwrap_or(""));
            if struck { format!("~~{line}~~") } else { line }
        })
        .collect();
    let embed = serenity::CreateEmbed::new()
        .title(format!("{} | {warns} active warning{}", user.name, if warns == 1 { "" } else { "s" }))
        .description(lines.join("\n").chars().take(4000).collect::<String>())
        .color(0x66BB6A);
    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true)).await?;
    Ok(())
}

/// Delete recent messages in this channel
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_MESSAGES", required_permissions = "MANAGE_MESSAGES")]
pub async fn purge(
    ctx: Context<'_>,
    #[description = "How many messages to check (1 to 100)"]
    #[min = 1]
    #[max = 100]
    amount: u8,
    #[description = "Only delete messages from this user"] user: Option<serenity::User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let ch = ctx.channel_id();
    let msgs = ch.messages(ctx, serenity::GetMessages::new().limit(amount)).await.map_err(|e| anyhow::anyhow!(friendly(&e)))?;
    let cutoff = crate::db::now() - 14 * 86400 + 60;
    let ids: Vec<serenity::MessageId> = msgs
        .iter()
        .filter(|m| user.as_ref().is_none_or(|u| m.author.id == u.id))
        .filter(|m| m.timestamp.unix_timestamp() > cutoff)
        .map(|m| m.id)
        .collect();
    let n = ids.len();
    let res = match n {
        0 => Ok(()),
        1 => ch.delete_message(ctx, ids[0]).await,
        _ => ch.delete_messages(ctx, &ids).await,
    };
    if let Err(e) = res {
        return private(ctx, friendly(&e)).await;
    }
    let skipped = msgs.len().saturating_sub(n);
    let mut text = format!("Deleted {n} message{}.", if n == 1 { "" } else { "s" });
    if user.is_none() && skipped > 0 {
        text.push_str(&format!(" Skipped {skipped} older than 14 days, Discord doesn't allow deleting those in bulk."));
    }
    private(ctx, text).await
}

/// Set slowmode for this channel (0 turns it off)
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_CHANNELS", required_permissions = "MANAGE_CHANNELS")]
pub async fn slowmode(
    ctx: Context<'_>,
    #[description = "Seconds between messages, 0 to 21600"]
    #[min = 0]
    #[max = 21600]
    seconds: u16,
) -> Result<(), Error> {
    if let Err(e) = ctx.channel_id().edit(ctx, serenity::EditChannel::new().rate_limit_per_user(seconds)).await {
        return private(ctx, friendly(&e)).await;
    }
    let text = if seconds == 0 { "Slowmode is off.".to_string() } else { format!("Slowmode is on: one message every {}.", fmt_duration(seconds as i64)) };
    ctx.say(text).await?;
    Ok(())
}

async fn set_lock(ctx: Context<'_>, lock: bool) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let everyone = serenity::RoleId::new(guild.get());
    let ch = ctx.channel_id();
    let existing = ch
        .to_channel(ctx)
        .await
        .ok()
        .and_then(|c| c.guild())
        .and_then(|c| c.permission_overwrites.into_iter().find(|o| o.kind == serenity::PermissionOverwriteType::Role(everyone)));
    let (mut allow, mut deny) = existing.map(|o| (o.allow, o.deny)).unwrap_or_default();
    let bits = serenity::Permissions::SEND_MESSAGES | serenity::Permissions::SEND_MESSAGES_IN_THREADS | serenity::Permissions::CREATE_PUBLIC_THREADS;
    if lock {
        allow.remove(bits);
        deny.insert(bits);
    } else {
        deny.remove(bits);
    }
    let ow = serenity::PermissionOverwrite { allow, deny, kind: serenity::PermissionOverwriteType::Role(everyone) };
    if let Err(e) = ch.create_permission(ctx, ow).await {
        return private(ctx, friendly(&e)).await;
    }
    ctx.say(if lock { "This channel is locked. Only staff can talk." } else { "This channel is unlocked." }).await?;
    Ok(())
}

/// Lock this channel so members can't send messages
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_CHANNELS", required_permissions = "MANAGE_CHANNELS")]
pub async fn lock(ctx: Context<'_>) -> Result<(), Error> {
    set_lock(ctx, true).await
}

/// Unlock this channel
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_CHANNELS", required_permissions = "MANAGE_CHANNELS")]
pub async fn unlock(ctx: Context<'_>) -> Result<(), Error> {
    set_lock(ctx, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("7d"), Some(604800));
        assert_eq!(parse_duration("1 week"), Some(604800));
        assert_eq!(parse_duration("15"), Some(900));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("0m"), None);
        assert_eq!(fmt_duration(3600), "1 hour");
        assert_eq!(fmt_duration(1209600), "2 weeks");
    }
}
