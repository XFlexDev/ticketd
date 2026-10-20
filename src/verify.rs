//! Captcha verification: a Verify button in Discord opens a Turnstile page on the dashboard, passing gives a role.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::Result;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct VerifyConfig {
    #[serde(default)]
    pub role_id: String,
    #[serde(default)]
    pub remove_role_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub button: String,
}

impl VerifyConfig {
    pub fn parse(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn validate(&mut self) -> Result<(), String> {
        let id_ok = |s: &str| s.is_empty() || (s.len() < 25 && s.bytes().all(|b| b.is_ascii_digit()));
        if self.role_id.is_empty() || !id_ok(&self.role_id) || !id_ok(&self.remove_role_id) {
            return Err("Pick the role members get after the captcha".into());
        }
        if self.remove_role_id == self.role_id {
            self.remove_role_id.clear();
        }
        self.title = self.title.trim().chars().take(256).collect();
        self.description = self.description.trim().chars().take(4000).collect();
        self.button = self.button.trim().chars().take(80).collect();
        self.color = self.color.trim().chars().take(7).collect();
        Ok(())
    }

    pub fn render(&self) -> (serenity::CreateEmbed, Vec<serenity::CreateActionRow>) {
        let or = |s: &str, d: &str| if s.is_empty() { d.to_string() } else { s.to_string() };
        let color = u32::from_str_radix(self.color.trim_start_matches('#'), 16).unwrap_or(0x66BB6A);
        let embed = serenity::CreateEmbed::new()
            .title(or(&self.title, "Verify you're human"))
            .description(or(&self.description, "Press the button below and solve a quick captcha to get access to the server."))
            .color(color);
        let button = serenity::CreateButton::new("verify:start").label(or(&self.button, "Verify")).style(serenity::ButtonStyle::Success);
        (embed, vec![serenity::CreateActionRow::Buttons(vec![button])])
    }
}

pub struct Pending {
    pub guild: u64,
    pub user: u64,
    pub guild_name: String,
    pub expires: i64,
}

static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);
const LINK_SECS: i64 = 600;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// (guild, user, guild name) for a live link.
pub fn peek(token: &str) -> Option<(u64, u64, String)> {
    let mut p = PENDING.lock().unwrap();
    p.retain(|_, v| v.expires > now());
    p.get(token).map(|v| (v.guild, v.user, v.guild_name.clone()))
}

pub fn consume(token: &str) {
    PENDING.lock().unwrap().remove(token);
}

async fn private(ctx: &serenity::Context, c: &serenity::ComponentInteraction, msg: serenity::CreateInteractionResponseMessage) -> Result<()> {
    c.create_response(&ctx.http, serenity::CreateInteractionResponse::Message(msg.ephemeral(true))).await?;
    Ok(())
}

pub async fn on_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<()> {
    let text = |t: &str| serenity::CreateInteractionResponseMessage::new().content(t);
    let Some(g) = c.guild_id else { return private(ctx, c, text("This only works inside a server.")).await };
    let cfg = data.db.verify_config(g.get()).await?.map(|s| VerifyConfig::parse(&s)).unwrap_or_default();
    let Ok(role) = cfg.role_id.parse::<u64>() else {
        return private(ctx, c, text("Verification isn't set up yet. Ask an admin to finish it in the dashboard.")).await;
    };
    if c.member.as_ref().is_some_and(|m| m.roles.contains(&serenity::RoleId::new(role))) {
        return private(ctx, c, text("You're already verified.")).await;
    }
    let token = crate::archive::random_token();
    let guild_name = g.name(&ctx.cache).unwrap_or_else(|| "this server".into());
    PENDING.lock().unwrap().insert(token.clone(), Pending { guild: g.get(), user: c.user.id.get(), guild_name, expires: now() + LINK_SECS });
    let base = std::env::var("DASHBOARD_BASE_URL").unwrap_or_else(|_| "https://kev.kreatix.dev".into());
    let link = serenity::CreateButton::new_link(format!("{}/verify/{token}", base.trim_end_matches('/'))).label("Open captcha");
    private(
        ctx,
        c,
        text("Solve the captcha to get access. The link only works for you and expires in 10 minutes.")
            .components(vec![serenity::CreateActionRow::Buttons(vec![link])]),
    )
    .await
}

pub fn page(guild_name: Option<&str>, sitekey: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let body = match guild_name {
        Some(name) => format!(
            r#"<p class="sub">{}</p><h1>Verify you're human</h1><p class="muted">Finish the check below to get access.</p>
<div id="cf" class="cf-turnstile" data-sitekey="{}" data-theme="dark" data-callback="done"></div><p id="msg" class="muted"></p>"#,
            esc(name),
            esc(sitekey)
        ),
        None => r#"<h1>Link expired</h1><p class="muted">Go back to Discord and press Verify again.</p>"#.into(),
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Verify - Kev</title>
<style>html,body{{margin:0;height:100%;background:#000;color:#fff;font-family:Inter,system-ui,-apple-system,sans-serif}}
main{{min-height:100%;display:flex;flex-direction:column;justify-content:center;align-items:center;text-align:center;padding:24px;box-sizing:border-box}}
h1{{font-size:28px;letter-spacing:-.5px;margin:6px 0 8px}}.sub{{color:#66bb6a;font-weight:600;margin:0}}.muted{{color:#9a9a9a;margin:0 0 24px;line-height:1.5}}
#cf{{min-height:66px}}#msg{{margin-top:18px;min-height:24px}}.ok{{color:#66bb6a!important}}.err{{color:#ef5350!important}}</style>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script></head><body><main>{body}</main>
<script>async function done(t){{const m=document.getElementById('msg');m.className='muted';m.textContent='Checking...';
try{{const r=await fetch(location.pathname,{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{token:t}})}});const j=await r.json();
if(j.ok){{document.getElementById('cf').remove();document.querySelector('h1').textContent="You're verified";m.className='ok';m.textContent='You can close this tab and go back to Discord.'}}
else{{m.className='err';m.textContent=j.error||'That did not work. Try again.';window.turnstile&&turnstile.reset()}}}}catch(e){{m.className='err';m.textContent='Network error. Try again.'}}}}</script></body></html>"#
    )
}
