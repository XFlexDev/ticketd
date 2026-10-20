use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::sync::Mutex;

/// OpenAI-compatible chat client (NVIDIA NIM, StepFun, whatever) with
/// comma-separated key rotation and per-key cooldown on 429/5xx.
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    model: String,
    keys: Vec<String>,
    cursor: AtomicUsize,
    cooldowns: Mutex<Vec<Option<Instant>>>,
}

impl Client {
    pub fn from_env() -> Result<Self> {
        let keys: Vec<String> = std::env::var("LLM_API_KEY")?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        anyhow::ensure!(!keys.is_empty(), "LLM_API_KEY is empty");
        let n = keys.len();
        Ok(Self {
            http: reqwest::Client::new(),
            base_url: std::env::var("LLM_BASE_URL")
                .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".into()),
            model: std::env::var("LLM_MODEL")
                .unwrap_or_else(|_| "nvidia/nemotron-3-ultra-550b-a55b".into()),
            keys,
            cursor: AtomicUsize::new(0),
            cooldowns: Mutex::new(vec![None; n]),
        })
    }

    pub fn enabled(&self) -> bool {
        true
    }

    /// Minimal chat call; returns assistant text.
    pub async fn chat(&self, system: &str, history: &[(String, String)], user: &str) -> Result<String> {
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(serde_json::json!({"role": "system", "content": system}));
        for (role, content) in history {
            messages.push(serde_json::json!({"role": role, "content": content}));
        }
        messages.push(serde_json::json!({"role": "user", "content": user}));

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.4,
            "max_tokens": 800,
        });

        let mut last_err = anyhow::anyhow!("no keys");
        for _ in 0..self.keys.len() {
            let Some((idx, key)) = self.next_key() else { break };
            let resp = self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .bearer_auth(key)
                .json(&body)
                .timeout(Duration::from_secs(45))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let v: serde_json::Value = r.json().await?;
                    let text = v["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    return Ok(text.trim().to_string());
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    if status == 429 || status >= 500 {
                        self.cool(idx);
                    }
                    last_err = anyhow::anyhow!("LLM HTTP {status}");
                }
                Err(e) => last_err = anyhow::anyhow!("LLM request failed: {e}"),
            }
        }
        Err(last_err)
    }

    fn next_key(&self) -> Option<(usize, String)> {
        let n = self.keys.len();
        let start = self.cursor.fetch_add(1, Ordering::Relaxed) % n;
        let cds = self.cooldowns.lock().unwrap();
        for off in 0..n {
            let i = (start + off) % n;
            let ok = match cds[i] {
                Some(t) => t.elapsed() > Duration::from_secs(60),
                None => true,
            };
            if ok {
                return Some((i, self.keys[i].clone()));
            }
        }
        None
    }

    fn cool(&self, idx: usize) {
        self.cooldowns.lock().unwrap()[idx] = Some(Instant::now());
    }
}
