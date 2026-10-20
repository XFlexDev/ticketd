//! Storage cleanup: images and documents that no config uses anymore get deleted from the bucket.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::{archive::Store, db::Kdb};

/// Grace period before an upload that was never saved anywhere counts as unused.
const ORPHAN_AGE_MS: i64 = 24 * 3600 * 1000;
const EXPORT_AGE_MS: i64 = 7 * 24 * 3600 * 1000;

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Media hashes (`m:<hash>`) and knowledge keys referenced in a config blob.
pub fn refs(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for part in s.split("/media/").skip(1) {
        if let Some(h) = part.get(..64).filter(|h| is_hex64(h)) {
            out.insert(format!("m:{h}"));
        }
    }
    for part in s.split("knowledge/").skip(1) {
        let Some((guild, rest)) = part.split_once('/') else { continue };
        if guild.is_empty() || !guild.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Some(h) = rest.get(..64).filter(|h| is_hex64(h) && rest[64..].starts_with(".txt.zst")) {
            out.insert(format!("knowledge/{guild}/{h}.txt.zst"));
        }
    }
    out
}

pub async fn guild_refs(db: &Kdb, guild_id: u64) -> anyhow::Result<BTreeSet<String>> {
    Ok(refs(&db.guild_configs(guild_id).await?.join("\n")))
}

/// Deletes whatever `before` referenced that this guild's config no longer does, unless another config still uses it.
pub async fn release(db: &Kdb, store: &Store, guild_id: u64, before: BTreeSet<String>) {
    let after = match guild_refs(db, guild_id).await {
        Ok(a) => a,
        Err(e) => return tracing::warn!("gc refs for {guild_id}: {e:#}"),
    };
    for r in before.difference(&after) {
        let needle = r.strip_prefix("m:").unwrap_or(r);
        match db.configs_mention(needle).await {
            Ok(false) => {
                if let Err(e) = drop_ref(db, store, r).await {
                    tracing::warn!("gc delete {r}: {e:#}");
                }
            }
            Ok(true) => {}
            Err(e) => tracing::warn!("gc check {r}: {e:#}"),
        }
    }
}

async fn drop_ref(db: &Kdb, store: &Store, r: &str) -> anyhow::Result<()> {
    if let Some(hash) = r.strip_prefix("m:") {
        let Some((key, _, _)) = db.file(hash).await? else { return Ok(()) };
        if !key.starts_with("media/") {
            return Ok(());
        }
        store.delete(&key).await?;
        db.delete_file(hash).await?;
        crate::web::forget_media(hash);
        purge_cdn(&key).await;
        tracing::info!("gc: deleted {key}");
    } else {
        store.delete(r).await?;
        crate::archive::forget_doc(r);
        tracing::info!("gc: deleted {r}");
    }
    Ok(())
}

/// Drops the copy Cloudflare cached for a public media URL.
async fn purge_cdn(key: &str) {
    let (Ok(token), Ok(zone), Ok(base)) = (std::env::var("CF_PURGE_TOKEN"), std::env::var("CF_ZONE_ID"), std::env::var("DASHBOARD_BASE_URL")) else {
        return;
    };
    let url = format!("{}/{key}", base.trim_end_matches('/'));
    let r = reqwest::Client::new()
        .post(format!("https://api.cloudflare.com/client/v4/zones/{zone}/purge_cache"))
        .bearer_auth(token)
        .json(&serde_json::json!({"files": [url]}))
        .send()
        .await;
    if let Err(e) = r.and_then(|r| r.error_for_status()) {
        tracing::warn!("cdn purge {key}: {e}");
    }
}

/// Full pass over the bucket: unused uploads, pruned backups and expired exports.
pub async fn sweep(db: &Kdb, store: &Store) -> anyhow::Result<usize> {
    let now = crate::db::now() * 1000;
    let live = refs(&db.all_configs().await?.join("\n"));
    let mut n = 0;
    for (name, at) in store.list("media/").await? {
        let Some(hash) = name.strip_prefix("media/").and_then(|f| f.split_once('.')).map(|(h, _)| h).filter(|h| is_hex64(h)) else { continue };
        if now - at > ORPHAN_AGE_MS && !live.contains(&format!("m:{hash}")) && drop_ref(db, store, &format!("m:{hash}")).await.is_ok() {
            n += 1;
        }
    }
    for (name, at) in store.list("knowledge/").await? {
        if now - at > ORPHAN_AGE_MS && !refs(&name).is_empty() && !live.contains(&name) && drop_ref(db, store, &name).await.is_ok() {
            n += 1;
        }
    }
    let kept: BTreeSet<String> = db.all_backup_keys().await?.into_iter().collect();
    for (name, at) in store.list("backups/").await? {
        if now - at > 3600 * 1000 && !kept.contains(&name) && store.delete(&name).await.is_ok() {
            n += 1;
        }
    }
    for (name, at) in store.list("exports/").await? {
        if now - at > EXPORT_AGE_MS && store.delete(&name).await.is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

pub async fn sweeper(db: Kdb, store: std::sync::Arc<Store>) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        match sweep(&db, &store).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("gc sweep removed {n} objects"),
            Err(e) => tracing::warn!("gc sweep: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_refs() {
        let h = "a".repeat(64);
        let s = format!(r#"{{"image":"https://kev.kreatix.dev/media/{h}.webp","k":[{{"key":"knowledge/123/{h}.txt.zst"}}],"x":"/media/short.png"}}"#);
        let r = refs(&s);
        assert!(r.contains(&format!("m:{h}")));
        assert!(r.contains(&format!("knowledge/123/{h}.txt.zst")));
        assert_eq!(r.len(), 2);
    }
}
