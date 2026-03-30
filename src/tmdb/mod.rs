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
    pub id: u32,
    pub name: String,
    #[allow(dead_code)]
    pub original_name: String,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub first_air_date: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: u32,
    pub number_of_seasons: u32,
    #[serde(default)]
    pub seasons: Vec<Season>,
    #[serde(default)]
    pub genres: Vec<NamedValue>,
    #[serde(default)]
    pub networks: Vec<NamedValue>,
    #[serde(default)]
    pub production_companies: Vec<NamedValue>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Season {
    pub season_number: u32,
    pub episode_count: u32,
    #[allow(dead_code)]
    pub name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SeasonDetails {
    pub season_number: u32,
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Episode {
    pub id: u32,
    pub episode_number: u32,
    pub name: String,
    #[serde(default)]
    pub air_date: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NamedValue {
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

    #[test]
    fn test_tv_details_deserializes_optional_nfo_fields() {
        let json = serde_json::json!({
            "id": 123,
            "name": "Test Show",
            "original_name": "Test Show Original",
            "overview": "Overview",
            "first_air_date": "2024-01-01",
            "status": "Ended",
            "vote_average": 8.1,
            "vote_count": 10,
            "number_of_seasons": 2,
            "seasons": [{"season_number": 1, "episode_count": 12, "name": "Season 1"}],
            "genres": [{"name": "Animation"}],
            "networks": [{"name": "Tokyo MX"}],
            "production_companies": [{"name": "Studio"}]
        });

        let details: TvDetails = serde_json::from_value(json).unwrap();

        assert_eq!(details.id, 123);
        assert_eq!(details.overview.as_deref(), Some("Overview"));
        assert_eq!(details.first_air_date.as_deref(), Some("2024-01-01"));
        assert_eq!(details.status.as_deref(), Some("Ended"));
        assert_eq!(details.vote_count, 10);
        assert_eq!(details.genres[0].name, "Animation");
        assert_eq!(details.networks[0].name, "Tokyo MX");
    }

    #[test]
    fn test_season_details_deserializes_optional_episode_fields() {
        let json = serde_json::json!({
            "season_number": 1,
            "episodes": [
                {
                    "id": 987,
                    "episode_number": 2,
                    "name": "Episode 2",
                    "air_date": "2024-01-08",
                    "overview": "Episode overview",
                    "vote_average": 7.9,
                    "vote_count": 32
                }
            ]
        });

        let details: SeasonDetails = serde_json::from_value(json).unwrap();

        assert_eq!(details.season_number, 1);
        assert_eq!(details.episodes[0].id, 987);
        assert_eq!(details.episodes[0].air_date.as_deref(), Some("2024-01-08"));
        assert_eq!(
            details.episodes[0].overview.as_deref(),
            Some("Episode overview")
        );
        assert_eq!(details.episodes[0].vote_count, 32);
    }

    #[test]
    fn test_tv_details_defaults_missing_optional_fields() {
        let json = serde_json::json!({
            "id": 1,
            "name": "Fallback",
            "original_name": "Fallback",
            "number_of_seasons": 0,
            "seasons": []
        });

        let details: TvDetails = serde_json::from_value(json).unwrap();

        assert_eq!(details.overview, None);
        assert_eq!(details.first_air_date, None);
        assert_eq!(details.status, None);
        assert_eq!(details.vote_average, 0.0);
        assert_eq!(details.vote_count, 0);
        assert!(details.genres.is_empty());
        assert!(details.networks.is_empty());
        assert!(details.production_companies.is_empty());
    }
}
