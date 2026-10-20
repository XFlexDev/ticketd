//! Server logs: message edits and deletes, members, roles, channels, voice, bans.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct LogConfig {
    pub messages: String,
    pub members: String,
    pub server: String,
    pub voice: String,
    pub ignored_channels: Vec<String>,
}

impl LogConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        for c in [&mut self.messages, &mut self.members, &mut self.server, &mut self.voice] {
            if !c.is_empty() && c.parse::<u64>().is_err() {
                c.clear();
            }
        }
        self.ignored_channels.retain(|c| c.parse::<u64>().is_ok());
        self.ignored_channels.truncate(100);
        Ok(())
    }
}

static CONFIGS: LazyLock<Mutex<HashMap<u64, (Instant, Arc<LogConfig>)>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CONFIGS.lock().unwrap().remove(&guild);
}

async fn config(data: &Data, guild: u64) -> Arc<LogConfig> {
    if let Some((at, c)) = CONFIGS.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let c: LogConfig = data.db.module_config(guild, "logging").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default();
    let c = Arc::new(c);
    CONFIGS.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

const RED: u32 = 0xEF5350;
const YELLOW: u32 = 0xFFB74D;
const GREEN: u32 = 0x66BB6A;
const BLUE: u32 = 0x64B5F6;

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 3).collect::<String>() + "..." }
}

async fn send(ctx: &serenity::Context, ch: &str, e: serenity::CreateEmbed) {
    let Ok(ch) = ch.parse::<u64>() else { return };
    let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, serenity::CreateMessage::new().embed(e.timestamp(serenity::Timestamp::now()))).await;
}

fn who(u: &serenity::User) -> serenity::CreateEmbedAuthor {
    serenity::CreateEmbedAuthor::new(&u.name).icon_url(u.face())
}

pub async fn on_event(ctx: &serenity::Context, data: &Data, event: &serenity::FullEvent) {
    use serenity::FullEvent as E;
    let guild = match event {
        E::MessageDelete { guild_id, .. } | E::MessageDeleteBulk { guild_id, .. } => *guild_id,
        E::MessageUpdate { event, .. } => event.guild_id,
        E::GuildMemberAddition { new_member } => Some(new_member.guild_id),
        E::GuildMemberRemoval { guild_id, .. } | E::GuildBanAddition { guild_id, .. } | E::GuildBanRemoval { guild_id, .. } => Some(*guild_id),
        E::GuildMemberUpdate { event, .. } => Some(event.guild_id),
        E::ChannelCreate { channel } => Some(channel.guild_id),
        E::ChannelDelete { channel, .. } => Some(channel.guild_id),
        E::GuildRoleCreate { new } => Some(new.guild_id),
        E::GuildRoleDelete { guild_id, .. } => Some(*guild_id),
        E::VoiceStateUpdate { new, .. } => new.guild_id,
        _ => None,
    };
    let Some(guild) = guild else { return };
    let cfg = config(data, guild.get()).await;
    let ignored = |c: &serenity::ChannelId| cfg.ignored_channels.contains(&c.to_string());

    match event {
        E::MessageDelete { channel_id, deleted_message_id, .. } if !cfg.messages.is_empty() && !ignored(channel_id) => {
            let cached = ctx.cache.message(*channel_id, *deleted_message_id).map(|m| m.clone());
            if cached.as_ref().is_some_and(|m| m.author.bot) {
                return;
            }
            let mut e = serenity::CreateEmbed::new().title("Message deleted").color(RED);
            match cached {
                Some(m) => {
                    e = e.author(who(&m.author)).description(format!("In <#{channel_id}> by <@{}>\n\n{}", m.author.id, cut(&m.content, 3500)));
                    let files: Vec<String> = m.attachments.iter().map(|a| a.filename.clone()).collect();
                    if !files.is_empty() {
                        e = e.field("Attachments", cut(&files.join(", "), 1000), false);
                    }
                }
                None => e = e.description(format!("In <#{channel_id}>. It was sent before Kev started, so the text isn't known.")),
            }
            send(ctx, &cfg.messages, e).await;
        }
        E::MessageDeleteBulk { channel_id, multiple_deleted_messages_ids, .. } if !cfg.messages.is_empty() && !ignored(channel_id) => {
            let e = serenity::CreateEmbed::new().title("Messages purged").color(RED).description(format!("{} messages deleted in <#{channel_id}>", multiple_deleted_messages_ids.len()));
            send(ctx, &cfg.messages, e).await;
        }
        E::MessageUpdate { old_if_available, new, event } if !cfg.messages.is_empty() && !ignored(&event.channel_id) => {
            let Some(new) = new else { return };
            if new.author.bot {
                return;
            }
            let before = old_if_available.as_ref().map(|m| m.content.clone());
            if before.as_deref() == Some(new.content.as_str()) {
                return;
            }
            let e = serenity::CreateEmbed::new()
                .title("Message edited")
                .color(YELLOW)
                .author(who(&new.author))
                .description(format!("In <#{}> by <@{}> [Jump]({})", new.channel_id, new.author.id, new.link()))
                .field("Before", cut(before.as_deref().unwrap_or("Not known"), 1000), false)
                .field("After", cut(&new.content, 1000), false);
            send(ctx, &cfg.messages, e).await;
        }
        E::GuildMemberAddition { new_member } if !cfg.members.is_empty() => {
            let u = &new_member.user;
            let e = serenity::CreateEmbed::new()
                .title("Member joined")
                .color(GREEN)
                .author(who(u))
                .description(format!("<@{}> {}\nAccount created <t:{}:R>", u.id, u.name, u.id.created_at().unix_timestamp()));
            send(ctx, &cfg.members, e).await;
        }
        E::GuildMemberRemoval { user, member_data_if_available, .. } if !cfg.members.is_empty() => {
            let mut e = serenity::CreateEmbed::new().title("Member left").color(RED).author(who(user)).description(format!("<@{}> {}", user.id, user.name));
            if let Some(m) = member_data_if_available {
                if let Some(j) = m.joined_at {
                    e = e.field("Joined", format!("<t:{}:R>", j.unix_timestamp()), true);
                }
                if !m.roles.is_empty() {
                    e = e.field("Roles", cut(&m.roles.iter().map(|r| format!("<@&{r}>")).collect::<Vec<_>>().join(" "), 1000), false);
                }
            }
            send(ctx, &cfg.members, e).await;
        }
        E::GuildMemberUpdate { old_if_available, new, .. } if !cfg.members.is_empty() => {
            let (Some(old), Some(new)) = (old_if_available, new) else { return };
            let mut changes = vec![];
            if old.nick != new.nick {
                changes.push(format!("Nickname: {} to {}", old.nick.as_deref().unwrap_or("none"), new.nick.as_deref().unwrap_or("none")));
            }
            let added: Vec<String> = new.roles.iter().filter(|r| !old.roles.contains(r)).map(|r| format!("<@&{r}>")).collect();
            let removed: Vec<String> = old.roles.iter().filter(|r| !new.roles.contains(r)).map(|r| format!("<@&{r}>")).collect();
            if !added.is_empty() {
                changes.push(format!("Roles added: {}", added.join(" ")));
            }
            if !removed.is_empty() {
                changes.push(format!("Roles removed: {}", removed.join(" ")));
            }
            if changes.is_empty() {
                return;
            }
            let e = serenity::CreateEmbed::new().title("Member updated").color(BLUE).author(who(&new.user)).description(format!("<@{}>\n{}", new.user.id, changes.join("\n")));
            send(ctx, &cfg.members, e).await;
        }
        E::GuildBanAddition { banned_user, .. } if !cfg.members.is_empty() => {
            send(ctx, &cfg.members, serenity::CreateEmbed::new().title("Member banned").color(RED).author(who(banned_user)).description(format!("<@{}> {}", banned_user.id, banned_user.name))).await;
        }
        E::GuildBanRemoval { unbanned_user, .. } if !cfg.members.is_empty() => {
            send(ctx, &cfg.members, serenity::CreateEmbed::new().title("Member unbanned").color(GREEN).author(who(unbanned_user)).description(format!("<@{}> {}", unbanned_user.id, unbanned_user.name))).await;
        }
        E::ChannelCreate { channel } if !cfg.server.is_empty() => {
            send(ctx, &cfg.server, serenity::CreateEmbed::new().title("Channel created").color(GREEN).description(format!("<#{}> {}", channel.id, channel.name))).await;
        }
        E::ChannelDelete { channel, .. } if !cfg.server.is_empty() => {
            send(ctx, &cfg.server, serenity::CreateEmbed::new().title("Channel deleted").color(RED).description(format!("#{}", channel.name))).await;
        }
        E::GuildRoleCreate { new } if !cfg.server.is_empty() => {
            send(ctx, &cfg.server, serenity::CreateEmbed::new().title("Role created").color(GREEN).description(format!("<@&{}> {}", new.id, new.name))).await;
        }
        E::GuildRoleDelete { removed_role_data_if_available, removed_role_id, .. } if !cfg.server.is_empty() => {
            let name = removed_role_data_if_available.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| removed_role_id.to_string());
            send(ctx, &cfg.server, serenity::CreateEmbed::new().title("Role deleted").color(RED).description(name)).await;
        }
        E::VoiceStateUpdate { old, new } if !cfg.voice.is_empty() => {
            let before = old.as_ref().and_then(|o| o.channel_id);
            let after = new.channel_id;
            if before == after {
                return;
            }
            let text = match (before, after) {
                (None, Some(a)) => format!("<@{}> joined <#{a}>", new.user_id),
                (Some(b), None) => format!("<@{}> left <#{b}>", new.user_id),
                (Some(b), Some(a)) => format!("<@{}> moved from <#{b}> to <#{a}>", new.user_id),
                _ => return,
            };
            let color = if after.is_some() { GREEN } else { RED };
            send(ctx, &cfg.voice, serenity::CreateEmbed::new().title("Voice").color(color).description(text)).await;
        }
        _ => {}
    }
}
