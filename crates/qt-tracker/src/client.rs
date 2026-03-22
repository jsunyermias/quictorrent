use reqwest::Client;
use tracing::debug;

use crate::{
    error::Result,
    types::{AnnounceEvent, AnnounceRequest, AnnounceResponse, ScrapeResponse},
};

/// Cliente HTTP para un tracker quictorrent.
pub struct TrackerClient {
    http:     Client,
    base_url: String,
    /// Token de autenticación opcional.
    auth_token: Option<String>,
}

impl TrackerClient {
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Result<Self> {
        let http = Client::builder()
            .use_rustls_tls()
            .build()?;
        Ok(Self { http, base_url: base_url.into(), auth_token })
    }

    fn request_builder(&self, url: &str) -> reqwest::RequestBuilder {
        let mut rb = self.http.post(url);
        if let Some(token) = &self.auth_token {
            rb = rb.header("X-Auth-Token", token);
        }
        rb
    }

    /// Envía un announce al tracker.
    pub async fn announce(
        &self,
        info_hash: &[u8; 32],
        peer_id:   &[u8; 32],
        addr:      &str,
        event:     AnnounceEvent,
        uploaded:  u64,
        downloaded: u64,
        left:      u64,
        num_want:  u32,
    ) -> Result<AnnounceResponse> {
        let req = AnnounceRequest {
            info_hash:  hex::encode(info_hash),
            peer_id:    hex::encode(peer_id),
            addr:       addr.to_string(),
            event,
            uploaded,
            downloaded,
            left,
            num_want,
        };

        let url = format!("{}/announce", self.base_url);
        debug!("announcing to {}", url);

        let resp = self.request_builder(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<AnnounceResponse>()
            .await?;

        Ok(resp)
    }

    /// Obtiene estadísticas de un torrent desde el tracker.
    pub async fn scrape(&self, info_hash: &[u8; 32]) -> Result<ScrapeResponse> {
        let url = format!("{}/scrape?info_hash={}", self.base_url, hex::encode(info_hash));
        debug!("scraping {}", url);

        let mut rb = self.http.get(&url);
        if let Some(token) = &self.auth_token {
            rb = rb.header("X-Auth-Token", token);
        }

        let resp = rb.send().await?
            .error_for_status()?
            .json::<ScrapeResponse>()
            .await?;

        Ok(resp)
    }
}
