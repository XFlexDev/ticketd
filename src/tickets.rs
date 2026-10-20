use crate::agent::{self, Action, AgentConfig, TicketInfo};
use crate::db::{Settings, Ticket};
use crate::panels::{Panel, PanelOption};
use crate::{Context, Data, Error};
use anyhow::{Context as _, Result};
use poise::serenity_prelude as serenity;
use serenity::Mentionable;

const LEFT_REASON: &str = "Member left the server";

/// Per-guild settings from KreatixDB; errors with a setup hint when missing.
async fn settings(data: &Data, guild_id: serenity::GuildId) -> Result<Settings> {
    data.db
        .settings(guild_id.get())
        .await?
        .ok_or_else(|| anyhow::anyhow!("This server isn't set up yet. An admin must run `/setup` first."))
}

/// Staff = server owner, anyone with Administrator/Manage Server, or the optional staff role.
pub fn is_staff(cache: &serenity::Cache, guild: serenity::GuildId, user: serenity::UserId, roles: &[serenity::RoleId], s: &Settings) -> bool {
    if s.mod_role_id != 0 && roles.contains(&serenity::RoleId::new(s.mod_role_id as u64)) {
        return true;
    }
    let Some(g) = cache.guild(guild) else { return false };
    g.owner_id == user
        || roles
            .iter()
            .filter_map(|r| g.roles.get(r))
            .any(|r| r.permissions.intersects(serenity::Permissions::ADMINISTRATOR | serenity::Permissions::MANAGE_GUILD))
}

/// Server staff, or one of the support roles of the ticket type this ticket was opened with.
fn is_ticket_staff(cache: &serenity::Cache, guild: serenity::GuildId, user: serenity::UserId, roles: &[serenity::RoleId], s: &Settings, t: &Ticket) -> bool {
    is_staff(cache, guild, user, roles, s) || roles.iter().any(|r| t.staff_roles.contains(&r.get()))
}

async fn guild_owner(ctx: &serenity::Context, guild: serenity::GuildId) -> Option<serenity::UserId> {
    let cached = ctx.cache.guild(guild).map(|g| g.owner_id);
    match cached {
        Some(o) => Some(o),
        None => guild.to_partial_guild(&ctx.http).await.ok().map(|g| g.owner_id),
    }
}

/// Returns the configured ticket category. If it's missing or not actually a category,
/// reuses a "Tickets" category or creates one, and saves it.
async fn ensure_category(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, s: &Settings) -> Result<serenity::ChannelId> {
    let channels = guild.channels(&ctx.http).await?;
    let is_cat = |id: serenity::ChannelId| channels.get(&id).is_some_and(|c| c.kind == serenity::ChannelType::Category);
    if s.ticket_category_id != 0 && is_cat(serenity::ChannelId::new(s.ticket_category_id as u64)) {
        return Ok(serenity::ChannelId::new(s.ticket_category_id as u64));
    }
    let id = match channels
        .values()
        .find(|c| c.kind == serenity::ChannelType::Category && c.name.eq_ignore_ascii_case("tickets"))
    {
        Some(c) => c.id,
        None => {
            guild
                .create_channel(&ctx.http, serenity::CreateChannel::new("Tickets").kind(serenity::ChannelType::Category))
                .await?
                .id
        }
    };
    let mut fixed = s.clone();
    fixed.ticket_category_id = id.get() as i64;
    data.db.upsert_settings(&fixed).await?;
    Ok(id)
}

/// The user's open ticket, unless its channel was deleted by hand (then it's closed in the DB).
async fn live_open_ticket(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, user: serenity::UserId) -> Result<Option<Ticket>> {
    let Some(t) = data.db.open_ticket_for_user(guild.get(), user.get()).await? else { return Ok(None) };
    match serenity::ChannelId::new(t.channel_id as u64).to_channel(&ctx.http).await {
        Err(e) if e.to_string().contains("Unknown Channel") => {
            data.db.close_ticket(t.id).await?;
            Ok(None)
        }
        _ => Ok(Some(t)),
    }
}

/// What a new ticket looks like: from a panel option, or a plain /open.
#[derive(Default)]
struct OpenSpec {
    subject: String,
    category_id: u64,
    staff_roles: Vec<u64>,
    ping_roles: Vec<u64>,
    instructions: String,
    answers: Vec<(String, String)>,
}

impl OpenSpec {
    fn from_option(o: &PanelOption, answers: Vec<(String, String)>) -> Self {
        let ids = |v: &[String]| v.iter().filter_map(|r| r.parse().ok()).collect::<Vec<u64>>();
        Self {
            subject: o.label.clone(),
            category_id: o.category_id.parse().unwrap_or(0),
            staff_roles: ids(&o.staff_roles),
            ping_roles: ids(&o.ping_roles),
            instructions: o.instructions.clone(),
            answers,
        }
    }
}

async fn create_ticket_channel(
    ctx: &serenity::Context,
    data: &Data,
    guild: serenity::GuildId,
    s: &Settings,
    user: &serenity::User,
    spec: &OpenSpec,
) -> Result<(i64, serenity::ChannelId)> {
    let category = if spec.category_id != 0 {
        let id = serenity::ChannelId::new(spec.category_id);
        match id.to_channel(&ctx.http).await {
            Ok(serenity::Channel::Guild(c)) if c.kind == serenity::ChannelType::Category && c.guild_id == guild => id,
            _ => ensure_category(ctx, data, guild, s).await?,
        }
    } else {
        ensure_category(ctx, data, guild, s).await?
    };
    let everyone_deny = serenity::PermissionOverwrite {
        allow: serenity::Permissions::empty(),
        deny: serenity::Permissions::VIEW_CHANNEL,
        kind: serenity::PermissionOverwriteType::Role(serenity::RoleId::new(guild.get())),
    };
    let staff_perms = serenity::Permissions::VIEW_CHANNEL
        | serenity::Permissions::SEND_MESSAGES
        | serenity::Permissions::READ_MESSAGE_HISTORY
        | serenity::Permissions::ATTACH_FILES
        | serenity::Permissions::MANAGE_MESSAGES;
    let mut roles: Vec<u64> = spec.staff_roles.clone();
    if s.mod_role_id != 0 {
        roles.push(s.mod_role_id as u64);
    }
    roles.sort();
    roles.dedup();
    let mut staff_allow: Vec<serenity::PermissionOverwrite> = roles
        .iter()
        .map(|r| serenity::PermissionOverwrite {
            allow: staff_perms,
            deny: serenity::Permissions::empty(),
            kind: serenity::PermissionOverwriteType::Role(serenity::RoleId::new(*r)),
        })
        .collect();
    if let Some(owner) = guild_owner(ctx, guild).await.filter(|o| *o != user.id) {
        staff_allow.push(serenity::PermissionOverwrite {
            allow: staff_perms,
            deny: serenity::Permissions::empty(),
            kind: serenity::PermissionOverwriteType::Member(owner),
        });
    }
    let user_allow = serenity::PermissionOverwrite {
        allow: serenity::Permissions::VIEW_CHANNEL
            | serenity::Permissions::SEND_MESSAGES
            | serenity::Permissions::READ_MESSAGE_HISTORY
            | serenity::Permissions::ATTACH_FILES,
        deny: serenity::Permissions::empty(),
        kind: serenity::PermissionOverwriteType::Member(user.id),
    };
    let bot_allow = serenity::PermissionOverwrite {
        allow: serenity::Permissions::VIEW_CHANNEL
            | serenity::Permissions::SEND_MESSAGES
            | serenity::Permissions::EMBED_LINKS
            | serenity::Permissions::READ_MESSAGE_HISTORY
            | serenity::Permissions::MANAGE_CHANNELS,
        deny: serenity::Permissions::empty(),
        kind: serenity::PermissionOverwriteType::Member(ctx.cache.current_user().id),
    };

    let n = data.db.count_tickets(guild.get()).await?;
    let name = format!("{}-{:04}-{}", data.cfg.ticket_prefix, n + 1, user.name.to_lowercase().replace(' ', "-"));

    let channel = guild
        .create_channel(
            &ctx.http,
            serenity::CreateChannel::new(name)
                .kind(serenity::ChannelType::Text)
                .category(category)
                .permissions([vec![everyone_deny, user_allow, bot_allow], staff_allow].concat())
                .topic(format!("Ticket by {} ({})", user.name, spec.subject)),
        )
        .await?;

    let id = data
        .db
        .create_ticket(guild.get(), channel.id.get(), user.id.get(), &spec.subject, &spec.staff_roles, &spec.ping_roles, &spec.instructions)
        .await?;
    Ok((id, channel.id))
}

/// Creates the ticket (or points to the open one). Returns the private reply for the member.
async fn open_ticket(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, user: &serenity::User, spec: OpenSpec) -> Result<String> {
    let s = settings(data, guild).await?;
    if let Some(t) = live_open_ticket(ctx, data, guild, user.id).await? {
        return Ok(format!("You already have an open ticket: <#{}>", t.channel_id));
    }
    let (id, ch) = create_ticket_channel(ctx, data, guild, &s, user, &spec).await?;
    greet_ticket(ctx, data, &s, ch, user, &spec, id, guild).await.ok();
    Ok(format!("Ticket opened: <#{ch}>"))
}

fn friendly_open_error(e: &anyhow::Error) -> String {
    let raw = e.to_string();
    if raw.contains("Invalid permissions") || raw.contains("Missing Permissions") || raw.contains("Missing Access") {
        "I can't create the ticket channel. Give my role **Manage Channels** and **Manage Roles** in the ticket category, or re-invite me from kev.kreatix.dev.".into()
    } else {
        format!("Couldn't open a ticket: {raw}")
    }
}

/// Everything the AI may look up about this ticket, gathered without holding cache refs across awaits.
async fn ticket_info(ctx: &serenity::Context, data: &Data, guild: serenity::GuildId, user: &serenity::User, t: &Ticket) -> TicketInfo {
    let (guild_name, server, role_names) = match ctx.cache.guild(guild) {
        Some(g) => {
            let mut roles: Vec<&serenity::Role> = g.roles.values().filter(|r| r.id.get() != guild.get() && !r.managed).collect();
            roles.sort_by_key(|r| std::cmp::Reverse(r.position));
            let everyone = serenity::RoleId::new(guild.get());
            let mut chans: Vec<&serenity::GuildChannel> = g
                .channels
                .values()
                .filter(|c| matches!(c.kind, serenity::ChannelType::Text | serenity::ChannelType::News | serenity::ChannelType::Forum))
                .filter(|c| {
                    !c.permission_overwrites.iter().any(|o| {
                        o.kind == serenity::PermissionOverwriteType::Role(everyone) && o.deny.contains(serenity::Permissions::VIEW_CHANNEL)
                    })
                })
                .collect();
            chans.sort_by_key(|c| c.position);
            let server = format!(
                "Server: {}\nMembers: {}\nRoles: {}\nPublic channels: {}",
                g.name,
                g.member_count,
                roles.iter().take(40).map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
                chans.iter().take(50).map(|c| format!("#{}", c.name)).collect::<Vec<_>>().join(", ")
            );
            let names: std::collections::HashMap<u64, String> = g.roles.iter().map(|(id, r)| (id.get(), r.name.clone())).collect();
            (g.name.clone(), server, names)
        }
        None => (String::new(), String::new(), Default::default()),
    };
    let (member, past) = tokio::join!(guild.member(&ctx.http, user.id), data.db.user_tickets(guild.get(), user.id.get(), t.id));
    let (member_roles, joined_at) = match member {
        Ok(m) => (
            m.roles.iter().filter_map(|r| role_names.get(&r.get()).cloned()).collect(),
            m.joined_at.map(|j| j.to_rfc3339().unwrap_or_default()).unwrap_or_default(),
        ),
        Err(_) => (vec![], String::new()),
    };
    TicketInfo {
        guild_id: guild.get(),
        guild_name,
        channel_id: t.channel_id as u64,
        user_id: user.id.get(),
        user_name: user.name.clone(),
        subject: t.subject.clone(),
        ticket_id: t.id,
        priority: t.priority.clone(),
        opened_at: t.created_at,
        type_instructions: t.instructions.clone(),
        member_roles,
        joined_at,
        past_tickets: past.unwrap_or_default(),
        server,
    }
}

fn ticket_buttons() -> serenity::CreateActionRow {
    serenity::CreateActionRow::Buttons(vec![
        serenity::CreateButton::new("ticket:close").label("Close ticket").style(serenity::ButtonStyle::Danger),
        serenity::CreateButton::new("ticket:claim").label("Claim").style(serenity::ButtonStyle::Secondary),
    ])
}

#[allow(clippy::too_many_arguments)]
async fn greet_ticket(
    ctx: &serenity::Context,
    data: &Data,
    s: &Settings,
    channel_id: serenity::ChannelId,
    user: &serenity::User,
    spec: &OpenSpec,
    ticket_id: i64,
    guild: serenity::GuildId,
) -> Result<()> {
    let bot_id = ctx.cache.current_user().id.get();
    let ai_on = s.ai_enabled && data.cfg.ai_enabled;
    let cfg = agent_config(data, guild).await;
    let form = spec.answers.iter().map(|(q, a)| format!("**{q}**\n{a}")).collect::<Vec<_>>().join("\n\n");
    if !spec.answers.is_empty() {
        let plain = spec.answers.iter().map(|(q, a)| format!("{q}: {a}")).collect::<Vec<_>>().join("\n");
        data.db.log_message(ticket_id, &user.name, user.id.get(), &format!("Form answers:\n{plain}"), false).await?;
    }

    let fallback = format!("Hi {}! Describe your issue and the team will help.", user.name);
    let text = if !cfg.greeting.is_empty() {
        cfg.greeting.replace("{user}", &user.mention().to_string()).replace("{type}", &spec.subject)
    } else if ai_on {
        let t = data.db.by_id(ticket_id).await?.unwrap_or_default();
        let info = ticket_info(ctx, data, guild, user, &t).await;
        let prompt = if spec.answers.is_empty() {
            format!("{} opened a {:?} ticket. Greet them briefly and ask what they need help with.", user.name, spec.subject)
        } else {
            format!(
                "{} opened a {:?} ticket and answered:\n{}\n\nGreet them briefly and start helping based on their answers. Ask only for what is still missing.",
                user.name,
                spec.subject,
                spec.answers.iter().map(|(q, a)| format!("{q}: {a}")).collect::<Vec<_>>().join("\n")
            )
        };
        data.ai.chat(&agent::system_prompt(&cfg, &info), &[], &prompt).await.unwrap_or(fallback)
    } else {
        fallback
    };

    let pings: String = spec.ping_roles.iter().map(|r| format!(" <@&{r}>")).collect();
    let mut msg = serenity::CreateMessage::new()
        .content(format!("{}{pings}", user.mention()))
        .allowed_mentions(
            serenity::CreateAllowedMentions::new()
                .users(vec![user.id])
                .roles(spec.ping_roles.iter().map(|r| serenity::RoleId::new(*r)).collect::<Vec<_>>()),
        )
        .components(vec![ticket_buttons()]);
    if !form.is_empty() {
        msg = msg.embed(serenity::CreateEmbed::new().title(&spec.subject).description(form.chars().take(4000).collect::<String>()).colour(0x2B2D31));
    }
    let w = &cfg.welcome;
    let mut welcome = serenity::CreateEmbed::new().description(&text).colour(crate::panels::parse_color(&w.color).unwrap_or(0x5865F2));
    if !w.title.is_empty() {
        welcome = welcome.title(w.title.replace("{type}", &spec.subject));
    }
    msg = msg.embed(crate::panels::media(welcome, &w.image, &w.thumbnail, "", "", &w.footer, &w.footer_icon));
    channel_id.send_message(&ctx.http, msg).await?;
    data.db.log_message(ticket_id, "Kev", bot_id, &text, true).await?;
    Ok(())
}

/// /setup — pick where tickets go and who mods them (Manage Server only).
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_GUILD", default_member_permissions = "MANAGE_GUILD")]
pub async fn setup(
    ctx: Context<'_>,
    #[description = "Category for ticket channels (default: creates \"Tickets\")"]
    #[channel_types("Category")]
    category: Option<serenity::GuildChannel>,
    #[description = "Extra staff role (owner and admins always have access)"] staff_role: Option<serenity::Role>,
    #[description = "Channel for closed-ticket transcripts (optional)"]
    #[channel_types("Text")]
    log_channel: Option<serenity::GuildChannel>,
    #[description = "AI auto-replies in tickets (default on)"] ai: Option<bool>,
) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    ctx.defer_ephemeral().await?;
    let mut s = ctx.data().db.settings(guild.get()).await?.unwrap_or(Settings {
        guild_id: guild.get() as i64,
        ai_enabled: true,
        close_on_leave: true,
        ratings: true,
        ..Default::default()
    });
    if let Some(c) = category {
        s.ticket_category_id = c.id.get() as i64;
    }
    if let Some(r) = &staff_role {
        s.mod_role_id = r.id.get() as i64;
    }
    if let Some(c) = log_channel {
        s.log_channel_id = Some(c.id.get() as i64);
    }
    if let Some(a) = ai {
        s.ai_enabled = a;
    }
    s.ticket_category_id = ensure_category(ctx.serenity_context(), ctx.data(), guild, &s).await?.get() as i64;
    ctx.data().db.upsert_settings(&s).await?;
    private(
        ctx,
        format!(
            "Setup saved: tickets → <#{}>, staff → owner + admins{}, logs → {}\nPanels, AI agent and more: <{}/g/{}>",
            s.ticket_category_id,
            if s.mod_role_id != 0 { format!(" + <@&{}>", s.mod_role_id) } else { String::new() },
            s.log_channel_id.map(|c| format!("<#{c}>")).unwrap_or_else(|| "off".into()),
            dashboard_url(),
            guild
        ),
    )
    .await?;
    Ok(())
}

fn dashboard_url() -> String {
    std::env::var("DASHBOARD_BASE_URL").unwrap_or_else(|_| "https://kev.kreatix.dev".into())
}

async fn private(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(poise::CreateReply::default().content(text).ephemeral(true)).await?;
    Ok(())
}

/// Ticket-channel commands: only the opener or staff may use them. Replies privately and returns None otherwise.
async fn ticket_guard(ctx: Context<'_>, staff_only: bool) -> Result<Option<(Ticket, Settings)>, Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(None) };
    let Some(t) = ctx.data().db.by_channel(ctx.channel_id().get()).await?.filter(|t| t.status == "open") else {
        private(ctx, "This isn't an open ticket channel.").await?;
        return Ok(None);
    };
    let s = settings(ctx.data(), guild).await?;
    let roles = ctx.author_member().await.map(|m| m.roles.clone()).unwrap_or_default();
    let staff = is_ticket_staff(&ctx.serenity_context().cache, guild, ctx.author().id, &roles, &s, &t);
    if staff_only && !staff {
        private(ctx, "Staff only.").await?;
        return Ok(None);
    }
    if !staff && t.user_id as u64 != ctx.author().id.get() {
        private(ctx, "Only staff or the person who opened this ticket can do that.").await?;
        return Ok(None);
    }
    Ok(Some((t, s)))
}

/// /open — create a ticket.
#[poise::command(slash_command, guild_only, rename = "open")]
pub async fn ticket_open(ctx: Context<'_>, #[description = "What do you need help with?"] subject: Option<String>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let spec = OpenSpec { subject: subject.unwrap_or_else(|| "support".into()), ..Default::default() };
    let text = match open_ticket(ctx.serenity_context(), ctx.data(), guild, ctx.author(), spec).await {
        Ok(t) => t,
        Err(e) => friendly_open_error(&e),
    };
    private(ctx, text).await?;
    Ok(())
}

async fn autocomplete_panel(ctx: Context<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let p = partial.to_lowercase();
    let names: Vec<String> = match ctx.guild_id() {
        Some(guild) => ctx.data().db.panels(guild.get()).await.unwrap_or_default().into_iter().map(|x| x.name).collect(),
        None => vec![],
    };
    serenity::CreateAutocompleteResponse::new().set_choices(
        names
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&p))
            .take(25)
            .map(|n| serenity::AutocompleteChoice::new(n.clone(), n))
            .collect(),
    )
}

/// /panel — post a ticket panel in this channel (staff only).
#[poise::command(slash_command, guild_only, rename = "panel", required_permissions = "MANAGE_GUILD", default_member_permissions = "MANAGE_GUILD")]
pub async fn ticket_panel(
    ctx: Context<'_>,
    #[description = "Which panel (make them on kev.kreatix.dev)"]
    #[autocomplete = "autocomplete_panel"]
    name: Option<String>,
) -> Result<(), Error> {
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let s = match settings(ctx.data(), guild).await {
        Ok(s) => s,
        Err(e) => {
            private(ctx, e.to_string()).await?;
            return Ok(());
        }
    };
    let member = ctx.author_member().await.ok_or_else(|| anyhow::anyhow!("members only"))?;
    if !is_staff(&ctx.serenity_context().cache, guild, member.user.id, &member.roles, &s) {
        private(ctx, "Staff only.").await?;
        return Ok(());
    }
    let panels = ctx.data().db.panels(guild.get()).await?;
    let panel = match &name {
        Some(n) => match panels.iter().find(|p| p.name.eq_ignore_ascii_case(n.trim())) {
            Some(p) => p.clone(),
            None => {
                private(ctx, format!("No panel called \"{n}\". Make one at <{}/g/{}?tab=panels>", dashboard_url(), guild)).await?;
                return Ok(());
            }
        },
        None => panels.first().cloned().unwrap_or_default(),
    };
    let (embed, rows) = panel.render();
    ctx.send(poise::CreateReply::default().embed(embed).components(rows)).await?;
    Ok(())
}

/// /close — close the ticket you're in (saves transcript + AI summary).
#[poise::command(slash_command, guild_only, rename = "close")]
pub async fn ticket_close(ctx: Context<'_>, #[description = "Why it's being closed (optional)"] reason: Option<String>) -> Result<(), Error> {
    if ticket_guard(ctx, false).await?.is_none() {
        return Ok(());
    }
    private(ctx, "Closing ticket…").await?;
    close_current(ctx.serenity_context(), ctx.data(), ctx.channel_id(), ctx.author().id, reason.as_deref().unwrap_or("")).await?;
    Ok(())
}

/// /add — add a user to this ticket.
#[poise::command(slash_command, guild_only, rename = "add")]
pub async fn ticket_add(ctx: Context<'_>, #[description = "User to add"] user: serenity::User) -> Result<(), Error> {
    if ticket_guard(ctx, false).await?.is_none() {
        return Ok(());
    }
    ctx.channel_id()
        .create_permission(
            ctx.serenity_context(),
            serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL | serenity::Permissions::SEND_MESSAGES | serenity::Permissions::READ_MESSAGE_HISTORY,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Member(user.id),
            },
        )
        .await?;
    ctx.say(format!("Added {} to the ticket.", user.mention())).await?;
    Ok(())
}

/// /remove — remove a user from this ticket.
#[poise::command(slash_command, guild_only, rename = "remove")]
pub async fn ticket_remove(ctx: Context<'_>, #[description = "User to remove"] user: serenity::User) -> Result<(), Error> {
    if ticket_guard(ctx, false).await?.is_none() {
        return Ok(());
    }
    ctx.channel_id()
        .create_permission(
            ctx.serenity_context(),
            serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Member(user.id),
            },
        )
        .await?;
    ctx.say(format!("Removed {} from the ticket.", user.mention())).await?;
    Ok(())
}

/// /transcript — dump this ticket's transcript.
#[poise::command(slash_command, guild_only, rename = "transcript")]
pub async fn ticket_transcript(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let Some(t) = ctx.data().db.by_channel(ctx.channel_id().get()).await? else {
        private(ctx, "This isn't a ticket channel.").await?;
        return Ok(());
    };
    let staff = match settings(ctx.data(), guild).await {
        Ok(s) => {
            let roles = ctx.author_member().await.map(|m| m.roles.clone()).unwrap_or_default();
            is_ticket_staff(&ctx.serenity_context().cache, guild, ctx.author().id, &roles, &s, &t)
        }
        Err(_) => false,
    };
    let text = ctx.data().db.transcript_for(t.id, staff).await?;
    ctx.send(
        poise::CreateReply::default()
            .content(format!("Transcript for ticket #{}", t.id))
            .attachment(serenity::CreateAttachment::bytes(text.into_bytes(), format!("ticket-{}.txt", t.id))),
    )
    .await?;
    Ok(())
}

/// Claim for `by`. Ok(public notice) or Err(private reason).
async fn claim(ctx: &serenity::Context, data: &Data, t: &Ticket, by: serenity::UserId) -> Result<String, String> {
    if t.user_id as u64 == by.get() {
        return Err("You can't claim your own ticket.".into());
    }
    if t.claimed_by as u64 == by.get() {
        return Err("You already claimed this ticket.".into());
    }
    if t.claimed_by != 0 {
        return Err(format!("Already claimed by <@{}>. Ask them to /unclaim or /transfer it.", t.claimed_by));
    }
    data.db.set_claim(t.id, by.get()).await.map_err(|e| e.to_string())?;
    data.db.touch(t.id, true, true).await.ok();
    let _ = ctx;
    Ok(format!("{} claimed this ticket and will help you from here.", by.mention()))
}

/// /claim — take this ticket; the AI steps back.
#[poise::command(slash_command, guild_only, rename = "claim")]
pub async fn ticket_claim(ctx: Context<'_>) -> Result<(), Error> {
    let Some((t, _)) = ticket_guard(ctx, true).await? else { return Ok(()) };
    match claim(ctx.serenity_context(), ctx.data(), &t, ctx.author().id).await {
        Ok(notice) => {
            ctx.send(poise::CreateReply::default().content(notice).allowed_mentions(serenity::CreateAllowedMentions::new())).await?;
        }
        Err(e) => private(ctx, e).await?,
    }
    Ok(())
}

/// /unclaim — release this ticket.
#[poise::command(slash_command, guild_only, rename = "unclaim")]
pub async fn ticket_unclaim(ctx: Context<'_>) -> Result<(), Error> {
    let Some((t, s)) = ticket_guard(ctx, true).await? else { return Ok(()) };
    let Some(guild) = ctx.guild_id() else { return Ok(()) };
    let roles = ctx.author_member().await.map(|m| m.roles.clone()).unwrap_or_default();
    if t.claimed_by == 0 {
        private(ctx, "Nobody has claimed this ticket.").await?;
        return Ok(());
    }
    if t.claimed_by as u64 != ctx.author().id.get() && !is_staff(&ctx.serenity_context().cache, guild, ctx.author().id, &roles, &s) {
        private(ctx, "Only the person who claimed it or an admin can unclaim it.").await?;
        return Ok(());
    }
    ctx.data().db.set_claim(t.id, 0).await?;
    ctx.say("This ticket is unclaimed again.").await?;
    Ok(())
}

/// /transfer — hand this ticket to another staff member.
#[poise::command(slash_command, guild_only, rename = "transfer")]
pub async fn ticket_transfer(ctx: Context<'_>, #[description = "Staff member to take over"] user: serenity::User) -> Result<(), Error> {
    let Some((t, _)) = ticket_guard(ctx, true).await? else { return Ok(()) };
    if user.bot || user.id.get() == t.user_id as u64 {
        private(ctx, "Pick a staff member, not the ticket owner or a bot.").await?;
        return Ok(());
    }
    ctx.channel_id()
        .create_permission(
            ctx.serenity_context(),
            serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL | serenity::Permissions::SEND_MESSAGES | serenity::Permissions::READ_MESSAGE_HISTORY,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Member(user.id),
            },
        )
        .await?;
    ctx.data().db.set_claim(t.id, user.id.get()).await?;
    ctx.send(
        poise::CreateReply::default()
            .content(format!("{} handed this ticket to {}.", ctx.author().mention(), user.mention()))
            .allowed_mentions(serenity::CreateAllowedMentions::new().users(vec![user.id])),
    )
    .await?;
    Ok(())
}

#[derive(poise::ChoiceParameter)]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

/// /priority — set how urgent this ticket is.
#[poise::command(slash_command, guild_only, rename = "priority")]
pub async fn ticket_priority(ctx: Context<'_>, #[description = "Priority"] level: Priority) -> Result<(), Error> {
    let Some((t, _)) = ticket_guard(ctx, true).await? else { return Ok(()) };
    let p = match level {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
        Priority::Urgent => "urgent",
    };
    ctx.data().db.set_priority(t.id, p).await?;
    ctx.say(format!("Priority set to **{p}** by {}.", ctx.author().mention())).await?;
    Ok(())
}

fn modal_for(custom_id: String, title: &str, o: &PanelOption) -> serenity::CreateModal {
    let rows = o
        .questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut input = serenity::CreateInputText::new(
                if q.long { serenity::InputTextStyle::Paragraph } else { serenity::InputTextStyle::Short },
                &q.label,
                format!("q{i}"),
            )
            .required(q.required)
            .max_length(if q.long { 1500 } else { 200 });
            if !q.placeholder.is_empty() {
                input = input.placeholder(&q.placeholder);
            }
            serenity::CreateActionRow::InputText(input)
        })
        .collect();
    serenity::CreateModal::new(custom_id, title.chars().take(45).collect::<String>()).components(rows)
}

async fn defer_private(ctx: &serenity::Context, c: &serenity::ComponentInteraction) -> Result<()> {
    c.create_response(&ctx.http, serenity::CreateInteractionResponse::Defer(serenity::CreateInteractionResponseMessage::new().ephemeral(true)))
        .await?;
    Ok(())
}

async fn reply_private(ctx: &serenity::Context, c: &serenity::ComponentInteraction, text: impl Into<String>) -> Result<()> {
    c.create_response(
        &ctx.http,
        serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(text).ephemeral(true)),
    )
    .await?;
    Ok(())
}

/// Panel button / dropdown: show the form if the option has questions, else open right away.
async fn panel_pick(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction, panel_id: i64, opt_id: &str) -> Result<()> {
    let Some(guild) = c.guild_id else { return Ok(()) };
    let panel = data.db.panel(guild.get(), panel_id).await?;
    let Some(o) = panel.as_ref().and_then(|p| p.option(opt_id)).cloned() else {
        return reply_private(ctx, c, "This panel was changed or deleted. Ask staff to post it again with /panel.").await;
    };
    if !o.questions.is_empty() {
        if let Some(t) = live_open_ticket(ctx, data, guild, c.user.id).await? {
            return reply_private(ctx, c, format!("You already have an open ticket: <#{}>", t.channel_id)).await;
        }
        c.create_response(&ctx.http, serenity::CreateInteractionResponse::Modal(modal_for(format!("tf:{panel_id}:{}", o.id), &o.label, &o)))
            .await?;
        return Ok(());
    }
    defer_private(ctx, c).await?;
    let text = open_ticket(ctx, data, guild, &c.user, OpenSpec::from_option(&o, vec![])).await.unwrap_or_else(|e| friendly_open_error(&e));
    c.edit_response(&ctx.http, serenity::EditInteractionResponse::new().content(text)).await?;
    Ok(())
}

fn modal_values(m: &serenity::ModalInteraction) -> Vec<(String, String)> {
    m.data
        .components
        .iter()
        .flat_map(|r| r.components.iter())
        .filter_map(|c| match c {
            serenity::ActionRowComponent::InputText(t) => Some((t.custom_id.clone(), t.value.clone().unwrap_or_default())),
            _ => None,
        })
        .collect()
}

async fn on_modal(ctx: &serenity::Context, data: &Data, m: &serenity::ModalInteraction) -> Result<()> {
    let id = m.data.custom_id.clone();
    if let Some(rest) = id.strip_prefix("tf:") {
        let Some(guild) = m.guild_id else { return Ok(()) };
        m.create_response(&ctx.http, serenity::CreateInteractionResponse::Defer(serenity::CreateInteractionResponseMessage::new().ephemeral(true)))
            .await?;
        let (pid, oid) = rest.split_once(':').unwrap_or((rest, ""));
        let panel: Option<Panel> = data.db.panel(guild.get(), pid.parse().unwrap_or(0)).await?;
        let text = match panel.as_ref().and_then(|p| p.option(oid)) {
            None => "This panel was changed or deleted. Ask staff to post it again with /panel.".to_string(),
            Some(o) => {
                let values = modal_values(m);
                let answers = o
                    .questions
                    .iter()
                    .enumerate()
                    .filter_map(|(i, q)| {
                        let v = values.iter().find(|(k, _)| *k == format!("q{i}")).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                        (!v.is_empty()).then(|| (q.label.clone(), v))
                    })
                    .collect();
                open_ticket(ctx, data, guild, &m.user, OpenSpec::from_option(o, answers)).await.unwrap_or_else(|e| friendly_open_error(&e))
            }
        };
        m.edit_response(&ctx.http, serenity::EditInteractionResponse::new().content(text)).await?;
    } else if let Some(rest) = id.strip_prefix("ratec:") {
        let (tid, stars) = rest.split_once(':').unwrap_or((rest, "0"));
        let comment = modal_values(m).into_iter().next().map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        let tid: i64 = tid.parse().unwrap_or(0);
        if !comment.is_empty() {
            data.db.rate_comment(tid, &comment).await?;
        }
        m.create_response(
            &ctx.http,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(rated_embed(stars.parse().unwrap_or(0), &comment))
                    .components(vec![]),
            ),
        )
        .await?;
        if let Some(t) = data.db.by_id(tid).await? {
            post_rating_log(ctx, data, &t, stars.parse().unwrap_or(0), &comment).await;
        }
    }
    Ok(())
}

fn rated_embed(stars: i64, comment: &str) -> serenity::CreateEmbed {
    let mut d = format!("Thanks! You rated this ticket **{stars}/5**.");
    if !comment.is_empty() {
        d.push_str(&format!("\n\n> {}", comment.replace('\n', "\n> ")));
    }
    serenity::CreateEmbed::new().description(d).colour(0x57F287)
}

async fn post_rating_log(ctx: &serenity::Context, data: &Data, t: &Ticket, stars: i64, comment: &str) {
    let Ok(Some(s)) = data.db.settings(t.guild_id as u64).await else { return };
    let Some(log) = s.log_channel_id else { return };
    let mut d = format!("<@{}> rated ticket #{} ({}) **{stars}/5**", t.user_id, t.id, t.subject);
    if !comment.is_empty() {
        d.push_str(&format!("\n> {}", comment.replace('\n', "\n> ")));
    }
    serenity::ChannelId::new(log as u64)
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new()
                .embed(serenity::CreateEmbed::new().description(d).colour(0xFEE75C))
                .allowed_mentions(serenity::CreateAllowedMentions::new()),
        )
        .await
        .ok();
}

pub async fn handle_component(ctx: &serenity::Context, interaction: &serenity::Interaction, data: &Data) -> Result<()> {
    if let serenity::Interaction::Modal(m) = interaction {
        return on_modal(ctx, data, m).await;
    }
    let Some(component) = interaction.clone().message_component() else { return Ok(()) };
    let id = component.data.custom_id.clone();
    if id.starts_with("music:") {
        return crate::music::on_click(ctx, data, &component).await;
    }
    if id == "giveaway:enter" {
        return crate::giveaways::on_click(ctx, data, &component).await;
    }
    if id.starts_with("roles:") {
        return crate::roles::on_click(ctx, data, &component).await;
    }
    if id == "automod:unlock" {
        return crate::automod::on_unlock_click(ctx, data, &component).await;
    }
    if id == "verify:start" {
        return crate::verify::on_click(ctx, data, &component).await;
    }
    if id == "ticket:open" {
        defer_private(ctx, &component).await?;
        let text = match component.guild_id {
            Some(g) => open_ticket(ctx, data, g, &component.user, OpenSpec { subject: "support".into(), ..Default::default() })
                .await
                .unwrap_or_else(|e| friendly_open_error(&e)),
            None => "Tickets only work inside a server.".into(),
        };
        component.edit_response(&ctx.http, serenity::EditInteractionResponse::new().content(text)).await?;
    } else if let Some(rest) = id.strip_prefix("tp:") {
        let (pid, oid) = rest.split_once(':').unwrap_or((rest, ""));
        panel_pick(ctx, data, &component, pid.parse().unwrap_or(0), oid).await?;
    } else if let Some(pid) = id.strip_prefix("tps:") {
        let opt = match &component.data.kind {
            serenity::ComponentInteractionDataKind::StringSelect { values } => values.first().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        panel_pick(ctx, data, &component, pid.parse().unwrap_or(0), &opt).await?;
    } else if id == "ticket:close" {
        let ch = component.channel_id;
        let allowed = match (data.db.by_channel(ch.get()).await?, component.guild_id, &component.member) {
            (Some(t), Some(g), Some(m)) => {
                let s = settings(data, g).await?;
                t.user_id as u64 == component.user.id.get() || is_ticket_staff(&ctx.cache, g, component.user.id, &m.roles, &s, &t)
            }
            _ => false,
        };
        if !allowed {
            return reply_private(ctx, &component, "Only staff or the person who opened this ticket can close it.").await;
        }
        component.create_response(&ctx.http, serenity::CreateInteractionResponse::Acknowledge).await?;
        close_current(ctx, data, ch, component.user.id, "").await?;
    } else if id == "ticket:claim" {
        let (Some(t), Some(g), Some(m)) = (data.db.by_channel(component.channel_id.get()).await?, component.guild_id, component.member.clone()) else {
            return Ok(());
        };
        let s = settings(data, g).await?;
        if !is_ticket_staff(&ctx.cache, g, component.user.id, &m.roles, &s, &t) {
            return reply_private(ctx, &component, "Only staff can claim tickets.").await;
        }
        match claim(ctx, data, &t, component.user.id).await {
            Ok(notice) => {
                component
                    .create_response(
                        &ctx.http,
                        serenity::CreateInteractionResponse::Message(
                            serenity::CreateInteractionResponseMessage::new().content(notice).allowed_mentions(serenity::CreateAllowedMentions::new()),
                        ),
                    )
                    .await?;
            }
            Err(e) => reply_private(ctx, &component, e).await?,
        }
    } else if let Some(rest) = id.strip_prefix("rate:") {
        let (tid, stars) = rest.split_once(':').unwrap_or((rest, "0"));
        let (tid, stars): (i64, i64) = (tid.parse().unwrap_or(0), stars.parse().unwrap_or(0).clamp(1, 5));
        let Some(t) = data.db.by_id(tid).await? else { return Ok(()) };
        if t.user_id as u64 != component.user.id.get() {
            return reply_private(ctx, &component, "Only the person who opened the ticket can rate it.").await;
        }
        if !data.db.rate(tid, stars).await? {
            component
                .create_response(
                    &ctx.http,
                    serenity::CreateInteractionResponse::UpdateMessage(
                        serenity::CreateInteractionResponseMessage::new().embed(rated_embed(t.rating, &t.rating_comment)).components(vec![]),
                    ),
                )
                .await?;
            return Ok(());
        }
        let input = serenity::CreateInputText::new(serenity::InputTextStyle::Paragraph, "Anything to add? (optional)", "comment")
            .required(false)
            .max_length(1000);
        component
            .create_response(
                &ctx.http,
                serenity::CreateInteractionResponse::Modal(
                    serenity::CreateModal::new(format!("ratec:{tid}:{stars}"), format!("You rated {stars}/5"))
                        .components(vec![serenity::CreateActionRow::InputText(input)]),
                ),
            )
            .await?;
    }
    Ok(())
}

/// Closes the ticket in `channel_id`: summary, log + transcript, rating DM, then deletes the channel.
async fn close_current(ctx: &serenity::Context, data: &Data, channel_id: serenity::ChannelId, by_id: serenity::UserId, reason: &str) -> Result<bool> {
    let Some(t) = data.db.by_channel(channel_id.get()).await? else {
        return Ok(false);
    };
    if t.status == "closed" {
        return Ok(true);
    }
    let by_human = if by_id == ctx.cache.current_user().id { 0 } else { by_id.get() };
    data.db.close_ticket_by(t.id, by_human, reason).await?;

    let s = data.db.settings(t.guild_id as u64).await.ok().flatten();
    let ai_on = s.as_ref().map(|s| s.ai_enabled).unwrap_or(false) && data.cfg.ai_enabled;

    let transcript = data.db.transcript(t.id).await.unwrap_or_default();
    let summary = if ai_on && !transcript.is_empty() {
        data.ai
            .chat(
                "Summarize this support ticket for moderators in 2-3 bullet points: what the user needed, what was resolved, anything pending. Never use emojis or em dashes.",
                &[],
                &transcript,
            )
            .await
            .ok()
    } else {
        None
    };
    let mut head = format!("Closed by {}", by_id.mention());
    if !reason.is_empty() {
        head.push_str(&format!("\nReason: {reason}"));
    }

    channel_id
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new().embed(
                serenity::CreateEmbed::new()
                    .title("Ticket closed")
                    .description(match &summary {
                        Some(sum) => format!("{head}\n\n{sum}\n\nThis channel will be deleted in 5 seconds."),
                        None => format!("{head}\n\nThis channel will be deleted in 5 seconds."),
                    })
                    .colour(0xED4245),
            ),
        )
        .await
        .ok();

    let share = archive_ticket(ctx, data, t.id).await;
    if let Some(log_id) = s.as_ref().and_then(|s| s.log_channel_id) {
        let mut text = format!("Ticket #{} ({}) by <@{}> closed by {}", t.id, t.subject, t.user_id, by_id.mention());
        if !reason.is_empty() {
            text.push_str(&format!("\nReason: {reason}"));
        }
        if let Some(sum) = &summary {
            text.push_str(&format!("\n{sum}"));
        }
        serenity::ChannelId::new(log_id as u64)
            .send_message(
                &ctx.http,
                serenity::CreateMessage::new()
                    .content(text.chars().take(1900).collect::<String>())
                    .allowed_mentions(serenity::CreateAllowedMentions::new())
                    .components(match &share {
                        Some(url) => vec![serenity::CreateActionRow::Buttons(vec![serenity::CreateButton::new_link(url).label("View transcript")])],
                        None => vec![],
                    })
                    .add_file(serenity::CreateAttachment::bytes(transcript.into_bytes(), format!("ticket-{}.txt", t.id))),
            )
            .await
            .ok();
    }

    if s.as_ref().is_some_and(|s| s.ratings) && reason != LEFT_REASON {
        ask_rating(ctx, &t).await;
    }

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    channel_id.delete(&ctx.http).await.ok();
    Ok(true)
}

/// Copies a Discord attachment (its links expire) into the archive, losslessly compressed and
/// deduplicated, then points the logged message at `kst:<hash>`.
async fn archive_attachment(data: Data, msg_id: i64, url: String, name: String, mime: String, size: u64) {
    let Some(store) = data.store.clone() else { return };
    if size > crate::archive::MAX_FILE {
        return;
    }
    let run = async {
        let bytes = reqwest::get(&url).await?.error_for_status()?.bytes().await?.to_vec();
        let hash = crate::archive::sha256(&bytes);
        if data.db.file(&hash).await?.is_none() {
            let (n, m) = (name.clone(), mime.clone());
            let orig = bytes.len();
            let (packed, enc) = tokio::task::spawn_blocking(move || crate::archive::compress(&bytes, &n, &m)).await?;
            let key = crate::archive::file_key(&hash, enc);
            let stored = packed.len();
            store.put(&key, packed).await?;
            let mime = if mime.is_empty() { "application/octet-stream".to_string() } else { mime.clone() };
            data.db.add_file(&hash, &key, enc.as_str(), &mime, orig, stored).await?;
            tracing::info!("archived {name}: {orig} -> {stored} bytes ({})", enc.as_str());
        }
        data.db.replace_in_message(msg_id, &url, &format!("kst:{hash}")).await?;
        anyhow::Ok(())
    };
    if let Err(e) = run.await {
        tracing::warn!("attachment archive failed for {name}: {e:#}");
    }
}

/// Full ticket as zstd JSON plus a shareable HTML transcript. Returns the share link.
async fn archive_ticket(ctx: &serenity::Context, data: &Data, id: i64) -> Option<String> {
    let store = data.store.clone()?;
    let run = async {
        let t = data.db.by_id(id).await?.context("ticket gone")?;
        let msgs = data.db.messages_json(id).await?;
        let doc = crate::archive::ticket_doc(&t, msgs.clone());
        let key = crate::archive::ticket_key(t.guild_id, t.id);
        store.put(&key, crate::archive::pack_ticket(&doc)?).await?;
        data.db.set_archive(id, &key).await?;
        let token = crate::archive::random_token();
        let base = format!("{}/share/{}/{token}", dashboard_url(), t.id);
        let guild = ctx.cache.guild(serenity::GuildId::new(t.guild_id as u64)).map(|g| g.name.to_string()).unwrap_or_default();
        let html = crate::archive::render_html(&t, &guild, &msgs, &format!("{base}/files"));
        let packed = zstd::bulk::compress(html.as_bytes(), 19)?;
        store.put(&crate::archive::transcript_key(t.guild_id, t.id), packed).await?;
        data.db.set_share(id, &token).await?;
        anyhow::Ok(base)
    };
    match run.await {
        Ok(url) => Some(url),
        Err(e) => {
            tracing::warn!("ticket {id} archive failed: {e:#}");
            None
        }
    }
}

async fn ask_rating(ctx: &serenity::Context, t: &Ticket) {
    let guild_name = ctx.cache.guild(serenity::GuildId::new(t.guild_id as u64)).map(|g| g.name.to_string()).unwrap_or_else(|| "the server".into());
    let Ok(dm) = serenity::UserId::new(t.user_id as u64).create_dm_channel(&ctx.http).await else { return };
    let buttons = (1..=5)
        .map(|n| serenity::CreateButton::new(format!("rate:{}:{n}", t.id)).label(n.to_string()).style(serenity::ButtonStyle::Secondary))
        .collect();
    dm.id
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new()
                .embed(
                    serenity::CreateEmbed::new()
                        .title("How did we do?")
                        .description(format!("Your ticket **{}** in **{guild_name}** was closed. Rate the support from 1 (bad) to 5 (great).", t.subject))
                        .colour(0x5865F2),
                )
                .components(vec![serenity::CreateActionRow::Buttons(buttons)]),
        )
        .await
        .ok();
}

/// Auto-reply with AI in ticket channels.
pub async fn on_message(ctx: &serenity::Context, msg: &serenity::Message, data: &Data) -> Result<()> {
    if msg.author.bot {
        return Ok(());
    }
    let Some(t) = data.db.by_channel(msg.channel_id.get()).await? else {
        return Ok(());
    };
    if t.status != "open" {
        return Ok(());
    }
    let Some(guild_id) = msg.guild_id else { return Ok(()) };
    let Ok(s) = settings(data, guild_id).await else { return Ok(()) };

    // The opener is always the customer in their own ticket, even if they're staff elsewhere.
    let opener = t.user_id as u64;
    let staff = msg.author.id.get() != opener
        && msg.member.as_ref().is_some_and(|m| is_ticket_staff(&ctx.cache, guild_id, msg.author.id, &m.roles, &s, &t));
    let mut content = msg.content.clone();
    for a in &msg.attachments {
        content.push_str(&format!("\n[attachment: {} {}]", a.filename, a.url));
    }
    let my_id = data.db.log_message(t.id, &msg.author.name, msg.author.id.get(), &content, staff).await?;
    for a in &msg.attachments {
        tokio::spawn(archive_attachment(
            data.clone(),
            my_id,
            a.url.clone(),
            a.filename.clone(),
            a.content_type.clone().unwrap_or_default(),
            a.size as u64,
        ));
    }
    data.db.touch(t.id, staff, staff).await.ok();

    let bot_id = ctx.cache.current_user().id.get();
    if staff
        || !s.ai_enabled
        || !data.cfg.ai_enabled
        || t.escalated
        || t.claimed_by != 0
        || data.db.staff_replied(t.id, bot_id, opener).await.unwrap_or(false)
    {
        return Ok(());
    }

    let cfg = agent_config(data, guild_id).await;
    if cfg.reply_delay > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(cfg.reply_delay as u64)).await;
        // a newer message will answer everything at once
        if data.db.last_member_msg(t.id).await.unwrap_or(0) > my_id {
            return Ok(());
        }
    }
    let typing = msg.channel_id.start_typing(&ctx.http);
    let info = ticket_info(ctx, data, guild_id, &msg.author, &t).await;
    let history = data.db.history(t.id, cfg.history).await.unwrap_or_default();
    let (prior, last) = match history.split_last() {
        Some((l, p)) if l.0 == "user" => (p.to_vec(), l.1.clone()),
        _ => (history.clone(), content.clone()),
    };
    let knowledge = crate::archive::load_knowledge(data.store.as_deref(), &cfg.knowledge).await;
    let out = agent::run(&data.ai, &cfg, &info, &knowledge, &prior, &last).await;
    drop(typing);

    let out = match out {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("agent failed in ticket {}: {e:#}", t.id);
            msg.channel_id
                .say(&ctx.http, "I couldn't answer that right now. Want me to get a staff member? Just say yes.")
                .await
                .ok();
            return Ok(());
        }
    };
    if !out.text.is_empty() {
        let text: String = out.text.chars().take(cfg.max_reply.max(200) as usize + 400).collect();
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(1900) {
            msg.channel_id.say(&ctx.http, chunk.iter().collect::<String>()).await?;
        }
        data.db.log_message(t.id, "Kev", bot_id, &text, true).await?;
        data.db.touch(t.id, true, false).await.ok();
    }
    for action in out.actions {
        match action {
            Action::Escalate(reason) => escalate(ctx, data, &t, &s, guild_id, &reason).await?,
            Action::Priority(p) => {
                data.db.set_priority(t.id, &p).await.ok();
            }
            Action::Rename(topic) => rename_ticket(ctx, data, &t, &topic).await,
            Action::Note(note) => staff_note(ctx, data, &t, &s, &note).await,
            Action::Close(reason) => {
                let me = ctx.cache.current_user().id;
                close_current(ctx, data, msg.channel_id, me, &reason).await?;
                break;
            }
        }
    }
    Ok(())
}

async fn rename_ticket(ctx: &serenity::Context, data: &Data, t: &Ticket, topic: &str) {
    data.db.set_subject(t.id, topic).await.ok();
    let ch = serenity::ChannelId::new(t.channel_id as u64);
    let slug = agent::slug(topic).replace('_', "-");
    let Ok(serenity::Channel::Guild(c)) = ch.to_channel(&ctx.http).await else { return };
    let prefix: Vec<&str> = c.name.splitn(3, '-').take(2).collect();
    let name: String = format!("{}-{slug}", prefix.join("-")).chars().take(90).collect();
    ch.edit(&ctx.http, serenity::EditChannel::new().name(name).topic(format!("{topic} (ticket #{})", t.id))).await.ok();
}

/// Private note for staff: kept out of the member's view and the AI history; posted to the log channel if set.
async fn staff_note(ctx: &serenity::Context, data: &Data, t: &Ticket, s: &Settings, note: &str) {
    let bot = ctx.cache.current_user().id.get();
    data.db.log_kind(t.id, "Staff note (AI)", bot, note, 2).await.ok();
    if let Some(log) = s.log_channel_id {
        serenity::ChannelId::new(log as u64)
            .send_message(
                &ctx.http,
                serenity::CreateMessage::new()
                    .embed(
                        serenity::CreateEmbed::new()
                            .title(format!("Note on ticket #{} ({})", t.id, t.subject))
                            .description(format!("{note}\n\n<#{}>", t.channel_id))
                            .colour(0x2B2D31),
                    )
                    .allowed_mentions(serenity::CreateAllowedMentions::new()),
            )
            .await
            .ok();
    }
}

async fn agent_config(data: &Data, guild: serenity::GuildId) -> AgentConfig {
    data.db.agent_config(guild.get()).await.ok().flatten().map(|c| AgentConfig::parse(&c)).unwrap_or_default()
}

/// Pings the ticket type's ping roles, the staff role, or the owner; the AI stays quiet from now on.
async fn escalate(ctx: &serenity::Context, data: &Data, t: &Ticket, s: &Settings, guild: serenity::GuildId, reason: &str) -> Result<()> {
    data.db.set_escalated(t.id).await?;
    let ping = if !t.ping_roles.is_empty() {
        t.ping_roles.iter().map(|r| format!("<@&{r}>")).collect::<Vec<_>>().join(" ")
    } else if s.mod_role_id != 0 {
        format!("<@&{}>", s.mod_role_id)
    } else {
        guild_owner(ctx, guild).await.map(|o| o.mention().to_string()).unwrap_or_default()
    };
    serenity::ChannelId::new(t.channel_id as u64)
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new()
                .content(format!("{ping} <@{}> needs a human.", t.user_id))
                .embed(
                    serenity::CreateEmbed::new()
                        .title("Escalated to staff")
                        .description(if reason.is_empty() { "No details given." } else { reason })
                        .colour(0xFEE75C),
                )
                .allowed_mentions(serenity::CreateAllowedMentions::new().all_roles(true).all_users(true)),
        )
        .await?;
    Ok(())
}

/// Every 10 minutes: close tickets whose member left, warn and then close inactive ones.
pub async fn sweeper(ctx: serenity::Context, data: Data) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
    loop {
        tick.tick().await;
        if let Err(e) = sweep(&ctx, &data).await {
            tracing::warn!("sweep failed: {e:#}");
        }
    }
}

async fn sweep(ctx: &serenity::Context, data: &Data) -> Result<()> {
    let open = data.db.all_open().await?;
    let mut cache: std::collections::HashMap<i64, Option<Settings>> = Default::default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
    let me = ctx.cache.current_user().id;
    for t in open {
        if !cache.contains_key(&t.guild_id) {
            cache.insert(t.guild_id, data.db.settings(t.guild_id as u64).await.ok().flatten());
        }
        let Some(s) = cache.get(&t.guild_id).cloned().flatten() else { continue };
        let guild = serenity::GuildId::new(t.guild_id as u64);
        let ch = serenity::ChannelId::new(t.channel_id as u64);
        if s.close_on_leave {
            if let Err(serenity::Error::Http(e)) = guild.member(&ctx.http, serenity::UserId::new(t.user_id as u64)).await {
                if e.status_code().map(|c| c.as_u16()) == Some(404) {
                    ch.say(&ctx.http, "The member who opened this ticket left the server.").await.ok();
                    close_current(ctx, data, ch, me, LEFT_REASON).await.ok();
                    continue;
                }
            }
        }
        if s.autoclose_hours > 0 {
            let limit = s.autoclose_hours * 3600;
            let idle = now - t.last_activity.max(t.created_at);
            if idle >= limit {
                close_current(ctx, data, ch, me, &format!("No activity for {} hours", s.autoclose_hours)).await.ok();
            } else if idle >= limit * 3 / 4 && !t.warned {
                data.db.set_warned(t.id).await.ok();
                let left = ((limit - idle) as f64 / 3600.0).ceil() as i64;
                ch.send_message(
                    &ctx.http,
                    serenity::CreateMessage::new()
                        .content(format!("<@{}> this ticket closes in about {left} hour{} unless someone replies.", t.user_id, if left == 1 { "" } else { "s" }))
                        .allowed_mentions(serenity::CreateAllowedMentions::new().users(vec![serenity::UserId::new(t.user_id as u64)])),
                )
                .await
                .ok();
            }
        }
    }
    Ok(())
}

/// The dashboard posts panels through the bot's HTTP client.
pub async fn post_panel(http: &serenity::Http, channel: u64, panel: &Panel) -> Result<()> {
    let (embed, rows) = panel.render();
    serenity::ChannelId::new(channel).send_message(http, serenity::CreateMessage::new().embed(embed).components(rows)).await?;
    Ok(())
}
