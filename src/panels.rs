use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Question {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub long: bool,
    #[serde(default)]
    pub required: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PanelOption {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub category_id: String,
    #[serde(default)]
    pub staff_roles: Vec<String>,
    #[serde(default)]
    pub ping_roles: Vec<String>,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub instructions: String,
}

/// A ticket panel: one embed plus buttons or a dropdown, one entry per ticket type.
#[derive(Serialize, Deserialize, Clone)]
pub struct Panel {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub author_icon: String,
    #[serde(default)]
    pub footer: String,
    #[serde(default)]
    pub footer_icon: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub options: Vec<PanelOption>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Support".into(),
            title: "Support".into(),
            description: "Need help? Open a ticket and we'll get to you.".into(),
            color: "#5865F2".into(),
            image: String::new(),
            thumbnail: String::new(),
            author_name: String::new(),
            author_icon: String::new(),
            footer: String::new(),
            footer_icon: String::new(),
            style: "buttons".into(),
            placeholder: String::new(),
            options: vec![PanelOption { id: "default".into(), label: "Open a ticket".into(), color: "primary".into(), ..Default::default() }],
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn random_id() -> String {
    use std::io::Read;
    let mut b = [0u8; 6];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn snowflakes(v: &mut Vec<String>) {
    v.retain(|r| !r.is_empty() && r.len() < 21 && r.bytes().all(|b| b.is_ascii_digit()));
    v.dedup();
    v.truncate(10);
}

impl Panel {
    pub fn parse(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    /// Trims, fills ids and enforces Discord's limits. Err = message for the dashboard.
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = clip(&self.name, 60);
        if self.name.is_empty() {
            return Err("Give the panel a name".into());
        }
        self.title = clip(&self.title, 256);
        self.description = clip(&self.description, 4000);
        self.placeholder = clip(&self.placeholder, 150);
        self.author_name = clip(&self.author_name, 256);
        self.footer = clip(&self.footer, 2048);
        for (label, url) in [
            ("Image", &mut self.image),
            ("Thumbnail", &mut self.thumbnail),
            ("Author icon", &mut self.author_icon),
            ("Footer icon", &mut self.footer_icon),
        ] {
            *url = url.trim().into();
            if !url.is_empty() && !url.starts_with("https://") {
                return Err(format!("{label} must be an https:// link"));
            }
        }
        if parse_color(&self.color).is_none() {
            self.color = "#5865F2".into();
        }
        self.style = if self.style == "select" { "select".into() } else { "buttons".into() };
        self.options.retain(|o| !o.label.trim().is_empty());
        if self.options.is_empty() {
            return Err("Add at least one ticket type".into());
        }
        if self.options.len() > 25 {
            return Err("A panel can have at most 25 ticket types".into());
        }
        let mut seen = std::collections::HashSet::new();
        for o in &mut self.options {
            o.label = clip(&o.label, 80);
            o.description = clip(&o.description, 100);
            o.emoji = o.emoji.trim().into();
            o.instructions = clip(&o.instructions, 4000);
            if !["primary", "secondary", "success", "danger"].contains(&o.color.as_str()) {
                o.color = "primary".into();
            }
            if o.id.is_empty() || o.id.len() > 20 || !o.id.bytes().all(|b| b.is_ascii_alphanumeric()) || !seen.insert(o.id.clone()) {
                o.id = random_id();
                seen.insert(o.id.clone());
            }
            if !o.category_id.bytes().all(|b| b.is_ascii_digit()) {
                o.category_id.clear();
            }
            snowflakes(&mut o.staff_roles);
            snowflakes(&mut o.ping_roles);
            o.questions.retain(|q| !q.label.trim().is_empty());
            if o.questions.len() > 5 {
                return Err(format!("\"{}\" has more than 5 questions (Discord's limit)", o.label));
            }
            for q in &mut o.questions {
                q.label = clip(&q.label, 45);
                q.placeholder = clip(&q.placeholder, 100);
            }
        }
        Ok(())
    }

    pub fn option(&self, id: &str) -> Option<&PanelOption> {
        self.options.iter().find(|o| o.id == id)
    }

    /// The embed and components that get posted in Discord.
    pub fn render(&self) -> (serenity::CreateEmbed, Vec<serenity::CreateActionRow>) {
        let mut embed = serenity::CreateEmbed::new().colour(parse_color(&self.color).unwrap_or(0x5865F2));
        if !self.title.is_empty() {
            embed = embed.title(&self.title);
        }
        if !self.description.is_empty() {
            embed = embed.description(&self.description);
        }
        embed = media(embed, &self.image, &self.thumbnail, &self.author_name, &self.author_icon, &self.footer, &self.footer_icon);
        let rows = if self.style == "select" {
            let options = self
                .options
                .iter()
                .map(|o| {
                    let mut opt = serenity::CreateSelectMenuOption::new(&o.label, &o.id);
                    if !o.description.is_empty() {
                        opt = opt.description(&o.description);
                    }
                    if let Some(e) = parse_emoji(&o.emoji) {
                        opt = opt.emoji(e);
                    }
                    opt
                })
                .collect();
            let menu = serenity::CreateSelectMenu::new(format!("tps:{}", self.id), serenity::CreateSelectMenuKind::String { options })
                .placeholder(if self.placeholder.is_empty() { "Pick what you need help with" } else { &self.placeholder });
            vec![serenity::CreateActionRow::SelectMenu(menu)]
        } else {
            self.options
                .chunks(5)
                .map(|chunk| {
                    serenity::CreateActionRow::Buttons(
                        chunk
                            .iter()
                            .map(|o| {
                                let mut b = serenity::CreateButton::new(format!("tp:{}:{}", self.id, o.id)).label(&o.label).style(match o.color.as_str() {
                                    "secondary" => serenity::ButtonStyle::Secondary,
                                    "success" => serenity::ButtonStyle::Success,
                                    "danger" => serenity::ButtonStyle::Danger,
                                    _ => serenity::ButtonStyle::Primary,
                                });
                                if let Some(e) = parse_emoji(&o.emoji) {
                                    b = b.emoji(e);
                                }
                                b
                            })
                            .collect(),
                    )
                })
                .collect()
        };
        (embed, rows)
    }
}

/// Image, thumbnail, author line and footer; empty values are skipped.
pub fn media(
    mut e: serenity::CreateEmbed,
    image: &str,
    thumbnail: &str,
    author: &str,
    author_icon: &str,
    footer: &str,
    footer_icon: &str,
) -> serenity::CreateEmbed {
    if !image.is_empty() {
        e = e.image(image);
    }
    if !thumbnail.is_empty() {
        e = e.thumbnail(thumbnail);
    }
    if !author.is_empty() {
        let mut a = serenity::CreateEmbedAuthor::new(author);
        if !author_icon.is_empty() {
            a = a.icon_url(author_icon);
        }
        e = e.author(a);
    }
    if !footer.is_empty() {
        let mut f = serenity::CreateEmbedFooter::new(footer);
        if !footer_icon.is_empty() {
            f = f.icon_url(footer_icon);
        }
        e = e.footer(f);
    }
    e
}

pub fn parse_color(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

/// `<:name:id>` / `<a:name:id>` custom emoji, or a plain unicode emoji.
pub fn parse_emoji(s: &str) -> Option<serenity::ReactionType> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(inner) = s.strip_prefix('<').and_then(|x| x.strip_suffix('>')) {
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() == 3 {
            let id = parts[2].parse::<u64>().ok()?;
            return Some(serenity::ReactionType::Custom {
                animated: parts[0] == "a",
                id: serenity::EmojiId::new(id),
                name: Some(parts[1].to_string()),
            });
        }
        return None;
    }
    if s.chars().count() > 8 || s.chars().any(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(serenity::ReactionType::Unicode(s.to_string()))
}
