mod agent;
mod alerts;
mod archive;
mod ai;
mod automod;
mod custom;
mod extras;
mod db;
mod gc;
mod giveaways;
mod leveling;
mod logs;
mod moderation;
mod music;
mod panels;
mod roles;
mod tickets;
mod utility;
mod verify;
mod web;
mod welcome;

use anyhow::Result;
use poise::serenity_prelude as serenity;
use songbird::SerenityInit;
use std::sync::Arc;
use tracing::info;
use std::sync::Mutex;

pub static WEB_TX: Mutex<Option<tokio::sync::oneshot::Sender<std::sync::Arc<Data>>>> = Mutex::new(None);

#[derive(Clone)]
pub struct Data {
    pub db: db::Kdb,
    pub cfg: Arc<Config>,
    pub ai: Arc<ai::Client>,
    pub store: Option<Arc<archive::Store>>,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;

pub struct Config {
    /// Commands register into this guild instantly when set (dev); otherwise global.
    pub dev_guild_id: Option<serenity::GuildId>,
    pub ai_enabled: bool,
    pub ticket_prefix: String,
}

fn env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("missing env var {name}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().init();
    // ring and aws-lc-rs are both compiled in (songbird vs reqwest), so rustls can't pick one itself.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let token = env("DISCORD_TOKEN")?;
    let cfg = Arc::new(Config {
        dev_guild_id: std::env::var("DEV_GUILD_ID").ok().and_then(|v| v.parse().ok()).map(serenity::GuildId::new),
        ai_enabled: std::env::var("AI_ENABLED").map(|v| v == "1").unwrap_or(true),
        ticket_prefix: std::env::var("TICKET_PREFIX").unwrap_or_else(|_| "ticket".into()),
    });

    let db = db::Kdb::from_env()?;
    db.init().await?;
    let ai = Arc::new(ai::Client::from_env()?);

    // optional web dashboard/API
    if std::env::var("DASHBOARD_ENABLED").map(|v| v == "1").unwrap_or(false) {
        // share Data with the web server once framework setup finishes
        // poise doesn't expose Data until setup, so grab it via a one-shot channel
        let (tx, rx) = tokio::sync::oneshot::channel::<std::sync::Arc<Data>>();
        WEB_TX.lock().unwrap().replace(tx);
        tokio::spawn(async move {
            if let Ok(data) = rx.await {
                if let Ok(app) = web::router(data) {
                    let port = std::env::var("DASHBOARD_PORT").unwrap_or_else(|_| "8420".into());
                    if let Ok(listener) = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await {
                        info!("dashboard on :{port}");
                        axum::serve(listener, app).await.ok();
                    }
                }
            }
        });
    }

    let base = serenity::GatewayIntents::GUILDS
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MODERATION
        | serenity::GatewayIntents::GUILD_VOICE_STATES
        | serenity::GatewayIntents::GUILD_MESSAGE_REACTIONS;
    // Server Members is privileged: if it isn't enabled in the dev portal, run without it.
    for intents in [base | serenity::GatewayIntents::GUILD_MEMBERS, base] {
        let framework = build_framework(db.clone(), cfg.clone(), ai.clone());
        let mut cache = serenity::cache::Settings::default();
        cache.max_messages = 200;
        let mut client = serenity::ClientBuilder::new(&token, intents).framework(framework).cache_settings(cache).register_songbird().await?;
        match client.start().await {
            Err(e) if intents.contains(serenity::GatewayIntents::GUILD_MEMBERS) && e.to_string().to_lowercase().contains("disallowed") => {
                tracing::warn!("Server Members intent is off in the Discord dev portal: welcome, join logs and anti-raid are disabled");
            }
            other => return other.map_err(Into::into),
        }
    }
    Ok(())
}

fn build_framework(db: db::Kdb, cfg: Arc<Config>, ai: Arc<ai::Client>) -> poise::Framework<Data, Error> {
    let dev_guild = cfg.dev_guild_id;
    let options = poise::FrameworkOptions {
        commands: custom::reserve(vec![
            tickets::ticket_open(),
            tickets::ticket_close(),
            tickets::ticket_add(),
            tickets::ticket_remove(),
            tickets::ticket_panel(),
            tickets::ticket_transcript(),
            tickets::setup(),
            tickets::ticket_claim(),
            tickets::ticket_unclaim(),
            tickets::ticket_transfer(),
            tickets::ticket_priority(),
            moderation::ban(),
            moderation::unban(),
            moderation::kick(),
            moderation::timeout(),
            moderation::untimeout(),
            moderation::warn(),
            moderation::unwarn(),
            moderation::warnings(),
            moderation::purge(),
            moderation::slowmode(),
            moderation::lock(),
            moderation::unlock(),
            leveling::rank(),
            leveling::leaderboard(),
            leveling::xp(),
            giveaways::giveaway(),
            utility::poll(),
            utility::remind(),
            utility::reminders(),
            music::play(),
            music::radio(),
            music::skip(),
            music::pause(),
            music::stop(),
            music::queue(),
            music::volume(),
            extras::userinfo(),
            extras::serverinfo(),
            extras::avatar(),
            extras::banner(),
            extras::roleinfo(),
            extras::afk(),
            extras::sticky(),
            extras::suggest(),
        ]),
        on_error: |err| Box::pin(on_error(err)),
        event_handler: |fctx, event| {
            Box::pin(async move {
                let ctx = fctx.serenity_context;
                let data = fctx.user_data;
                if let serenity::FullEvent::InteractionCreate { interaction, .. } = event {
                    custom::on_command(ctx, data, interaction).await;
                    tickets::handle_component(ctx, interaction, data).await?;
                }
                if let serenity::FullEvent::Message { new_message, .. } = event {
                    if !automod::on_message(ctx, new_message, data).await {
                        leveling::on_message(ctx, new_message, data).await;
                        custom::on_message(ctx, new_message, data).await;
                        extras::on_message(ctx, new_message, data).await;
                        tickets::on_message(ctx, new_message, data).await?;
                    }
                }
                if let serenity::FullEvent::GuildMemberAddition { new_member } = event {
                    if !automod::on_join(ctx, data, new_member).await? {
                        welcome::on_join(ctx, data, new_member).await?;
                    }
                }
                if let serenity::FullEvent::GuildMemberRemoval { guild_id, user, .. } = event {
                    welcome::on_leave(ctx, data, *guild_id, user).await?;
                }
                if let serenity::FullEvent::ReactionAdd { add_reaction } = event {
                    extras::on_reaction(ctx, data, add_reaction).await;
                }
                if let serenity::FullEvent::ReactionRemove { removed_reaction } = event {
                    extras::on_reaction(ctx, data, removed_reaction).await;
                }
                if let serenity::FullEvent::VoiceStateUpdate { old, new } = event {
                    music::on_voice(ctx, data, old.as_ref(), new).await;
                }
                logs::on_event(ctx, data, event).await;
                Ok(())
            })
        },
        ..Default::default()
    };

    poise::Framework::builder()
        .options(options)
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                match dev_guild {
                    Some(g) => {
                        poise::builtins::register_in_guild(ctx, &framework.options().commands, g).await?;
                        info!("registered commands in dev guild {g}");
                    }
                    None => {
                        poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                        info!("registered commands globally");
                    }
                }
                let store = archive::Store::from_env().map(Arc::new);
                if store.is_none() {
                    tracing::warn!("KST_* not set: ticket archive disabled");
                }
                let data = Data { db, cfg, ai, store };
                tokio::spawn(tickets::sweeper(ctx.clone(), data.clone()));
                tokio::spawn(moderation::sweeper(ctx.clone(), data.clone()));
                tokio::spawn(giveaways::sweeper(ctx.clone(), data.clone()));
                tokio::spawn(utility::sweeper(ctx.clone(), data.clone()));
                tokio::spawn(alerts::poller(ctx.clone(), data.clone()));
                tokio::spawn(music::startup(ctx.clone(), data.clone()));
                tokio::spawn(extras::startup(data.clone()));
                tokio::spawn(automod::scam_refresher());
                if let Some(store) = data.store.clone() {
                    tokio::spawn(gc::sweeper(data.db.clone(), store));
                }
                if let Some(tx) = WEB_TX.lock().unwrap().take() {
                    let _ = tx.send(std::sync::Arc::new(data.clone()));
                }
                Ok(data)
            })
        })
        .build()

}

async fn on_error(err: poise::FrameworkError<'_, Data, anyhow::Error>) {
    match err {
        poise::FrameworkError::Command { error, ctx, .. } => {
            tracing::warn!("/{} failed: {error:#}", ctx.command().name);
            let raw = error.to_string();
            let text = if raw.contains("Invalid permissions") || raw.contains("Missing Permissions") || raw.contains("Missing Access") {
                "I'm missing permissions for that. Give my role **Manage Channels**, **Manage Roles** and **Send Messages**, or re-invite me from kev.kreatix.dev.".to_string()
            } else {
                format!("Something went wrong: {raw}")
            };
            let _ = ctx.send(poise::CreateReply::default().content(text).ephemeral(true)).await;
        }
        other => {
            let _ = poise::builtins::on_error(other).await;
        }
    }
}
