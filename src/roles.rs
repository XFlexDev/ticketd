//! Self-role menus: buttons or a dropdown members use to pick their own roles.

use anyhow::Result;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RoleOption {
    pub role_id: String,
    pub label: String,
    pub emoji: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Menu {
    pub id: String,
    pub title: String,
    pub description: String,
    pub color: String,
    /// "buttons" or "dropdown"
    pub style: String,
    /// "toggle": pick any, "unique": only one, "add": can't remove
    pub mode: String,
    pub roles: Vec<RoleOption>,
}

impl Default for Menu {
    fn default() -> Self {
        Self { id: String::new(), title: "Pick your roles".into(), description: String::new(), color: "#66BB6A".into(), style: "buttons".into(), mode: "toggle".into(), roles: vec![] }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RolesConfig {
    pub menus: Vec<Menu>,
}

impl RolesConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.menus.len() > 25 {
            return Err("25 menus max".into());
        }
        for (i, m) in self.menus.iter_mut().enumerate() {
            if m.id.is_empty() || m.id.len() > 16 || !m.id.chars().all(|c| c.is_ascii_alphanumeric()) {
                m.id = format!("{:x}", rand_id(i));
            }
            if !matches!(m.style.as_str(), "buttons" | "dropdown") {
                m.style = "buttons".into();
            }
            if !matches!(m.mode.as_str(), "toggle" | "unique" | "add") {
                m.mode = "toggle".into();
            }
            m.title = m.title.trim().chars().take(256).collect();
            m.description = m.description.trim().chars().take(2000).collect();
            m.color = m.color.trim().chars().take(7).collect();
            m.roles.retain(|r| r.role_id.parse::<u64>().is_ok());
            let mut seen = std::collections::HashSet::new();
            m.roles.retain(|r| seen.insert(r.role_id.clone()));
            if m.roles.len() > 25 {
                return Err(format!("\"{}\" has more than 25 roles", m.title));
            }
            for r in &mut m.roles {
                r.label = r.label.trim().chars().take(80).collect();
                r.description = r.description.trim().chars().take(100).collect();
                r.emoji = r.emoji.trim().chars().take(64).collect();
            }
        }
        let mut ids = std::collections::HashSet::new();
        for (i, m) in self.menus.iter_mut().enumerate() {
            if !ids.insert(m.id.clone()) {
                m.id = format!("{:x}", rand_id(i + 100));
                ids.insert(m.id.clone());
            }
        }
        Ok(())
    }
}

fn rand_id(salt: usize) -> u64 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    n.wrapping_mul(6364136223846793005).wrapping_add(salt as u64) >> 20
}

pub async fn config(data: &Data, guild: u64) -> RolesConfig {
    data.db.module_config(guild, "roles").await.ok().flatten().and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default()
}

fn emoji(s: &str) -> Option<serenity::ReactionType> {
    if s.is_empty() {
        return None;
    }
    serenity::ReactionType::try_from(s).ok()
}

/// Role names for labels left empty.
pub fn build_message(menu: &Menu, names: &dyn Fn(&str) -> String) -> serenity::CreateMessage {
    let color = u32::from_str_radix(menu.color.trim_start_matches('#'), 16).unwrap_or(0x66BB6A);
    let mut e = serenity::CreateEmbed::new().title(&menu.title).color(color);
    if !menu.description.is_empty() {
        e = e.description(&menu.description);
    }
    let label = |r: &RoleOption| if r.label.is_empty() { names(&r.role_id) } else { r.label.clone() };
    let rows = if menu.style == "dropdown" {
        let opts = menu
            .roles
            .iter()
            .map(|r| {
                let mut o = serenity::CreateSelectMenuOption::new(label(r), &r.role_id);
                if !r.description.is_empty() {
                    o = o.description(&r.description);
                }
                if let Some(em) = emoji(&r.emoji) {
                    o = o.emoji(em);
                }
                o
            })
            .collect::<Vec<_>>();
        let max = if menu.mode == "unique" { 1 } else { opts.len() as u8 };
        vec![serenity::CreateActionRow::SelectMenu(
            serenity::CreateSelectMenu::new(format!("roles:{}", menu.id), serenity::CreateSelectMenuKind::String { options: opts })
                .placeholder("Pick roles")
                .min_values(0)
                .max_values(max.max(1)),
        )]
    } else {
        menu.roles
            .chunks(5)
            .map(|chunk| {
                serenity::CreateActionRow::Buttons(
                    chunk
                        .iter()
                        .map(|r| {
                            let mut b = serenity::CreateButton::new(format!("roles:{}:{}", menu.id, r.role_id)).label(label(r)).style(serenity::ButtonStyle::Secondary);
                            if let Some(em) = emoji(&r.emoji) {
                                b = b.emoji(em);
                            }
                            b
                        })
                        .collect(),
                )
            })
            .collect()
    };
    serenity::CreateMessage::new().embed(e).components(rows)
}

pub async fn on_click(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> Result<()> {
    let Some(guild) = c.guild_id else { return Ok(()) };
    let reply = |text: String| serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(text).ephemeral(true));
    let mut parts = c.data.custom_id.splitn(3, ':').skip(1);
    let menu_id = parts.next().unwrap_or_default().to_string();
    let clicked = parts.next().map(String::from);
    let cfg = config(data, guild.get()).await;
    let Some(menu) = cfg.menus.into_iter().find(|m| m.id == menu_id) else {
        c.create_response(&ctx.http, reply("This role menu was deleted.".into())).await?;
        return Ok(());
    };
    let Some(member) = c.member.as_ref() else { return Ok(()) };
    let menu_roles: Vec<serenity::RoleId> = menu.roles.iter().filter_map(|r| r.role_id.parse().ok()).map(serenity::RoleId::new).collect();
    let has = |r: &serenity::RoleId| member.roles.contains(r);

    let (mut add, mut remove): (Vec<serenity::RoleId>, Vec<serenity::RoleId>) = (vec![], vec![]);
    match (clicked, &c.data.kind) {
        (Some(role), _) => {
            let Some(role) = role.parse::<u64>().ok().map(serenity::RoleId::new).filter(|r| menu_roles.contains(r)) else { return Ok(()) };
            if has(&role) {
                if menu.mode != "add" {
                    remove.push(role);
                }
            } else {
                add.push(role);
                if menu.mode == "unique" {
                    remove.extend(menu_roles.iter().filter(|r| **r != role && has(r)));
                }
            }
        }
        (None, serenity::ComponentInteractionDataKind::StringSelect { values }) => {
            let picked: Vec<serenity::RoleId> = values.iter().filter_map(|v| v.parse().ok()).map(serenity::RoleId::new).filter(|r| menu_roles.contains(r)).collect();
            add.extend(picked.iter().filter(|r| !has(r)));
            if menu.mode != "add" {
                remove.extend(menu_roles.iter().filter(|r| has(r) && !picked.contains(r)));
            }
        }
        _ => return Ok(()),
    }

    let mut failed = false;
    for r in &add {
        failed |= ctx.http.add_member_role(guild, member.user.id, *r, Some("Kev self role")).await.is_err();
    }
    for r in &remove {
        failed |= ctx.http.remove_member_role(guild, member.user.id, *r, Some("Kev self role")).await.is_err();
    }
    let fmt = |v: &[serenity::RoleId]| v.iter().map(|r| format!("<@&{r}>")).collect::<Vec<_>>().join(" ");
    let mut lines = vec![];
    if !add.is_empty() {
        lines.push(format!("Added {}", fmt(&add)));
    }
    if !remove.is_empty() {
        lines.push(format!("Removed {}", fmt(&remove)));
    }
    if failed {
        lines.push("Some roles couldn't be changed. Kev's role has to be above them and needs Manage Roles.".into());
    }
    if lines.is_empty() {
        lines.push("Nothing changed.".into());
    }
    c.create_response(&ctx.http, reply(lines.join("\n"))).await?;
    Ok(())
}
