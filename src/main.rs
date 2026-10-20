mod ai;
mod db;
mod tickets;

use anyhow::Result;
use poise::serenity_prelude as serenity;
use std::sync::Arc;
use tracing::info;

pub struct Data {
    pub db: db::Kdb,
    pub cfg: Arc<Config>,
    pub ai: ai::Client,
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

    let token = env("DISCORD_TOKEN")?;
    let cfg = Arc::new(Config {
        dev_guild_id: std::env::var("DEV_GUILD_ID").ok().and_then(|v| v.parse().ok()).map(serenity::GuildId::new),
        ai_enabled: std::env::var("AI_ENABLED").map(|v| v == "1").unwrap_or(true),
        ticket_prefix: std::env::var("TICKET_PREFIX").unwrap_or_else(|_| "ticket".into()),
    });

    let db = db::Kdb::from_env()?;
    db.init().await?;
    let ai = ai::Client::from_env()?;

    let options = poise::FrameworkOptions {
        commands: vec![
            tickets::ticket_open(),
            tickets::ticket_close(),
            tickets::ticket_add(),
            tickets::ticket_remove(),
            tickets::ticket_panel(),
            tickets::ticket_transcript(),
            tickets::setup(),
        ],
        event_handler: |fctx, event| {
            Box::pin(async move {
                let ctx = fctx.serenity_context;
                let data = fctx.user_data;
                if let serenity::FullEvent::InteractionCreate { interaction, .. } = event {
                    tickets::handle_component(ctx, interaction, data).await?;
                }
                if let serenity::FullEvent::Message { new_message, .. } = event {
                    tickets::on_message(ctx, new_message, data).await?;
                }
                Ok(())
            })
        },
        ..Default::default()
    };

    let dev_guild = cfg.dev_guild_id;
    let framework = poise::Framework::builder()
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
                Ok(Data { db, cfg, ai })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::GUILDS
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::ClientBuilder::new(token, intents).framework(framework).await?;
    client.start().await?;
    Ok(())
}
