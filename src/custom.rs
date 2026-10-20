//! Custom slash commands, auto replies and embeds made in the dashboard.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

static RESERVED: OnceLock<Vec<String>> = OnceLock::new();

/// Remembers the built-in command names so custom ones can't shadow them.
pub fn reserve<U, E>(cmds: Vec<poise::Command<U, E>>) -> Vec<poise::Command<U, E>> {
    let _ = RESERVED.set(cmds.iter().map(|c| c.name.to_string()).collect());
    cmds
}

fn reserved(name: &str) -> bool {
    RESERVED.get().is_some_and(|r| r.iter().any(|n| n == name))
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Command {
    pub id: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    pub role_id: String,
    pub text: String,
    pub embed: bool,
    pub title: String,
    pub color: String,
    pub image: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Responder {
    pub id: String,
    pub trigger: String,
    /// "exact", "contains" or "starts"
    pub mode: String,
    pub reply: bool,
    pub text: String,
    pub embed: bool,
    pub title: String,
    pub color: String,
    pub image: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct CustomConfig {
    pub commands: Vec<Command>,
    pub responders: Vec<Responder>,
}

fn new_id(i: usize) -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    format!("{:x}", (n >> 12) ^ (i as u64 * 7919))
}

fn check_body(text: &mut String, embed: bool, title: &mut String, color: &mut String, image: &mut String, what: &str) -> Result<(), String> {
    *text = text.trim().chars().take(2000).collect();
    *title = title.trim().chars().take(256).collect();
    *image = image.trim().to_string();
    if !image.is_empty() && !image.starts_with("https://") && !image.starts_with("http://") {
        return Err(format!("The image for {what} has to be a link"));
    }
    if parse_color(color).is_none() {
        *color = "#66BB6A".into();
    }
    if text.is_empty() && !(embed && (!title.is_empty() || !image.is_empty())) {
        return Err(format!("{what} needs a response"));
    }
    Ok(())
}

impl CustomConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.commands.len() > 50 || self.responders.len() > 50 {
            return Err("50 commands and 50 auto replies max".into());
        }
        let mut names = std::collections::HashSet::new();
        for (i, c) in self.commands.iter_mut().enumerate() {
            c.name = c.name.trim().trim_start_matches('/').to_lowercase().replace(' ', "-");
            if c.name.is_empty() || c.name.len() > 32 || !c.name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
                return Err(format!("\"/{}\" isn't a valid name. Use 1 to 32 letters, numbers, - or _", c.name));
            }
            if reserved(&c.name) {
                return Err(format!("/{} is already one of Kev's commands", c.name));
            }
            if !names.insert(c.name.clone()) {
                return Err(format!("/{} is there twice", c.name));
            }
            c.description = c.description.trim().chars().take(100).collect();
            if c.description.is_empty() {
                c.description = "Custom command".into();
            }
            if c.role_id.parse::<u64>().is_err() {
                c.role_id.clear();
            }
            let what = format!("/{}", c.name);
            check_body(&mut c.text, c.embed, &mut c.title, &mut c.color, &mut c.image, &what)?;
            if c.id.is_empty() {
                c.id = new_id(i);
            }
        }
        for (i, r) in self.responders.iter_mut().enumerate() {
            r.trigger = r.trigger.trim().to_lowercase().chars().take(100).collect();
            if r.trigger.is_empty() {
                return Err("Every auto reply needs a trigger".into());
            }
            if !matches!(r.mode.as_str(), "exact" | "contains" | "starts") {
                r.mode = "contains".into();
            }
            let what = format!("\"{}\"", r.trigger);
            check_body(&mut r.text, r.embed, &mut r.title, &mut r.color, &mut r.image, &what)?;
            if r.id.is_empty() {
                r.id = new_id(i + 1000);
            }
        }
        Ok(())
    }
}

pub fn parse_color(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim().trim_start_matches('#'), 16).ok().filter(|c| *c <= 0xFFFFFF)
}

struct Vars<'a> {
    user: &'a serenity::User,
    name: String,
    server: String,
    channel: serenity::ChannelId,
}

fn fill(s: &str, v: &Vars) -> String {
    s.replace("{mention}", &format!("<@{}>", v.user.id)).replace("{user}", &v.name).replace("{username}", &v.user.name).replace("{server}", &v.server).replace("{channel}", &format!("<#{}>", v.channel))
}

fn render(text: &str, embed: bool, title: &str, color: &str, image: &str, v: &Vars) -> (String, Option<serenity::CreateEmbed>) {
    let text = fill(text, v);
    if !embed {
        return (text, None);
    }
    let mut e = serenity::CreateEmbed::new().color(parse_color(color).unwrap_or(0x66BB6A));
    if !text.is_empty() {
        e = e.description(text);
    }
    if !title.is_empty() {
        e = e.title(fill(title, v));
    }
    if !image.is_empty() {
        e = e.image(image);
    }
    (String::new(), Some(e))
}

static CACHE: LazyLock<Mutex<HashMap<u64, (Instant, Arc<CustomConfig>)>>> = LazyLock::new(Default::default);
static COOLDOWN: LazyLock<Mutex<HashMap<(u64, String), Instant>>> = LazyLock::new(Default::default);

pub fn forget(guild: u64) {
    CACHE.lock().unwrap().remove(&guild);
}

pub async fn config(data: &Data, guild: u64) -> Arc<CustomConfig> {
    if let Some((at, c)) = CACHE.lock().unwrap().get(&guild) {
        if at.elapsed() < Duration::from_secs(120) {
            return c.clone();
        }
    }
    let raw = data.db.module_config(guild, "commands").await.ok().flatten();
    let c = Arc::new(raw.and_then(|r| serde_json::from_str::<CustomConfig>(&r).ok()).unwrap_or_default());
    CACHE.lock().unwrap().insert(guild, (Instant::now(), c.clone()));
    c
}

/// Replaces this server's custom slash commands with the saved ones.
pub async fn sync(http: &serenity::Http, guild: u64, cfg: &CustomConfig) -> Result<(), String> {
    let cmds: Vec<serenity::CreateCommand> = cfg.commands.iter().map(|c| serenity::CreateCommand::new(&c.name).description(&c.description).dm_permission(false)).collect();
    serenity::GuildId::new(guild).set_commands(http, cmds).await.map(|_| ()).map_err(|e| {
        let raw = e.to_string();
        if raw.contains("Missing Access") || raw.contains("50001") {
            "Kev can't add commands in this server. Re-invite it with the applications.commands scope.".to_string()
        } else {
            format!("Discord refused the commands: {raw}")
        }
    })
}

pub async fn on_command(ctx: &serenity::Context, data: &Data, interaction: &serenity::Interaction) {
    let serenity::Interaction::Command(c) = interaction else { return };
    let Some(guild) = c.guild_id else { return };
    if reserved(&c.data.name) {
        return;
    }
    let cfg = config(data, guild.get()).await;
    let Some(cmd) = cfg.commands.iter().find(|x| x.name == c.data.name) else { return };
    let msg = |m: serenity::CreateInteractionResponseMessage| serenity::CreateInteractionResponse::Message(m);
    if let Ok(role) = cmd.role_id.parse::<u64>() {
        if !c.member.as_ref().is_some_and(|m| m.roles.contains(&serenity::RoleId::new(role))) {
            let _ = c.create_response(&ctx.http, msg(serenity::CreateInteractionResponseMessage::new().content(format!("You need <@&{role}> for this.")).ephemeral(true))).await;
            return;
        }
    }
    let v = Vars {
        user: &c.user,
        name: c.member.as_ref().map(|m| m.display_name().to_string()).unwrap_or_else(|| c.user.display_name().to_string()),
        server: ctx.cache.guild(guild).map(|g| g.name.clone()).unwrap_or_default(),
        channel: c.channel_id,
    };
    let (text, embed) = render(&cmd.text, cmd.embed, &cmd.title, &cmd.color, &cmd.image, &v);
    let mut m = serenity::CreateInteractionResponseMessage::new().ephemeral(cmd.private).allowed_mentions(serenity::CreateAllowedMentions::new().all_users(true));
    if !text.is_empty() {
        m = m.content(text);
    }
    if let Some(e) = embed {
        m = m.embed(e);
    }
    let _ = c.create_response(&ctx.http, msg(m)).await;
}

pub async fn on_message(ctx: &serenity::Context, msg: &serenity::Message, data: &Data) {
    if msg.author.bot || msg.content.is_empty() {
        return;
    }
    let Some(guild) = msg.guild_id else { return };
    let cfg = config(data, guild.get()).await;
    if cfg.responders.is_empty() {
        return;
    }
    let lower = msg.content.to_lowercase();
    let Some(r) = cfg.responders.iter().find(|r| match r.mode.as_str() {
        "exact" => lower.trim() == r.trigger,
        "starts" => lower.starts_with(&r.trigger),
        _ => lower.contains(&r.trigger),
    }) else {
        return;
    };
    {
        let mut cd = COOLDOWN.lock().unwrap();
        let key = (msg.channel_id.get(), r.id.clone());
        if cd.get(&key).is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            return;
        }
        if cd.len() > 5000 {
            cd.clear();
        }
        cd.insert(key, Instant::now());
    }
    let v = Vars {
        user: &msg.author,
        name: msg.member.as_ref().and_then(|m| m.nick.clone()).unwrap_or_else(|| msg.author.display_name().to_string()),
        server: ctx.cache.guild(guild).map(|g| g.name.clone()).unwrap_or_default(),
        channel: msg.channel_id,
    };
    let (text, embed) = render(&r.text, r.embed, &r.title, &r.color, &r.image, &v);
    let mut m = serenity::CreateMessage::new().allowed_mentions(serenity::CreateAllowedMentions::new().all_users(true));
    if !text.is_empty() {
        m = m.content(text);
    }
    if let Some(e) = embed {
        m = m.embed(e);
    }
    if r.reply {
        m = m.reference_message(msg);
    }
    let _ = msg.channel_id.send_message(&ctx.http, m).await;
}
