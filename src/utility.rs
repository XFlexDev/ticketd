//! Polls and reminders.

use poise::serenity_prelude as serenity;

use crate::moderation::{fmt_duration, parse_duration};
use crate::{Context, Data, Error};

/// Post a poll
#[poise::command(slash_command, guild_only, default_member_permissions = "MANAGE_MESSAGES")]
pub async fn poll(
    ctx: Context<'_>,
    #[description = "The question"] question: String,
    #[description = "Options, split with |  like  Pizza | Burgers | Tacos"] options: String,
    #[description = "How long, like 1h or 3d (max 32d)"] duration: Option<String>,
    #[description = "Let people pick more than one"] multiple: Option<bool>,
) -> Result<(), Error> {
    let answers: Vec<String> = options.split('|').map(|s| s.trim().chars().take(55).collect::<String>()).filter(|s| !s.is_empty()).take(10).collect();
    if answers.len() < 2 {
        ctx.send(poise::CreateReply::default().content("Give at least 2 options split with `|`, like `Pizza | Burgers`.").ephemeral(true)).await?;
        return Ok(());
    }
    let secs = duration.as_deref().and_then(parse_duration).unwrap_or(86400).clamp(3600, 32 * 86400);
    let mut p = serenity::CreatePoll::new()
        .question(question.chars().take(300).collect::<String>())
        .answers(answers.into_iter().map(|a| serenity::CreatePollAnswer::new().text(a)).collect())
        .duration(std::time::Duration::from_secs(secs as u64));
    if multiple.unwrap_or(false) {
        p = p.allow_multiselect();
    }
    match ctx.channel_id().send_message(ctx.http(), serenity::CreateMessage::new().poll(p)).await {
        Ok(_) => ctx.send(poise::CreateReply::default().content(format!("Poll posted, it runs for {}.", fmt_duration(secs))).ephemeral(true)).await?,
        Err(_) => ctx.send(poise::CreateReply::default().content("I can't post a poll here. I need Send Messages and Send Polls.").ephemeral(true)).await?,
    };
    Ok(())
}

/// Remind you about something later
#[poise::command(slash_command)]
pub async fn remind(
    ctx: Context<'_>,
    #[description = "When, like 10m, 2h or 3d"] when: String,
    #[description = "What to remind you about"] what: String,
) -> Result<(), Error> {
    let Some(secs) = parse_duration(&when).filter(|s| (60..=365 * 86400).contains(s)) else {
        ctx.send(poise::CreateReply::default().content("Pick a time between 1m and 1 year, like `10m`, `2h` or `3d`.").ephemeral(true)).await?;
        return Ok(());
    };
    let data = ctx.data();
    if data.db.reminders(ctx.author().id.get()).await?.len() >= 25 {
        ctx.send(poise::CreateReply::default().content("You have 25 reminders already. Cancel some with `/reminders`.").ephemeral(true)).await?;
        return Ok(());
    }
    let due = crate::db::now() + secs;
    let text: String = what.chars().take(1000).collect();
    data.db.add_reminder(ctx.author().id.get(), ctx.guild_id().map(|g| g.get()).unwrap_or(0), ctx.channel_id().get(), &text, due).await?;
    ctx.send(poise::CreateReply::default().content(format!("I'll remind you <t:{due}:R>.")).ephemeral(true)).await?;
    Ok(())
}

/// Your reminders
#[poise::command(slash_command)]
pub async fn reminders(ctx: Context<'_>, #[description = "Number of a reminder to cancel"] cancel: Option<i64>) -> Result<(), Error> {
    let data = ctx.data();
    let list = data.db.reminders(ctx.author().id.get()).await?;
    if let Some(n) = cancel {
        let Some(r) = list.get((n - 1).max(0) as usize).filter(|_| n >= 1) else {
            ctx.send(poise::CreateReply::default().content("No reminder with that number.").ephemeral(true)).await?;
            return Ok(());
        };
        data.db.delete_reminder(r.0).await?;
        ctx.send(poise::CreateReply::default().content("Cancelled.").ephemeral(true)).await?;
        return Ok(());
    }
    let text = if list.is_empty() {
        "No reminders. Make one with `/remind`.".to_string()
    } else {
        list.iter().enumerate().map(|(i, r)| format!("`{}` <t:{}:R> {}", i + 1, r.4, r.3.chars().take(80).collect::<String>())).collect::<Vec<_>>().join("\n")
    };
    ctx.send(poise::CreateReply::default().content(text).ephemeral(true)).await?;
    Ok(())
}

pub async fn sweeper(ctx: serenity::Context, data: Data) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        for (id, user, channel, text, _) in data.db.due_reminders().await.unwrap_or_default() {
            let _ = data.db.delete_reminder(id).await;
            let content = format!("<@{user}> reminder: {text}");
            let mentions = serenity::CreateAllowedMentions::new().users(vec![serenity::UserId::new(user)]);
            let sent = serenity::ChannelId::new(channel).send_message(&ctx.http, serenity::CreateMessage::new().content(&content).allowed_mentions(mentions)).await;
            if sent.is_err() {
                if let Ok(u) = serenity::UserId::new(user).to_user(&ctx).await {
                    let _ = u.direct_message(&ctx.http, serenity::CreateMessage::new().content(format!("Reminder: {text}"))).await;
                }
            }
        }
    }
}
