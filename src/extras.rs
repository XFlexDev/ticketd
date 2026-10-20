//! Utility: info commands, AFK, sticky messages, starboard and suggestions.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::{Context, Data, Error};

const GREEN: u32 = 0x66BB6A;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct UtilityConfig {
    pub starboard_on: bool,
    pub starboard_channel_id: String,
    pub star_emoji: String,
    pub star_threshold: i64,
    pub suggestions_channel_id: String,
}

impl Default for UtilityConfig {
    fn default() -> Self {
        Self { starboard_on: false, starboard_channel_id: String::new(), star_emoji: "⭐".into(), star_threshold: 3, suggestions_channel_id: String::new() }
    }
}

impl UtilityConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        self.star_emoji = self.star_emoji.trim().chars().take(80).collect();
        if self.star_emoji.is_empty() {
            self.star_emoji = "⭐".into();
        }
        self.star_threshold = self.star_threshold.clamp(1, 100);
        for c in [&mut self.starboard_channel_id, &mut self.suggestions_channel_id] {
            if c.parse::<u64>().is_err() {
                c.clear();
            }
        }
        if self.starboard_on && self.starboard_channel_id.is_empty() {
            return Err("Pick a starboard channel".into());
        }
        Ok(())
    }
}

static CACHE: LazyLock<Mutex<HashMap<u64, (Instant, Arc<UtilityConfig>)>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CACHE.lock().unwrap().remove(&guild);
}

pub async fn config(data: &Data, guild: u64) -> Arc<UtilityConfig> {
    if let Some((at, c)) = CACHE.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let raw = data.db.module_config(guild, "utility").await.ok().flatten();
    let c = Arc::new(raw.and_then(|r| serde_json::from_str::<UtilityConfig>(&r).ok()).unwrap_or_default());
    CACHE.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

async fn private(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(poise::CreateReply::default().content(text).ephemeral(true)).await?;
    Ok(())
}

// ---------- info commands ----------

/// Info about a member
#[poise::command(slash_command, guild_only)]
pub async fn userinfo(ctx: Context<'_>, #[description = "Who, you by default"] user: Option<serenity::User>) -> Result<(), Error> {
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let full = ctx.http().get_user(user.id).await.unwrap_or(user.clone());
    let member = match ctx.guild_id() {
        Some(g) => g.member(ctx, user.id).await.ok(),
        None => None,
    };
    let mut e = serenity::CreateEmbed::new()
        .author(serenity::CreateEmbedAuthor::new(full.tag()).icon_url(full.face()))
        .thumbnail(member.as_ref().map(|m| m.face()).unwrap_or_else(|| full.face()))
        .color(full.accent_colour.map(|c| c.0).unwrap_or(GREEN))
        .field("Account made", format!("<t:{}:D> (<t:{}:R>)", user.id.created_at().unix_timestamp(), user.id.created_at().unix_timestamp()), false)
        .footer(serenity::CreateEmbedFooter::new(format!("ID {}", user.id)));
    if let Some(m) = &member {
        if let Some(j) = m.joined_at {
            e = e.field("Joined", format!("<t:{}:D> (<t:{}:R>)", j.unix_timestamp(), j.unix_timestamp()), false);
        }
        if let Some(nick) = &m.nick {
            e = e.field("Nickname", nick, true);
        }
        if !m.roles.is_empty() {
            let roles: Vec<String> = m.roles.iter().take(20).map(|r| format!("<@&{r}>")).collect();
            let more = m.roles.len().saturating_sub(20);
            e = e.field(format!("Roles ({})", m.roles.len()), if more > 0 { format!("{} and {more} more", roles.join(" ")) } else { roles.join(" ") }, false);
        }
        if let Some(t) = m.communication_disabled_until.filter(|t| t.unix_timestamp() > crate::db::now()) {
            e = e.field("Timed out until", format!("<t:{}:f>", t.unix_timestamp()), false);
        }
    }
    if full.bot {
        e = e.description("Bot");
    }
    if let Some(b) = full.banner_url() {
        e = e.image(b);
    }
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Info about this server
#[poise::command(slash_command, guild_only)]
pub async fn serverinfo(ctx: Context<'_>) -> Result<(), Error> {
    let Some(gid) = ctx.guild_id() else { return Ok(()) };
    let e = {
        let Some(g) = ctx.cache().guild(gid) else { return private(ctx, "I can't see this server right now, try again in a bit.").await };
        let text = g.channels.values().filter(|c| matches!(c.kind, serenity::ChannelType::Text | serenity::ChannelType::News)).count();
        let voice = g.channels.values().filter(|c| matches!(c.kind, serenity::ChannelType::Voice | serenity::ChannelType::Stage)).count();
        let made = gid.created_at().unix_timestamp();
        let mut e = serenity::CreateEmbed::new()
            .title(&g.name)
            .color(GREEN)
            .field("Owner", format!("<@{}>", g.owner_id), true)
            .field("Members", g.member_count.to_string(), true)
            .field("Boosts", format!("{} (level {})", g.premium_subscription_count.unwrap_or(0), u8::from(g.premium_tier)), true)
            .field("Channels", format!("{text} text, {voice} voice"), true)
            .field("Roles", g.roles.len().to_string(), true)
            .field("Emojis", g.emojis.len().to_string(), true)
            .field("Made", format!("<t:{made}:D> (<t:{made}:R>)"), false)
            .footer(serenity::CreateEmbedFooter::new(format!("ID {gid}")));
        if let Some(d) = &g.description {
            e = e.description(d);
        }
        if let Some(i) = g.icon_url() {
            e = e.thumbnail(i);
        }
        if let Some(b) = g.banner_url() {
            e = e.image(b);
        }
        e
    };
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Show someone's avatar
#[poise::command(slash_command)]
pub async fn avatar(ctx: Context<'_>, #[description = "Who, you by default"] user: Option<serenity::User>) -> Result<(), Error> {
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let server = match ctx.guild_id() {
        Some(g) => g.member(ctx, user.id).await.ok().and_then(|m| m.avatar_url()),
        None => None,
    };
    let main = user.face().replace("?size=1024", "") + "?size=1024";
    let mut e = serenity::CreateEmbed::new().title(format!("{}'s avatar", user.display_name())).image(server.clone().unwrap_or(main.clone())).color(GREEN);
    if server.is_some() {
        e = e.description(format!("Server avatar. [Main avatar]({main})"));
    }
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Show someone's profile banner
#[poise::command(slash_command)]
pub async fn banner(ctx: Context<'_>, #[description = "Who, you by default"] user: Option<serenity::User>) -> Result<(), Error> {
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let full = ctx.http().get_user(user.id).await?;
    match full.banner_url() {
        Some(b) => {
            ctx.send(poise::CreateReply::default().embed(serenity::CreateEmbed::new().title(format!("{}'s banner", full.display_name())).image(b).color(GREEN))).await?;
            Ok(())
        }
        None => private(ctx, format!("{} has no banner.", full.display_name())).await,
    }
}

/// Info about a role
#[poise::command(slash_command, guild_only)]
pub async fn roleinfo(ctx: Context<'_>, #[description = "The role"] role: serenity::Role) -> Result<(), Error> {
    let members = ctx.guild_id().and_then(|g| ctx.cache().guild(g).map(|g| g.members.values().filter(|m| m.roles.contains(&role.id)).count()));
    let perms = role.permissions.get_permission_names();
    let made = role.id.created_at().unix_timestamp();
    let e = serenity::CreateEmbed::new()
        .title(&role.name)
        .color(if role.colour.0 == 0 { GREEN } else { role.colour.0 })
        .field("Color", if role.colour.0 == 0 { "None".into() } else { format!("#{}", role.colour.hex()) }, true)
        .field("Members", members.map(|n| n.to_string()).unwrap_or_else(|| "?".into()), true)
        .field("Position", role.position.to_string(), true)
        .field("Shown separately", if role.hoist { "Yes" } else { "No" }, true)
        .field("Mentionable", if role.mentionable { "Yes" } else { "No" }, true)
        .field("Made", format!("<t:{made}:D>"), true)
        .field("Permissions", if perms.is_empty() { "None".to_string() } else { perms.join(", ").chars().take(1000).collect() }, false)
        .footer(serenity::CreateEmbedFooter::new(format!("ID {}", role.id)));
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

// ---------- AFK ----------

static AFK: LazyLock<Mutex<HashMap<(u64, u64), (String, i64)>>> = LazyLock::new(Default::default);
static AFK_NOTICE: LazyLock<Mutex<HashMap<(u64, u64), Instant>>> = LazyLock::new(Default::default);

/// Tell people you're away when they mention you
#[poise::command(slash_command, guild_only)]
pub async fn afk(ctx: Context<'_>, #[description = "Why, like sleeping or at school"] reason: Option<String>) -> Result<(), Error> {
    let Some(g) = ctx.guild_id() else { return Ok(()) };
    let reason: String = reason.unwrap_or_default().trim().chars().take(200).collect();
    let since = crate::db::now();
    AFK.lock().unwrap().insert((g.get(), ctx.author().id.get()), (reason.clone(), since));
    ctx.data().db.set_afk(g.get(), ctx.author().id.get(), &reason, since).await?;
    let text = if reason.is_empty() { "You're AFK now. It turns off when you send a message.".to_string() } else { format!("You're AFK now: {reason}. It turns off when you send a message.") };
    private(ctx, text).await
}

// ---------- sticky ----------

/// channel -> (guild, text, last sticky message)
static STICKY: LazyLock<Mutex<HashMap<u64, (u64, String, u64)>>> = LazyLock::new(Default::default);
static STICKY_GEN: LazyLock<Mutex<HashMap<u64, u64>>> = LazyLock::new(Default::default);

#[poise::command(slash_command, guild_only, subcommands("sticky_set", "sticky_clear"), rename = "sticky", default_member_permissions = "MANAGE_MESSAGES", required_permissions = "MANAGE_MESSAGES")]
pub async fn sticky(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Keep a message at the bottom of this channel
#[poise::command(slash_command, guild_only, rename = "set")]
pub async fn sticky_set(ctx: Context<'_>, #[description = "What the sticky message says"] text: String) -> Result<(), Error> {
    let Some(g) = ctx.guild_id() else { return Ok(()) };
    let text: String = text.trim().chars().take(1800).collect();
    let ch = ctx.channel_id();
    let old = STICKY.lock().unwrap().get(&ch.get()).map(|s| s.2);
    if let Some(old) = old.filter(|o| *o != 0) {
        let _ = ch.delete_message(ctx.http(), serenity::MessageId::new(old)).await;
    }
    let sent = ch.send_message(ctx.http(), sticky_message(&text)).await;
    let last = match sent {
        Ok(m) => m.id.get(),
        Err(_) => return private(ctx, "I can't post here. I need Send Messages and Embed Links.").await,
    };
    STICKY.lock().unwrap().insert(ch.get(), (g.get(), text.clone(), last));
    ctx.data().db.set_sticky(ch.get(), g.get(), &text, last).await?;
    private(ctx, "Sticky message set. It stays at the bottom of this channel.").await
}

/// Remove this channel's sticky message
#[poise::command(slash_command, guild_only, rename = "clear")]
pub async fn sticky_clear(ctx: Context<'_>) -> Result<(), Error> {
    let ch = ctx.channel_id();
    let Some((_, _, last)) = STICKY.lock().unwrap().remove(&ch.get()) else { return private(ctx, "This channel has no sticky message.").await };
    if last != 0 {
        let _ = ch.delete_message(ctx.http(), serenity::MessageId::new(last)).await;
    }
    ctx.data().db.clear_sticky(ch.get()).await?;
    private(ctx, "Sticky message removed.").await
}

fn sticky_message(text: &str) -> serenity::CreateMessage {
    serenity::CreateMessage::new().embed(serenity::CreateEmbed::new().description(text).color(GREEN)).allowed_mentions(serenity::CreateAllowedMentions::new())
}

fn bump_sticky(ctx: &serenity::Context, data: &Data, channel: u64) {
    if !STICKY.lock().unwrap().contains_key(&channel) {
        return;
    }
    let round = {
        let mut g = STICKY_GEN.lock().unwrap();
        let e = g.entry(channel).or_insert(0);
        *e += 1;
        *e
    };
    let (ctx, db) = (ctx.clone(), data.db.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        if STICKY_GEN.lock().unwrap().get(&channel) != Some(&round) {
            return;
        }
        let Some((guild, text, last)) = STICKY.lock().unwrap().get(&channel).cloned() else { return };
        let ch = serenity::ChannelId::new(channel);
        if last != 0 {
            let _ = ch.delete_message(&ctx.http, serenity::MessageId::new(last)).await;
        }
        if let Ok(m) = ch.send_message(&ctx.http, sticky_message(&text)).await {
            if let Some(s) = STICKY.lock().unwrap().get_mut(&channel) {
                s.2 = m.id.get();
            }
            let _ = db.set_sticky(channel, guild, &text, m.id.get()).await;
        }
    });
}

pub async fn on_message(ctx: &serenity::Context, msg: &serenity::Message, data: &Data) {
    if msg.author.bot {
        return;
    }
    let Some(guild) = msg.guild_id.map(|g| g.get()) else { return };
    bump_sticky(ctx, data, msg.channel_id.get());
    let me = msg.author.id.get();
    let was_afk = AFK.lock().unwrap().remove(&(guild, me));
    if was_afk.is_some() {
        let _ = data.db.clear_afk(guild, me).await;
        if let Ok(m) = msg.reply(&ctx.http, "Welcome back, your AFK is off.").await {
            let http = ctx.http.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(8)).await;
                let _ = m.delete(&http).await;
            });
        }
    }
    let notes: Vec<String> = {
        let afk = AFK.lock().unwrap();
        let mut seen = AFK_NOTICE.lock().unwrap();
        msg.mentions
            .iter()
            .filter(|u| u.id.get() != me)
            .filter_map(|u| afk.get(&(guild, u.id.get())).map(|a| (u, a)))
            .filter(|(u, _)| {
                let key = (msg.channel_id.get(), u.id.get());
                let fresh = seen.get(&key).is_none_or(|t| t.elapsed() > Duration::from_secs(30));
                if fresh {
                    seen.insert(key, Instant::now());
                }
                fresh
            })
            .take(5)
            .map(|(u, (reason, since))| if reason.is_empty() { format!("{} is AFK (since <t:{since}:R>)", u.display_name()) } else { format!("{} is AFK: {reason} (since <t:{since}:R>)", u.display_name()) })
            .collect()
    };
    if !notes.is_empty() {
        let _ = msg.channel_id.send_message(&ctx.http, serenity::CreateMessage::new().content(notes.join("\n")).reference_message(msg).allowed_mentions(serenity::CreateAllowedMentions::new())).await;
    }
}

// ---------- starboard ----------

fn emoji_matches(r: &serenity::ReactionType, want: &str) -> bool {
    match r {
        serenity::ReactionType::Unicode(s) => s.trim_end_matches('\u{fe0f}') == want.trim_end_matches('\u{fe0f}'),
        serenity::ReactionType::Custom { id, .. } => want.contains(&id.to_string()),
        _ => false,
    }
}

pub async fn on_reaction(ctx: &serenity::Context, data: &Data, r: &serenity::Reaction) {
    let Some(guild) = r.guild_id.map(|g| g.get()) else { return };
    let cfg = config(data, guild).await;
    if !cfg.starboard_on || !emoji_matches(&r.emoji, &cfg.star_emoji) {
        return;
    }
    let Ok(board) = cfg.starboard_channel_id.parse::<u64>() else { return };
    if r.channel_id.get() == board {
        return;
    }
    let Ok(msg) = r.channel_id.message(&ctx.http, r.message_id).await else { return };
    if msg.author.bot {
        return;
    }
    let count = msg.reactions.iter().filter(|x| emoji_matches(&x.reaction_type, &cfg.star_emoji)).map(|x| x.count).sum::<u64>() as i64;
    let existing = data.db.star_entry(msg.id.get()).await.ok().flatten();
    if existing.is_none() && count < cfg.star_threshold {
        return;
    }
    let header = format!("{} **{count}** in <#{}>", cfg.star_emoji, msg.channel_id);
    let board = serenity::ChannelId::new(board);
    if let Some(entry) = existing {
        let _ = board.edit_message(&ctx.http, serenity::MessageId::new(entry), serenity::EditMessage::new().content(header)).await;
        return;
    }
    let mut e = serenity::CreateEmbed::new()
        .author(serenity::CreateEmbedAuthor::new(msg.author.display_name()).icon_url(msg.author.face()))
        .color(0xFFC94D)
        .timestamp(msg.timestamp)
        .field("Source", format!("[Jump to message]({})", msg.link()), false);
    if !msg.content.is_empty() {
        e = e.description(msg.content.chars().take(4000).collect::<String>());
    }
    if let Some(img) = msg.attachments.iter().find(|a| a.content_type.as_deref().is_some_and(|t| t.starts_with("image/"))) {
        e = e.image(&img.url);
    }
    if let Ok(posted) = board.send_message(&ctx.http, serenity::CreateMessage::new().content(header).embed(e).allowed_mentions(serenity::CreateAllowedMentions::new())).await {
        let _ = data.db.set_star_entry(msg.id.get(), guild, posted.id.get()).await;
    }
}

// ---------- suggestions ----------

fn suggestion_buttons(up: i64, down: i64, open: bool) -> Vec<serenity::CreateActionRow> {
    if !open {
        return vec![];
    }
    vec![serenity::CreateActionRow::Buttons(vec![
        serenity::CreateButton::new("suggest:up").label(format!("Upvote {up}")).style(serenity::ButtonStyle::Success),
        serenity::CreateButton::new("suggest:down").label(format!("Downvote {down}")).style(serenity::ButtonStyle::Danger),
        serenity::CreateButton::new("suggest:approve").label("Approve").style(serenity::ButtonStyle::Secondary),
        serenity::CreateButton::new("suggest:deny").label("Deny").style(serenity::ButtonStyle::Secondary),
    ])]
}

/// Suggest something for this server
#[poise::command(slash_command, guild_only)]
pub async fn suggest(ctx: Context<'_>, #[description = "Your idea"] idea: String) -> Result<(), Error> {
    let Some(g) = ctx.guild_id() else { return Ok(()) };
    let cfg = config(ctx.data(), g.get()).await;
    let Ok(ch) = cfg.suggestions_channel_id.parse::<u64>() else {
        return private(ctx, "Suggestions aren't set up here. An admin can pick a channel in the dashboard under Utility.").await;
    };
    let e = serenity::CreateEmbed::new()
        .author(serenity::CreateEmbedAuthor::new(ctx.author().display_name()).icon_url(ctx.author().face()))
        .description(idea.chars().take(2000).collect::<String>())
        .color(GREEN)
        .footer(serenity::CreateEmbedFooter::new(format!("Suggested by {}", ctx.author().id)));
    match serenity::ChannelId::new(ch).send_message(ctx.http(), serenity::CreateMessage::new().embed(e).components(suggestion_buttons(0, 0, true))).await {
        Ok(m) => private(ctx, format!("Posted: {}", m.link())).await,
        Err(_) => private(ctx, format!("I can't post in <#{ch}>.")).await,
    }
}

pub async fn on_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<(), Error> {
    let what = c.data.custom_id.trim_start_matches("suggest:");
    let ephemeral = |t: &str| serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(t).ephemeral(true));
    let mid = c.message.id.get();
    match what {
        "up" | "down" => {
            data.db.vote_suggestion(mid, c.user.id.get(), if what == "up" { 1 } else { -1 }).await?;
            let (up, down) = data.db.suggestion_counts(mid).await.unwrap_or((0, 0));
            c.create_response(&ctx.http, serenity::CreateInteractionResponse::UpdateMessage(serenity::CreateInteractionResponseMessage::new().components(suggestion_buttons(up, down, true)))).await?;
        }
        "approve" | "deny" => {
            let staff = c.member.as_ref().and_then(|m| m.permissions).is_some_and(|p| p.manage_guild() || p.administrator());
            if !staff {
                c.create_response(&ctx.http, ephemeral("Only staff with Manage Server can do that.")).await?;
                return Ok(());
            }
            let (up, down) = data.db.suggestion_counts(mid).await.unwrap_or((0, 0));
            let old = c.message.embeds.first().cloned();
            let approved = what == "approve";
            let mut e = old.map(serenity::CreateEmbed::from).unwrap_or_default();
            e = e
                .color(if approved { GREEN } else { 0xEF5350 })
                .field(if approved { "Approved" } else { "Denied" }, format!("by <@{}> · {up} up, {down} down", c.user.id), false);
            c.create_response(&ctx.http, serenity::CreateInteractionResponse::UpdateMessage(serenity::CreateInteractionResponseMessage::new().embed(e).components(vec![]))).await?;
        }
        _ => {}
    }
    Ok(())
}

pub async fn startup(data: Data) {
    for (ch, guild, text, last) in data.db.stickies().await.unwrap_or_default() {
        STICKY.lock().unwrap().insert(ch, (guild, text, last));
    }
    for (guild, user, reason, since) in data.db.afks().await.unwrap_or_default() {
        AFK.lock().unwrap().insert((guild, user), (reason, since));
    }
}
