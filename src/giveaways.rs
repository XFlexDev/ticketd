//! Giveaways: a message with an Enter button, random winners when it ends.

use poise::serenity_prelude as serenity;

use crate::moderation::{fmt_duration, parse_duration};
use crate::{Context, Data, Error};

const GREEN: u32 = 0x66BB6A;

fn rng() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static S: AtomicU64 = AtomicU64::new(0);
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut x = S.fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed) ^ seed;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x
}

pub fn pick(mut pool: Vec<u64>, n: usize) -> Vec<u64> {
    let mut out = vec![];
    while out.len() < n && !pool.is_empty() {
        let i = (rng() % pool.len() as u64) as usize;
        out.push(pool.swap_remove(i));
    }
    out
}

fn embed(prize: &str, ends_at: i64, winners: i64, host: u64, role: u64, entries: i64, result: Option<&[u64]>) -> serenity::CreateEmbed {
    let mut lines = vec![];
    match result {
        None => lines.push(format!("Ends <t:{ends_at}:R>")),
        Some([]) => lines.push("Ended with no entries.".into()),
        Some(w) => lines.push(format!("Winner{}: {}", if w.len() == 1 { "" } else { "s" }, w.iter().map(|u| format!("<@{u}>")).collect::<Vec<_>>().join(" "))),
    }
    lines.push(format!("{winners} winner{} · hosted by <@{host}>", if winners == 1 { "" } else { "s" }));
    if role != 0 {
        lines.push(format!("Only for <@&{role}>"));
    }
    lines.push(format!("{entries} entr{}", if entries == 1 { "y" } else { "ies" }));
    serenity::CreateEmbed::new().title(prize).description(lines.join("\n")).color(if result.is_some() { 0x2B2D31 } else { GREEN })
}

fn button(ended: bool) -> Vec<serenity::CreateActionRow> {
    vec![serenity::CreateActionRow::Buttons(vec![serenity::CreateButton::new("giveaway:enter").label(if ended { "Ended" } else { "Enter" }).style(serenity::ButtonStyle::Success).disabled(ended)])]
}

fn message_id(s: &str) -> Option<u64> {
    s.trim().rsplit('/').next()?.parse().ok()
}

#[poise::command(slash_command, guild_only, subcommands("start", "end", "reroll"), default_member_permissions = "MANAGE_GUILD", required_permissions = "MANAGE_GUILD")]
pub async fn giveaway(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Start a giveaway
#[poise::command(slash_command, guild_only)]
pub async fn start(
    ctx: Context<'_>,
    #[description = "What they win"] prize: String,
    #[description = "How long, like 1h, 2d or 1w"] duration: String,
    #[description = "How many winners"]
    #[min = 1]
    #[max = 20]
    winners: Option<u8>,
    #[description = "Only members with this role can enter"] role: Option<serenity::Role>,
    #[description = "Where to post it"]
    #[channel_types("Text", "News")]
    channel: Option<serenity::GuildChannel>,
) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let Some(secs) = parse_duration(&duration).filter(|s| (60..=60 * 86400).contains(s)) else {
        ctx.send(poise::CreateReply::default().content("Pick a time between 1m and 60d, like `1h`, `2d` or `1w`.").ephemeral(true)).await?;
        return Ok(());
    };
    let winners = winners.unwrap_or(1) as i64;
    let role = role.map(|r| r.id.get()).unwrap_or(0);
    let prize: String = prize.chars().take(200).collect();
    let ends_at = crate::db::now() + secs;
    let ch = channel.map(|c| c.id).unwrap_or(ctx.channel_id());
    let host = ctx.author().id.get();
    let msg = match ch.send_message(ctx.http(), serenity::CreateMessage::new().embed(embed(&prize, ends_at, winners, host, role, 0, None)).components(button(false))).await {
        Ok(m) => m,
        Err(_) => {
            ctx.send(poise::CreateReply::default().content(format!("I can't post in <#{ch}>. Give me View Channel, Send Messages and Embed Links there.")).ephemeral(true)).await?;
            return Ok(());
        }
    };
    ctx.data().db.add_giveaway(msg.id.get(), guild.get(), ch.get(), &prize, winners, ends_at, host, role).await?;
    ctx.send(poise::CreateReply::default().content(format!("Giveaway started, it ends in {}. {}", fmt_duration(secs), msg.link())).ephemeral(true)).await?;
    Ok(())
}

/// End a giveaway now
#[poise::command(slash_command, guild_only)]
pub async fn end(ctx: Context<'_>, #[description = "Message link or ID of the giveaway"] message: String) -> Result<(), Error> {
    let Some(id) = message_id(&message) else {
        ctx.send(poise::CreateReply::default().content("Paste the giveaway's message link.").ephemeral(true)).await?;
        return Ok(());
    };
    let found = ctx.data().db.giveaway(id).await?.filter(|g| g["guild_id"].as_u64() == ctx.guild_id().map(|g| g.get()));
    let Some(g) = found else {
        ctx.send(poise::CreateReply::default().content("No giveaway with that message here.").ephemeral(true)).await?;
        return Ok(());
    };
    if g["ended"].as_bool() == Some(true) {
        ctx.send(poise::CreateReply::default().content("That giveaway already ended. Use `/giveaway reroll` to pick again.").ephemeral(true)).await?;
        return Ok(());
    }
    finish(ctx.serenity_context(), ctx.data(), &g).await;
    ctx.send(poise::CreateReply::default().content("Ended.").ephemeral(true)).await?;
    Ok(())
}

/// Pick new winners for an ended giveaway
#[poise::command(slash_command, guild_only)]
pub async fn reroll(
    ctx: Context<'_>,
    #[description = "Message link or ID of the giveaway"] message: String,
    #[description = "How many new winners"]
    #[min = 1]
    #[max = 20]
    winners: Option<u8>,
) -> Result<(), Error> {
    let Some(id) = message_id(&message) else {
        ctx.send(poise::CreateReply::default().content("Paste the giveaway's message link.").ephemeral(true)).await?;
        return Ok(());
    };
    let data = ctx.data();
    let found = data.db.giveaway(id).await?.filter(|g| g["guild_id"].as_u64() == ctx.guild_id().map(|g| g.get()));
    let Some(g) = found.filter(|g| g["ended"].as_bool() == Some(true)) else {
        ctx.send(poise::CreateReply::default().content("No ended giveaway with that message here.").ephemeral(true)).await?;
        return Ok(());
    };
    let old: Vec<u64> = g["winners_str"].as_str().unwrap_or("").split(',').filter_map(|s| s.parse().ok()).collect();
    let pool: Vec<u64> = data.db.entries(id).await?.into_iter().filter(|u| !old.contains(u)).collect();
    let new = pick(pool, winners.unwrap_or(1) as usize);
    if new.is_empty() {
        ctx.send(poise::CreateReply::default().content("Nobody else entered.").ephemeral(true)).await?;
        return Ok(());
    }
    let mentions = new.iter().map(|u| format!("<@{u}>")).collect::<Vec<_>>().join(" ");
    let ch = serenity::ChannelId::new(g["channel_id"].as_u64().unwrap_or(0));
    let _ = ch
        .send_message(ctx.http(), serenity::CreateMessage::new().content(format!("New winner{}: {mentions}, you won **{}**!", if new.len() == 1 { "" } else { "s" }, g["prize"].as_str().unwrap_or(""))).reference_message((ch, serenity::MessageId::new(id))))
        .await;
    ctx.send(poise::CreateReply::default().content("Rerolled.").ephemeral(true)).await?;
    Ok(())
}

pub async fn on_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<(), Error> {
    let reply = |t: &str| serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(t).ephemeral(true));
    let Some(g) = data.db.giveaway(c.message.id.get()).await? else {
        c.create_response(&ctx.http, reply("This giveaway doesn't exist anymore.")).await?;
        return Ok(());
    };
    if g["ended"].as_bool() == Some(true) || g["ends_at"].as_i64().unwrap_or(0) <= crate::db::now() {
        c.create_response(&ctx.http, reply("This giveaway has ended.")).await?;
        return Ok(());
    }
    let role = g["role_id"].as_u64().unwrap_or(0);
    if role != 0 && !c.member.as_ref().is_some_and(|m| m.roles.contains(&serenity::RoleId::new(role))) {
        c.create_response(&ctx.http, reply(&format!("Only members with <@&{role}> can enter."))).await?;
        return Ok(());
    }
    let joined = data.db.toggle_entry(c.message.id.get(), c.user.id.get()).await?;
    c.create_response(&ctx.http, reply(if joined { "You're in. Press Enter again to leave." } else { "You left the giveaway." })).await?;
    let count = data.db.entry_count(c.message.id.get()).await.unwrap_or(0);
    let e = embed(g["prize"].as_str().unwrap_or(""), g["ends_at"].as_i64().unwrap_or(0), g["winners"].as_i64().unwrap_or(1), g["host_id"].as_u64().unwrap_or(0), role, count, None);
    let _ = c.channel_id.edit_message(&ctx.http, c.message.id, serenity::EditMessage::new().embed(e)).await;
    Ok(())
}

async fn finish(ctx: &serenity::Context, data: &Data, g: &serde_json::Value) {
    let id = g["message_id"].as_u64().unwrap_or(0);
    let entries = data.db.entries(id).await.unwrap_or_default();
    let n = g["winners"].as_i64().unwrap_or(1);
    let won = pick(entries.clone(), n as usize);
    if data.db.end_giveaway(id, &won.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",")).await.is_err() {
        return;
    }
    let ch = serenity::ChannelId::new(g["channel_id"].as_u64().unwrap_or(0));
    let prize = g["prize"].as_str().unwrap_or("");
    let e = embed(prize, g["ends_at"].as_i64().unwrap_or(0), n, g["host_id"].as_u64().unwrap_or(0), g["role_id"].as_u64().unwrap_or(0), entries.len() as i64, Some(&won));
    let _ = ch.edit_message(&ctx.http, serenity::MessageId::new(id), serenity::EditMessage::new().embed(e).components(button(true))).await;
    let text = if won.is_empty() {
        format!("The giveaway for **{prize}** ended with no entries.")
    } else {
        format!("Congrats {}, you won **{prize}**!", won.iter().map(|u| format!("<@{u}>")).collect::<Vec<_>>().join(" "))
    };
    let _ = ch.send_message(&ctx.http, serenity::CreateMessage::new().content(text).reference_message((ch, serenity::MessageId::new(id)))).await;
}

pub async fn sweeper(ctx: serenity::Context, data: Data) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        for g in data.db.due_giveaways().await.unwrap_or_default() {
            finish(&ctx, &data, &g).await;
        }
    }
}
