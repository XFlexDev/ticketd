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
