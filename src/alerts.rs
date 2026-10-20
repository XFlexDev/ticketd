//! Social alerts: YouTube uploads, subreddit posts and any RSS or Atom feed, posted to a channel.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::Data;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Feed {
    pub id: String,
    /// "youtube", "reddit" or "rss"
    pub kind: String,
    pub source: String,
    pub channel_id: String,
    pub role_id: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct AlertsConfig {
    pub feeds: Vec<Feed>,
}

impl AlertsConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.feeds.len() > 25 {
            return Err("25 alerts max".into());
        }
        let mut ids = std::collections::HashSet::new();
        for (i, f) in self.feeds.iter_mut().enumerate() {
            if !matches!(f.kind.as_str(), "youtube" | "reddit" | "rss") {
                return Err("Unknown alert type".into());
            }
            f.source = f.source.trim().chars().take(300).collect();
            if f.source.is_empty() {
                return Err("Every alert needs a channel, subreddit or feed link".into());
            }
            if f.kind == "rss" && !f.source.starts_with("https://") && !f.source.starts_with("http://") {
                return Err(format!("\"{}\" isn't a feed link", f.source));
            }
            if f.channel_id.parse::<u64>().is_err() {
                return Err("Pick a Discord channel for every alert".into());
            }
            if f.role_id.parse::<u64>().is_err() {
                f.role_id.clear();
            }
            f.message = f.message.trim().chars().take(500).collect();
            if f.id.is_empty() || !ids.insert(f.id.clone()) {
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
                f.id = format!("{:x}", (n >> 12) ^ (i as u64 * 7919));
                ids.insert(f.id.clone());
            }
        }
        Ok(())
    }
}

pub fn default_message(kind: &str) -> &'static str {
    match kind {
        "youtube" => "**{author}** posted a new video: {title}\n{link}",
        "reddit" => "New post in **{feed}**: {title}\n{link}",
        _ => "**{feed}**: {title}\n{link}",
    }
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder().user_agent("Mozilla/5.0 (compatible; KevBot/1.0; +https://kev.kreatix.dev)").timeout(Duration::from_secs(15)).build().unwrap_or_default()
});
static YT_IDS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

async fn youtube_id(source: &str) -> Option<String> {
    let s = source.trim().trim_end_matches('/');
    let re = regex::Regex::new(r"(UC[\w-]{22})").ok()?;
    if let Some(m) = re.captures(s).filter(|_| !s.contains('@')) {
        return Some(m[1].to_string());
    }
    if let Some(id) = YT_IDS.lock().unwrap().get(s) {
        return Some(id.clone());
    }
    let url = if s.starts_with("http") { s.to_string() } else { format!("https://www.youtube.com/{}", if s.starts_with('@') { s.to_string() } else { format!("@{s}") }) };
    let html = HTTP.get(&url).header("accept-language", "en").send().await.ok()?.text().await.ok()?;
    let canon = regex::Regex::new(r#"<link rel="canonical" href="https://www\.youtube\.com/channel/(UC[\w-]{22})""#).ok()?;
    let any = regex::Regex::new(r#""(?:externalId|channelId)":"(UC[\w-]{22})""#).ok()?;
    let id = canon.captures(&html).or_else(|| any.captures(&html)).map(|c| c[1].to_string())?;
    YT_IDS.lock().unwrap().insert(s.to_string(), id.clone());
    Some(id)
}

async fn feed_url(f: &Feed) -> Option<String> {
    match f.kind.as_str() {
        "youtube" => youtube_id(&f.source).await.map(|id| format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}")),
        "reddit" => {
            let sub = f.source.trim().trim_start_matches("https://").trim_start_matches("www.reddit.com/").trim_start_matches("reddit.com/").trim_start_matches("r/").trim_start_matches('/').split('/').next()?.to_string();
            (!sub.is_empty()).then(|| format!("https://www.reddit.com/r/{sub}/new/.rss"))
        }
        _ => Some(f.source.clone()),
    }
}

pub struct Item {
    pub id: String,
    pub title: String,
    pub link: String,
    pub author: String,
}

pub async fn fetch(url: &str, guarded: bool) -> Result<(String, Vec<Item>), String> {
    let req = if guarded {
        let (u, client) = crate::agent::guarded_client(url).await.map_err(|e| e.to_string())?;
        client.get(u).header("user-agent", "Mozilla/5.0 (compatible; KevBot/1.0; +https://kev.kreatix.dev)")
    } else {
        HTTP.get(url)
    };
    let r = req.send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("The feed answered {}", r.status()));
    }
    let bytes = r.bytes().await.map_err(|e| e.to_string())?;
    let feed = feed_rs::parser::parse(&bytes[..]).map_err(|_| "That link isn't an RSS or Atom feed".to_string())?;
    let name = feed.title.map(|t| t.content).unwrap_or_default();
    let items = feed
        .entries
        .into_iter()
        .take(30)
        .map(|e| Item {
            id: e.id,
            title: e.title.map(|t| t.content).unwrap_or_default(),
            link: e.links.first().map(|l| l.href.clone()).unwrap_or_default(),
            author: e.authors.first().map(|a| a.name.clone()).unwrap_or_else(|| name.clone()),
        })
        .collect();
    Ok((name, items))
}

/// Checks a feed from the dashboard: (feed name, newest titles).
pub async fn test(f: &Feed) -> Result<(String, Vec<String>), String> {
    let url = feed_url(f).await.ok_or_else(|| "Couldn't find that YouTube channel".to_string())?;
    let (name, items) = fetch(&url, f.kind == "rss").await?;
    Ok((name, items.into_iter().take(3).map(|i| i.title).collect()))
}

async fn run_once(ctx: &serenity::Context, data: &Data) {
    let configs = data.db.module_configs("alerts").await.unwrap_or_default();
    let mut cache: HashMap<String, Option<(String, Vec<Item>)>> = HashMap::new();
    for (guild, raw) in configs {
        let Ok(cfg) = serde_json::from_str::<AlertsConfig>(&raw) else { continue };
        for f in cfg.feeds {
            let Some(url) = feed_url(&f).await else { continue };
            if !cache.contains_key(&url) {
                let got = fetch(&url, f.kind == "rss").await.map_err(|e| tracing::debug!("alerts {url}: {e}")).ok();
                cache.insert(url.clone(), got);
            }
            let Some(Some((name, items))) = cache.get(&url) else { continue };
            let key = format!("{}:{}", f.id, url);
            let seen = data.db.alert_seen(guild, &key).await.ok().flatten();
            let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
            if let Some(seen) = &seen {
                let fresh: Vec<&Item> = items.iter().filter(|i| !seen.contains(&i.id)).take(3).collect();
                for item in fresh.into_iter().rev() {
                    post(ctx, &f, name, item).await;
                }
            }
            let mut keep = ids;
            if let Some(old) = seen {
                let rest: Vec<String> = old.into_iter().filter(|o| !keep.contains(o)).collect();
                keep.extend(rest);
            }
            keep.truncate(60);
            let _ = data.db.save_alert_seen(guild, &key, &keep).await;
        }
    }
}

async fn post(ctx: &serenity::Context, f: &Feed, feed: &str, item: &Item) {
    let Ok(ch) = f.channel_id.parse::<u64>() else { return };
    let tpl = if f.message.is_empty() { default_message(&f.kind) } else { &f.message };
    let mut text = tpl.replace("{title}", &item.title).replace("{link}", &item.link).replace("{author}", &item.author).replace("{feed}", feed);
    let mut mentions = serenity::CreateAllowedMentions::new();
    if let Ok(role) = f.role_id.parse::<u64>() {
        text = format!("<@&{role}> {text}");
        mentions = mentions.roles(vec![serenity::RoleId::new(role)]);
    }
    let _ = serenity::ChannelId::new(ch).send_message(&ctx.http, serenity::CreateMessage::new().content(text.chars().take(2000).collect::<String>()).allowed_mentions(mentions)).await;
}

pub async fn poller(ctx: serenity::Context, data: Data) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        run_once(&ctx, &data).await;
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}
