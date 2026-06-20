//! Thin async wrapper around the Supabase PostgREST REST API.
//! All operations target the `PlayerInformation` table.

use anyhow::Context;
use reqwest::header;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SupabaseClient {
    http: reqwest::Client,
    base_url: String,
    service_key: String,
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PlayerRow {
    pub id: i64,
    pub player_name: String,
    pub password: String,
    pub log_out_position_x: f32,
    pub log_out_position_y: f32,
}

#[derive(Serialize)]
struct CreatePlayerBody<'a> {
    player_name: &'a str,
    password: &'a str,
    log_out_position_x: f32,
    log_out_position_y: f32,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl SupabaseClient {
    pub fn new(project_url: &str, service_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            // e.g. https://xxxx.supabase.co/rest/v1
            base_url: format!("{}/rest/v1", project_url.trim_end_matches('/')),
            service_key: service_key.into(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut map = header::HeaderMap::new();
        let key = header::HeaderValue::from_str(&self.service_key)
            .expect("SUPABASE_SERVICE_KEY contains characters not valid in HTTP headers");
        map.insert("apikey", key.clone());
        map.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", self.service_key))
                .expect("SUPABASE_SERVICE_KEY contains characters not valid in HTTP headers"),
        );
        map
    }

    /// Look up a player by name. Returns `None` if no matching row exists.
    pub async fn find_player(&self, name: &str) -> anyhow::Result<Option<PlayerRow>> {
        let rows: Vec<PlayerRow> = self
            .http
            .get(format!("{}/PlayerInformation", self.base_url))
            .headers(self.auth_headers())
            .query(&[
                ("player_name", format!("eq.{name}")),
                ("select", "id,player_name,password,log_out_position_x,log_out_position_y".to_string()),
            ])
            .send()
            .await
            .context("Supabase find_player: request failed")?
            .error_for_status()
            .context("Supabase find_player: non-2xx response")?
            .json()
            .await
            .context("Supabase find_player: JSON parse error")?;

        Ok(rows.into_iter().next())
    }

    /// Insert a new player. Returns the row as stored (with the DB-assigned `id`).
    pub async fn create_player(&self, name: &str, password: &str) -> anyhow::Result<PlayerRow> {
        let mut rows: Vec<PlayerRow> = self
            .http
            .post(format!("{}/PlayerInformation", self.base_url))
            .headers(self.auth_headers())
            // Ask PostgREST to return the inserted row so we can read the generated id.
            .header("Prefer", "return=representation")
            .json(&CreatePlayerBody {
                player_name: name,
                password,
                log_out_position_x: 0.0, // Default logout position X
                log_out_position_y: 0.0, // Default logout position Y
            })
            .send()
            .await
            .context("Supabase create_player: request failed")?
            .error_for_status()
            .context("Supabase create_player: non-2xx response")?
            .json()
            .await
            .context("Supabase create_player: JSON parse error")?;

        rows.pop()
            .context("Supabase create_player: response was empty")
    }
}
