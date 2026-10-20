//! Long-term ticket archive on Kreatix Storage (B2): lossless compression + dedupe by content hash.
use anyhow::{ensure, Context as _, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub struct Store {
    http: reqwest::Client,
    url: String,
    token: String,
    bucket: String,
}

/// How a stored blob is encoded. `webp` = lossless WebP of a PNG/BMP/TIFF (pixel-identical).
#[derive(Clone, Copy, PartialEq)]
pub enum Enc {
    Raw,
    Zstd,
    Webp,
}

impl Enc {
    pub fn as_str(self) -> &'static str {
        match self {
            Enc::Raw => "raw",
            Enc::Zstd => "zst",
            Enc::Webp => "webp",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "zst" => Enc::Zstd,
            "webp" => Enc::Webp,
            _ => Enc::Raw,
        }
    }
}

pub const MAX_FILE: u64 = 50 * 1024 * 1024;

impl Store {
    pub fn from_env() -> Option<Self> {
        Some(Self {
            http: reqwest::Client::builder().timeout(Duration::from_secs(120)).build().ok()?,
            url: std::env::var("KST_URL").ok()?.trim_end_matches('/').to_string(),
            token: std::env::var("KST_TOKEN").ok()?,
            bucket: std::env::var("KST_BUCKET").ok()?,
        })
    }

    fn object_url(&self, key: &str, suffix: &str) -> String {
        format!("{}/api/buckets/{}/{suffix}/{}", self.url, self.bucket, urlenc(key))
    }

    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        let r = self
            .http
            .put(self.object_url(key, "objects"))
            .bearer_auth(&self.token)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        let status = r.status();
        ensure!(status.is_success(), "storage upload {status}: {}", r.text().await.unwrap_or_default().chars().take(200).collect::<String>());
        Ok(())
    }

    /// Signed download link, 1-168 hours.
    pub async fn link(&self, key: &str, hours: u32) -> Result<String> {
        let v: Value = self
            .http
            .get(format!("{}?hours={hours}", self.object_url(key, "link")))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["url"].as_str().context("no download link")?.to_string())
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let link: Value = self
            .http
            .get(format!("{}?hours=1", self.object_url(key, "link")))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let url = link["url"].as_str().context("no download link")?;
        let r = self.http.get(url).send().await?.error_for_status()?;
        Ok(r.bytes().await?.to_vec())
    }
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

pub fn sha256(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn is_texty(name: &str, mime: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("javascript")
        || [
            "txt", "log", "json", "csv", "tsv", "md", "xml", "html", "htm", "svg", "yml", "yaml", "toml", "ini", "cfg", "conf", "js", "ts", "py",
            "rs", "java", "lua", "sk", "properties", "sql", "sh", "bat", "ps1", "c", "cpp", "h", "cs", "go", "kt", "php", "rb", "css",
        ]
        .contains(&ext.as_str())
}

/// Lossless only: PNG/BMP/TIFF (8-bit) to lossless WebP, text-like files to zstd -19, everything else
/// (JPEG, video, zips, already-compressed stuff) stays byte-for-byte. Keeps whichever is smaller.
pub fn compress(bytes: &[u8], name: &str, mime: &str) -> (Vec<u8>, Enc) {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let lossless_image = ["png", "bmp", "tif", "tiff"].contains(&ext.as_str()) || ["image/png", "image/bmp", "image/tiff"].contains(&mime);
    if lossless_image {
        if let Some(webp) = to_lossless_webp(bytes) {
            if webp.len() < bytes.len() * 95 / 100 {
                return (webp, Enc::Webp);
            }
        }
    }
    if is_texty(name, mime) || !lossless_image && bytes.len() < 8 * 1024 * 1024 {
        if let Ok(z) = zstd::bulk::compress(bytes, 19) {
            if z.len() < bytes.len() * 90 / 100 {
                return (z, Enc::Zstd);
            }
        }
    }
    (bytes.to_vec(), Enc::Raw)
}

fn to_lossless_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::{DynamicImage, ImageEncoder};
    let img = image::load_from_memory(bytes).ok()?;
    // only 8-bit images map 1:1 onto WebP; 16-bit/float would lose precision
    let (raw, w, h, color) = match img {
        DynamicImage::ImageRgba8(i) => {
            let (w, h) = i.dimensions();
            (i.into_raw(), w, h, image::ExtendedColorType::Rgba8)
        }
        DynamicImage::ImageRgb8(i) => {
            let (w, h) = i.dimensions();
            (i.into_raw(), w, h, image::ExtendedColorType::Rgb8)
        }
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageLumaA8(_) => {
            let i = img.to_rgba8();
            let (w, h) = i.dimensions();
            (i.into_raw(), w, h, image::ExtendedColorType::Rgba8)
        }
        _ => return None,
    };
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).write_image(&raw, w, h, color).ok()?;
    Some(out)
}

pub fn decompress(bytes: Vec<u8>, enc: Enc) -> Result<Vec<u8>> {
    Ok(match enc {
        Enc::Zstd => zstd::stream::decode_all(&bytes[..])?,
        _ => bytes,
    })
}

/// Stored object key for a blob.
pub fn file_key(hash: &str, enc: Enc) -> String {
    format!("files/{}/{hash}.{}", &hash[..2], enc.as_str())
}

pub fn ticket_key(guild: i64, id: i64) -> String {
    format!("tickets/{guild}/{id}.json.zst")
}

pub fn pack_ticket(v: &Value) -> Result<Vec<u8>> {
    Ok(zstd::bulk::compress(serde_json::to_vec(v)?.as_slice(), 19)?)
}

pub fn ticket_doc(t: &crate::db::Ticket, messages: Vec<Value>) -> Value {
    json!({
        "version": 1,
        "ticket": {
            "id": t.id, "guild_id": t.guild_id.to_string(), "channel_id": t.channel_id.to_string(), "user_id": t.user_id.to_string(),
            "subject": t.subject, "priority": t.priority, "claimed_by": t.claimed_by.to_string(), "closed_by": t.closed_by.to_string(),
            "escalated": t.escalated, "rating": t.rating, "rating_comment": t.rating_comment,
            "created_at": t.created_at, "closed_at": t.closed_at,
        },
        "messages": messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_roundtrip() {
        for path in ["/home/ubuntu/shots/desk-stats.png", "/home/ubuntu/shots/mob-agent.png"] {
            let Ok(orig) = std::fs::read(path) else { continue };
            let (packed, enc) = compress(&orig, "shot.png", "image/png");
            let a = image::load_from_memory(&orig).unwrap().to_rgba8();
            let b = image::load_from_memory(&decompress(packed.clone(), enc).unwrap()).unwrap().to_rgba8();
            assert_eq!(a.as_raw(), b.as_raw(), "pixels changed");
            println!("{path}: {} -> {} bytes ({})", orig.len(), packed.len(), enc.as_str());
        }
        let log = "[12:00:01] INFO player joined the game\n".repeat(2000).into_bytes();
        let (packed, enc) = compress(&log, "latest.log", "text/plain");
        assert_eq!(decompress(packed.clone(), enc).unwrap(), log);
        println!("log: {} -> {} bytes ({})", log.len(), packed.len(), enc.as_str());
    }
}


// ---------- knowledge ----------

pub const MAX_DOC: usize = 5 * 1024 * 1024;

/// Plain text out of an uploaded document.
pub fn extract_text(bytes: &[u8], name: &str) -> Result<String> {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let text = match ext.as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(bytes).map_err(|e| anyhow::anyhow!("couldn't read the PDF: {e}"))?,
        "html" | "htm" => crate::agent::html_to_text(&String::from_utf8_lossy(bytes)),
        _ => {
            ensure!(!bytes.iter().take(4096).any(|b| *b == 0), "only text, markdown, JSON, CSV, HTML or PDF files");
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    let text: String = text.chars().take(400_000).collect();
    ensure!(!text.trim().is_empty(), "no text found in that file");
    Ok(text)
}

static KCACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<String>>>> =
    std::sync::LazyLock::new(Default::default);

pub fn cache_doc(key: &str, text: String) {
    KCACHE.lock().unwrap().insert(key.to_string(), std::sync::Arc::new(text));
}

/// Loads the server's documents (memory cache first, then storage).
pub async fn load_knowledge(store: Option<&Store>, docs: &[crate::agent::Doc]) -> crate::agent::Knowledge {
    let mut out = vec![];
    for d in docs {
        let cached = KCACHE.lock().unwrap().get(&d.key).cloned();
        let text = match cached {
            Some(t) => t,
            None => {
                let Some(store) = store else { continue };
                match store.get(&d.key).await.and_then(|b| decompress(b, Enc::Zstd)) {
                    Ok(b) => {
                        let t = std::sync::Arc::new(String::from_utf8_lossy(&b).into_owned());
                        KCACHE.lock().unwrap().insert(d.key.clone(), t.clone());
                        t
                    }
                    Err(e) => {
                        tracing::warn!("knowledge {} failed: {e:#}", d.name);
                        continue;
                    }
                }
            }
        };
        out.push((d.name.clone(), text));
    }
    out
}

/// Keyword search over ~900-char chunks; returns the best few with their file names.
pub fn search(docs: &crate::agent::Knowledge, query: &str, k: usize) -> String {
    let terms: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_string).collect();
    if terms.is_empty() {
        return "Give a few keywords to search for.".into();
    }
    let mut hits: Vec<(f64, &str, String)> = vec![];
    for (name, text) in docs {
        let paras: Vec<&str> = text.split("\n\n").collect();
        let mut chunk = String::new();
        let mut chunks = vec![];
        for p in paras {
            if chunk.len() + p.len() > 900 && !chunk.is_empty() {
                chunks.push(std::mem::take(&mut chunk));
            }
            chunk.push_str(p);
            chunk.push_str("\n\n");
        }
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        for c in chunks {
            let lc = c.to_lowercase();
            let mut score = 0.0;
            for t in &terms {
                let n = lc.matches(t.as_str()).count();
                if n > 0 {
                    score += 1.0 + (n as f64).ln();
                }
            }
            if score > 0.0 {
                hits.push((score, name.as_str(), c.chars().take(1200).collect()));
            }
        }
    }
    if hits.is_empty() {
        return "Nothing in the documents matches that.".into();
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.into_iter().take(k).map(|(_, n, c)| format!("From {n}:\n{}", c.trim())).collect::<Vec<_>>().join("\n\n---\n\n")
}

// ---------- HTML transcripts ----------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Self-contained dark transcript page. `files` is the URL prefix for archived attachments.
pub fn render_html(t: &crate::db::Ticket, guild: &str, msgs: &[Value], files: &str) -> String {
    let mut body = String::new();
    for m in msgs {
        if m["kind"].as_i64() == Some(2) {
            continue;
        }
        let author = m["author"].as_str().unwrap_or("?");
        let id = m["author_id"].as_str().unwrap_or("0");
        let hue = id.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
        let at = crate::agent::utc(m["at"].as_i64().unwrap_or(0).max(0) as u64);
        let mut html = String::new();
        let content = m["content"].as_str().unwrap_or("");
        let mut rest = content;
        while let Some(i) = rest.find("[attachment: ") {
            html.push_str(&esc(&rest[..i]));
            let after = &rest[i + 13..];
            let Some(end) = after.find(']') else { html.push_str(&esc(&rest[i..])); rest = ""; break };
            let inner = &after[..end];
            let (name, target) = inner.rsplit_once(' ').unwrap_or((inner, ""));
            let url = match target.strip_prefix("kst:") { Some(h) => format!("{files}/{h}"), None => target.to_string() };
            let is_img = ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&name.rsplit('.').next().unwrap_or("").to_lowercase().as_str());
            if is_img {
                html.push_str(&format!("<a href=\"{0}\"><img src=\"{0}\" alt=\"{1}\" loading=\"lazy\"></a>", esc(&url), esc(name)));
            } else {
                html.push_str(&format!("<a class=\"file\" href=\"{}\">{}</a>", esc(&url), esc(name)));
            }
            rest = &after[end + 1..];
        }
        html.push_str(&esc(rest));
        body.push_str(&format!(
            "<div class=\"m\"><div class=\"av\" style=\"background:hsl({hue} 45% 32%)\">{}</div><div><div class=\"h\"><b>{}</b><time>{at} UTC</time></div><div class=\"c\">{}</div></div></div>\n",
            esc(&author.chars().next().unwrap_or('?').to_uppercase().to_string()),
            esc(author),
            html
        ));
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{title}</title><style>\
:root{{color-scheme:dark}}body{{margin:0;background:#000;color:#e8e8e8;font:15px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}}\
main{{max-width:860px;margin:0 auto;padding:32px 20px 64px}}header{{border-bottom:1px solid #222;padding-bottom:18px;margin-bottom:12px}}\
h1{{font-size:24px;font-weight:600;margin:0 0 6px}}.meta{{color:#8a8a8a;font-size:13px}}.m{{display:flex;gap:14px;padding:10px 0}}\
.av{{width:38px;height:38px;border-radius:50%;flex-shrink:0;display:grid;place-items:center;font-weight:600}}.h{{display:flex;gap:10px;align-items:baseline}}\
time{{color:#777;font-size:12px}}.c{{white-space:pre-wrap;overflow-wrap:anywhere}}.c img{{display:block;max-width:min(100%,460px);max-height:360px;border-radius:6px;margin:6px 0}}\
a{{color:#90caf9}}.file{{display:inline-block;padding:2px 8px;border-radius:4px;background:#1a1a1a;text-decoration:none}}</style></head><body><main>\
<header><h1>{title}</h1><div class=\"meta\">{guild} · ticket #{id} · opened {opened} UTC{closed}</div></header>{body}</main></body></html>",
        title = esc(&t.subject),
        guild = esc(guild),
        id = t.id,
        opened = crate::agent::utc(t.created_at.max(0) as u64),
        closed = if t.closed_at > 0 { format!(" · closed {} UTC", crate::agent::utc(t.closed_at as u64)) } else { String::new() },
    )
}

pub fn transcript_key(guild: i64, id: i64) -> String {
    format!("transcripts/{guild}/{id}.html.zst")
}

pub fn backup_key(guild: u64, at: i64) -> String {
    format!("backups/{guild}/{at}.json.zst")
}

pub fn random_token() -> String {
    use std::io::Read;
    let mut b = [0u8; 18];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b.iter().map(|x| format!("{x:02x}")).collect()
}
