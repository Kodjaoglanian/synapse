//! HTTP signaling client.
//!
//! Exchanges SDP offers/answers and trickle ICE candidates with a remote peer
//! through a simple REST endpoint. The signaling server itself is not part of
//! synapse — any HTTP server that stores a body under a key and returns it on
//! GET works (a 10-line `python3 -m http.server` won't, but a tiny static
//! store will; see AGENTS.md for a reference snippet).
//!
//! Protocol (all bodies are raw JSON strings):
//!
//! ```text
//! POST {base}/offer/{room}    -> stores the offer body
//! GET  {base}/offer/{room}    -> returns the stored offer (200) or 404
//! POST {base}/answer/{room}   -> stores the answer body
//! GET  {base}/answer/{room}   -> returns the stored answer (200) or 404
//! POST {base}/ice/{room}/{side} -> stores an ICE candidate (append-only)
//! GET  {base}/ice/{room}/{side} -> returns newline-joined candidates
//! ```
//!
//! `{side}` is `a` or `b` so each peer reads the other's candidates without
//! consuming its own.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// A signaling client bound to a base URL.
#[derive(Clone)]
pub struct Signaling {
    base: String,
    client: reqwest::Client,
}

/// One ICE candidate exchanged via signaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
    pub username_fragment: Option<String>,
}

impl Signaling {
    /// Build a client. `base` should not have a trailing slash.
    pub fn new(base: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("build reqwest client")?;
        Ok(Self { base, client })
    }

    /// Publish our offer for a room. Idempotent on the server side.
    pub async fn post_offer(&self, room: &str, sdp: &str) -> Result<()> {
        let url = format!("{}/offer/{}", self.base, room);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(sdp.to_string())
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("POST {url} -> {}", resp.status()));
        }
        Ok(())
    }

    /// Fetch the remote peer's offer for a room. Returns `None` if not posted yet.
    pub async fn get_offer(&self, room: &str) -> Result<Option<String>> {
        let url = format!("{}/offer/{}", self.base, room);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("GET {url} -> {}", resp.status()));
        }
        Ok(Some(
            resp.text()
                .await
                .with_context(|| format!("read body {url}"))?,
        ))
    }

    /// Publish our answer for a room.
    pub async fn post_answer(&self, room: &str, sdp: &str) -> Result<()> {
        let url = format!("{}/answer/{}", self.base, room);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(sdp.to_string())
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("POST {url} -> {}", resp.status()));
        }
        Ok(())
    }

    /// Fetch the remote peer's answer for a room. Returns `None` if not posted yet.
    pub async fn get_answer(&self, room: &str) -> Result<Option<String>> {
        let url = format!("{}/answer/{}", self.base, room);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("GET {url} -> {}", resp.status()));
        }
        Ok(Some(
            resp.text()
                .await
                .with_context(|| format!("read body {url}"))?,
        ))
    }

    /// Post one of our ICE candidates. The server is expected to append.
    pub async fn post_ice(&self, room: &str, side: char, cand: &IceCandidate) -> Result<()> {
        let url = format!("{}/ice/{}/{}", self.base, room, side);
        let body = serde_json::to_string(cand)?;
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("POST {url} -> {}", resp.status()));
        }
        Ok(())
    }

    /// Fetch the remote peer's ICE candidates (newline-delimited JSON).
    pub async fn get_ice(&self, room: &str, side: char) -> Result<Vec<IceCandidate>> {
        let url = format!("{}/ice/{}/{}", self.base, room, side);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !resp.status().is_success() {
            return Err(anyhow!("GET {url} -> {}", resp.status()));
        }
        let text = resp
            .text()
            .await
            .with_context(|| format!("read body {url}"))?;
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<IceCandidate>(line) {
                Ok(c) => out.push(c),
                Err(_) => continue, // server may emit partial/extra lines; skip
            }
        }
        Ok(out)
    }
}
