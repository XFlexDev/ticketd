//! Welcome and leave messages, auto roles on join.

use anyhow::Result;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct WelcomeConfig {
    pub welcome_on: bool,
    pub welcome_channel_id: String,
    pub welcome_message: String,
    pub embed: bool,
    pub title: String,
    pub color: String,
    pub image: String,
    pub dm_on: bool,
    pub dm_message: String,
    pub leave_on: bool,
    pub leave_channel_id: String,
    pub leave_message: String,
    pub autoroles: Vec<String>,
    pub bot_roles: Vec<String>,
}

impl Default for WelcomeConfig {
    fn default() -> Self {
        Self {
            welcome_on: false,
            welcome_channel_id: String::new(),
            welcome_message: "Welcome {mention} to **{server}**! You're member #{count}.".into(),
            embed: true,
            title: "Welcome".into(),
            color: "#66BB6A".into(),
            image: String::new(),
            dm_on: false,
            dm_message: "Hey {user}, welcome to {server}!".into(),
            leave_on: false,
            leave_channel_id: String::new(),
            leave_message: "**{user}** left the server.".into(),
            autoroles: vec![],
            bot_roles: vec![],
        }
    }
}

impl WelcomeConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        let id = |s: &String| s.parse::<u64>().is_ok();
        for c in [&mut self.welcome_channel_id, &mut self.leave_channel_id] {
            if !c.is_empty() && !id(c) {
                c.clear();
            }
        }
        self.autoroles.retain(id);
        self.bot_roles.retain(id);
        self.autoroles.truncate(10);
        self.bot_roles.truncate(10);
        for (s, n) in [(&mut self.welcome_message, 2000), (&mut self.dm_message, 2000), (&mut self.leave_message, 2000), (&mut self.title, 256), (&mut self.image, 500), (&mut self.color, 7)] {
            *s = s.trim().chars().take(n).collect();
        }
        if !self.image.is_empty() && !self.image.starts_with("https://") {
            return Err("The image has to be an https link".into());
        }
        Ok(())
    }
}

pub async fn config(data: &Data, guild: u64) -> WelcomeConfig {
    data.db.module_config(guild, "welcome").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default()
}

fn fill(t: &str, user: &serenity::User, server: &str, count: u64) -> String {
    t.replace("{mention}", &format!("<@{}>", user.id))
        .replace("{user}", user.global_name.as_deref().unwrap_or(&user.name))
        .replace("{username}", &user.name)
        .replace("{server}", server)
        .replace("{count}", &count.to_string())
}

fn message(cfg: &WelcomeConfig, text: String, user: &serenity::User, title: bool) -> serenity::CreateMessage {
    let m = serenity::CreateMessage::new().allowed_mentions(serenity::CreateAllowedMentions::new().users(vec![user.id]));
    if !cfg.embed {
        return m.content(text);
    }
    let color = u32::from_str_radix(cfg.color.trim_start_matches('#'), 16).unwrap_or(0x66BB6A);
    let mut e = serenity::CreateEmbed::new().description(text).color(color).thumbnail(user.face());
    if title && !cfg.title.is_empty() {
        e = e.title(&cfg.title);
    }
    if title && !cfg.image.is_empty() {
        e = e.image(&cfg.image);
    }
    let m = if title { m.content(format!("<@{}>", user.id)) } else { m };
    m.embed(e)
}

pub async fn on_join(ctx: &serenity::Context, data: &Data, member: &serenity::Member) -> Result<()> {
    let guild = member.guild_id;
    let cfg = config(data, guild.get()).await;
    let roles: Vec<serenity::RoleId> = (if member.user.bot { &cfg.bot_roles } else { &cfg.autoroles }).iter().filter_map(|r| r.parse().ok()).map(serenity::RoleId::new).collect();
    for r in roles {
        if let Err(e) = ctx.http.add_member_role(guild, member.user.id, r, Some("Kev auto role")).await {
            tracing::warn!("autorole {r} in {guild}: {e}");
        }
    }
    if member.user.bot {
        return Ok(());
    }
    let (name, count) = ctx.cache.guild(guild).map(|g| (g.name.clone(), g.member_count)).unwrap_or_default();
    if cfg.welcome_on {
        if let Ok(ch) = cfg.welcome_channel_id.parse::<u64>() {
            let text = fill(&cfg.welcome_message, &member.user, &name, count);
            let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, message(&cfg, text, &member.user, true)).await;
        }
    }
    if cfg.dm_on && !cfg.dm_message.is_empty() {
        let text = fill(&cfg.dm_message, &member.user, &name, count);
        let _ = member.user.direct_message(&ctx.http, serenity::CreateMessage::new().content(text)).await;
    }
    Ok(())
}

pub async fn on_leave(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, user: &serenity::User) -> Result<()> {
    if user.bot {
        return Ok(());
    }
    let cfg = config(data, guild.get()).await;
    if !cfg.leave_on {
        return Ok(());
    }
    let Ok(ch) = cfg.leave_channel_id.parse::<u64>() else { return Ok(()) };
    let (name, count) = ctx.cache.guild(guild).map(|g| (g.name.clone(), g.member_count)).unwrap_or_default();
    let text = fill(&cfg.leave_message, user, &name, count);
    let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, message(&cfg, text, user, false)).await;
    Ok(())
}
