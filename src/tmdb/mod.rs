use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;

const DEFAULT_API_KEY: &str = "454dec4903d35bb318ab2ad9e578c615";
const DEFAULT_BASE_URL: &str = "https://api.themoviedb.org";
const API_VERSION_PATH: &str = "/3";

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub results: Vec<TvShow>,
}

#[derive(Debug, Deserialize)]
pub struct TvShow {
    pub id: u32,
    pub name: String,
    #[allow(dead_code)]
    pub original_name: String,
    pub first_air_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TvDetails {
    #[allow(dead_code)]
    pub id: u32,
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub original_name: String,
    pub number_of_seasons: u32,
    pub seasons: Vec<Season>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Season {
    pub season_number: u32,
    pub episode_count: u32,
    #[allow(dead_code)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SeasonDetails {
    pub season_number: u32,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Episode {
    pub episode_number: u32,
    pub name: String,
}

pub struct TmdbClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl TmdbClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: resolve_api_key(),
            base_url: resolve_base_url(),
        }
    }

    pub async fn search_tv(&self, query: &str, language: &str) -> Result<Vec<TvShow>> {
        let url = self.build_url("/search/tv");

        let response = self
            .client
            .get(&url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("query", query),
                ("language", language),
            ])
            .send()
            .await
            .context("Failed to send search request")?;

        let search_result: SearchResult = response
            .json()
            .await
            .context("Failed to parse search response")?;

        Ok(search_result.results)
    }

    pub async fn get_tv_details(&self, tv_id: u32, language: &str) -> Result<TvDetails> {
        let url = self.build_url(&format!("/tv/{}", tv_id));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str()), ("language", language)])
            .send()
            .await
            .context("Failed to send tv details request")?;

        let details: TvDetails = response
            .json()
            .await
            .context("Failed to parse tv details response")?;

        Ok(details)
    }

    #[allow(dead_code)]
    pub async fn get_season_details(
        &self,
        tv_id: u32,
        season_number: u32,
        language: &str,
    ) -> Result<SeasonDetails> {
        let url = self.build_url(&format!("/tv/{}/season/{}", tv_id, season_number));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str()), ("language", language)])
            .send()
            .await
            .context("Failed to send season details request")?;

        let season: SeasonDetails = response
            .json()
            .await
            .context("Failed to parse season details response")?;

        Ok(season)
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}{}{}", self.base_url, API_VERSION_PATH, path)
    }
}

fn resolve_api_key() -> String {
    resolve_api_key_from_env(env::var("TMDB_API_KEY").ok())
}

fn resolve_base_url() -> String {
    resolve_base_url_from_env(env::var("TMDB_BASE_URL").ok())
}

fn resolve_api_key_from_env(value: Option<String>) -> String {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_KEY.to_string())
}

fn resolve_base_url_from_env(value: Option<String>) -> String {
    let normalized = value
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    normalized
        .strip_suffix(API_VERSION_PATH)
        .unwrap_or(&normalized)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_api_key_falls_back_to_default() {
        assert_eq!(resolve_api_key_from_env(None), DEFAULT_API_KEY);
    }

    #[test]
    fn test_resolve_api_key_prefers_env_value() {
        assert_eq!(
            resolve_api_key_from_env(Some(" custom-key ".to_string())),
            "custom-key"
        );
    }

    #[test]
    fn test_resolve_base_url_falls_back_to_default() {
        assert_eq!(resolve_base_url_from_env(None), DEFAULT_BASE_URL);
    }

    #[test]
    fn test_resolve_base_url_trims_trailing_slash() {
        assert_eq!(
            resolve_base_url_from_env(Some(" https://example.com/api/ ".to_string())),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_resolve_base_url_strips_version_suffix() {
        assert_eq!(
            resolve_base_url_from_env(Some("https://example.com/api/3".to_string())),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_build_url_appends_fixed_api_version() {
        let client = TmdbClient {
            client: reqwest::Client::new(),
            api_key: "key".to_string(),
            base_url: "https://example.com/tmdb".to_string(),
        };

        assert_eq!(
            client.build_url("/search/tv"),
            "https://example.com/tmdb/3/search/tv"
        );
    }

    #[test]
    fn test_build_url_remains_correct_with_legacy_base_url_value() {
        let client = TmdbClient {
            client: reqwest::Client::new(),
            api_key: "key".to_string(),
            base_url: resolve_base_url_from_env(Some("https://example.com/tmdb/3".to_string())),
        };

        assert_eq!(
            client.build_url("/tv/42"),
            "https://example.com/tmdb/3/tv/42"
        );
    }
}
