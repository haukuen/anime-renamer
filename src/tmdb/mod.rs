use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DEFAULT_API_KEY: &str = "454dec4903d35bb318ab2ad9e578c615";
const DEFAULT_BASE_URL: &str = "https://api.themoviedb.org";
const API_VERSION_PATH: &str = "/3";
const HTTP_TIMEOUT_SECONDS: u64 = 30;
const MAX_CONCURRENT_REQUESTS: usize = 8;

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
    pub poster_path: Option<String>,
    #[serde(default)]
    pub backdrop_path: Option<String>,
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
    #[serde(default)]
    pub poster_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SeasonDetails {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub name: String,
    pub season_number: u32,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub air_date: Option<String>,
    #[serde(default)]
    pub poster_path: Option<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Episode {
    pub id: u32,
    pub episode_number: u32,
    pub name: String,
    #[serde(default)]
    pub still_path: Option<String>,
    #[serde(default)]
    pub air_date: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: u32,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct EpisodeExternalIds {
    #[serde(default)]
    pub imdb_id: Option<String>,
    #[serde(default)]
    pub tvdb_id: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct EpisodeCredits {
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub crew: Vec<CrewMember>,
    #[serde(default)]
    pub guest_stars: Vec<CastMember>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CastMember {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub character: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub profile_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrewMember {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub job: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub profile_path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NamedValue {
    pub name: String,
}

#[derive(Clone)]
pub struct TmdbClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    semaphore: Arc<Semaphore>,
}

impl TmdbClient {
    pub fn new() -> Self {
        Self {
            client: build_http_client(),
            api_key: resolve_api_key(),
            base_url: resolve_base_url(),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        }
    }

    pub async fn search_tv(&self, query: &str, language: &str) -> Result<Vec<TvShow>> {
        let _permit = self.acquire_permit().await;
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
            .context("TMDB 搜索请求发送失败")?
            .error_for_status()
            .context("TMDB 搜索请求返回错误状态")?;

        let search_result: SearchResult = response.json().await.context("TMDB 搜索响应解析失败")?;

        Ok(search_result.results)
    }

    pub async fn get_tv_details(&self, tv_id: u32, language: &str) -> Result<TvDetails> {
        let _permit = self.acquire_permit().await;
        let url = self.build_url(&format!("/tv/{}", tv_id));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str()), ("language", language)])
            .send()
            .await
            .context("TMDB 详情请求发送失败")?
            .error_for_status()
            .context("TMDB 详情请求返回错误状态")?;

        let details: TvDetails = response.json().await.context("TMDB 详情响应解析失败")?;

        Ok(details)
    }

    #[allow(dead_code)]
    pub async fn get_season_details(
        &self,
        tv_id: u32,
        season_number: u32,
        language: &str,
    ) -> Result<SeasonDetails> {
        let _permit = self.acquire_permit().await;
        let url = self.build_url(&format!("/tv/{}/season/{}", tv_id, season_number));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str()), ("language", language)])
            .send()
            .await
            .context("TMDB 季详情请求发送失败")?
            .error_for_status()
            .context("TMDB 季详情请求返回错误状态")?;

        let season: SeasonDetails = response.json().await.context("TMDB 季详情响应解析失败")?;

        Ok(season)
    }

    pub async fn download_image(&self, file_path: &str) -> Result<Vec<u8>> {
        let _permit = self.acquire_permit().await;
        let response = self
            .client
            .get(self.build_image_url(file_path))
            .send()
            .await
            .context("TMDB 图片下载请求发送失败")?
            .error_for_status()
            .context("TMDB 图片下载返回错误状态")?;

        let bytes = response.bytes().await.context("TMDB 图片响应读取失败")?;

        Ok(bytes.to_vec())
    }

    pub async fn get_episode_credits(
        &self,
        tv_id: u32,
        season_number: u32,
        episode_number: u32,
        language: &str,
    ) -> Result<EpisodeCredits> {
        let _permit = self.acquire_permit().await;
        let url = self.build_url(&format!(
            "/tv/{}/season/{}/episode/{}/credits",
            tv_id, season_number, episode_number
        ));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str()), ("language", language)])
            .send()
            .await
            .context("TMDB 单集演职员请求发送失败")?
            .error_for_status()
            .context("TMDB 单集演职员请求返回错误状态")?;

        let credits: EpisodeCredits = response
            .json()
            .await
            .context("TMDB 单集演职员响应解析失败")?;

        Ok(credits)
    }

    pub async fn get_episode_external_ids(
        &self,
        tv_id: u32,
        season_number: u32,
        episode_number: u32,
    ) -> Result<EpisodeExternalIds> {
        let _permit = self.acquire_permit().await;
        let url = self.build_url(&format!(
            "/tv/{}/season/{}/episode/{}/external_ids",
            tv_id, season_number, episode_number
        ));

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", self.api_key.as_str())])
            .send()
            .await
            .context("TMDB 单集外部 ID 请求发送失败")?
            .error_for_status()
            .context("TMDB 单集外部 ID 请求返回错误状态")?;

        let external_ids: EpisodeExternalIds = response
            .json()
            .await
            .context("TMDB 单集外部 ID 响应解析失败")?;

        Ok(external_ids)
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}{}{}", self.base_url, API_VERSION_PATH, path)
    }

    fn build_image_url(&self, file_path: &str) -> String {
        format!("https://image.tmdb.org/t/p/original{}", file_path)
    }

    async fn acquire_permit(&self) -> OwnedSemaphorePermit {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("TMDB 并发信号量已关闭")
    }
}

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .build()
        .expect("创建 TMDB HTTP 客户端失败")
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

pub fn image_extension(file_path: &str) -> &str {
    Path::new(file_path)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("jpg")
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
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
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
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
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
            "poster_path": "/poster.jpg",
            "backdrop_path": "/backdrop.jpg",
            "overview": "Overview",
            "first_air_date": "2024-01-01",
            "status": "Ended",
            "vote_average": 8.1,
            "vote_count": 10,
            "number_of_seasons": 2,
            "seasons": [{
                "season_number": 1,
                "episode_count": 12,
                "name": "Season 1",
                "poster_path": "/season-1.jpg"
            }],
            "genres": [{"name": "Animation"}],
            "networks": [{"name": "Tokyo MX"}],
            "production_companies": [{"name": "Studio"}]
        });

        let details: TvDetails = serde_json::from_value(json).unwrap();

        assert_eq!(details.id, 123);
        assert_eq!(details.poster_path.as_deref(), Some("/poster.jpg"));
        assert_eq!(details.backdrop_path.as_deref(), Some("/backdrop.jpg"));
        assert_eq!(details.overview.as_deref(), Some("Overview"));
        assert_eq!(details.first_air_date.as_deref(), Some("2024-01-01"));
        assert_eq!(details.status.as_deref(), Some("Ended"));
        assert_eq!(details.vote_count, 10);
        assert_eq!(details.genres[0].name, "Animation");
        assert_eq!(details.networks[0].name, "Tokyo MX");
        assert_eq!(
            details.seasons[0].poster_path.as_deref(),
            Some("/season-1.jpg")
        );
    }

    #[test]
    fn test_season_details_deserializes_optional_episode_fields() {
        let json = serde_json::json!({
            "id": 456,
            "name": "Season 1",
            "season_number": 1,
            "overview": "Season overview",
            "air_date": "2024-01-01",
            "poster_path": "/season-1.jpg",
            "episodes": [
                {
                    "id": 987,
                    "episode_number": 2,
                    "name": "Episode 2",
                    "still_path": "/episode-2.jpg",
                    "air_date": "2024-01-08",
                    "overview": "Episode overview",
                    "vote_average": 7.9,
                    "vote_count": 32
                }
            ]
        });

        let details: SeasonDetails = serde_json::from_value(json).unwrap();

        assert_eq!(details.id, 456);
        assert_eq!(details.name, "Season 1");
        assert_eq!(details.season_number, 1);
        assert_eq!(details.overview.as_deref(), Some("Season overview"));
        assert_eq!(details.air_date.as_deref(), Some("2024-01-01"));
        assert_eq!(details.poster_path.as_deref(), Some("/season-1.jpg"));
        assert_eq!(details.episodes[0].id, 987);
        assert_eq!(
            details.episodes[0].still_path.as_deref(),
            Some("/episode-2.jpg")
        );
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

    #[test]
    fn test_image_extension_uses_path_suffix() {
        assert_eq!(image_extension("/abc/poster.png"), "png");
        assert_eq!(image_extension("/abc/poster"), "jpg");
    }

    #[test]
    fn test_build_image_url_uses_tmdb_image_host() {
        let client = TmdbClient {
            client: reqwest::Client::new(),
            api_key: "key".to_string(),
            base_url: "https://example.com/tmdb".to_string(),
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        };

        assert_eq!(
            client.build_image_url("/poster.jpg"),
            "https://image.tmdb.org/t/p/original/poster.jpg"
        );
    }

    #[test]
    fn test_episode_credits_deserialize_cast_crew_and_guest_stars() {
        let json = serde_json::json!({
            "cast": [{
                "id": 1,
                "name": "Main Cast",
                "character": "Hero",
                "profile_path": "/cast.jpg"
            }],
            "crew": [{
                "id": 2,
                "name": "Director Name",
                "department": "Directing",
                "job": "Director",
                "profile_path": "/crew.jpg"
            }],
            "guest_stars": [{
                "id": 3,
                "name": "Guest Star",
                "character": "Guest",
                "profile_path": "/guest.jpg"
            }]
        });

        let credits: EpisodeCredits = serde_json::from_value(json).unwrap();

        assert_eq!(credits.cast[0].name, "Main Cast");
        assert_eq!(credits.crew[0].job.as_deref(), Some("Director"));
        assert_eq!(
            credits.guest_stars[0].profile_path.as_deref(),
            Some("/guest.jpg")
        );
    }

    #[test]
    fn test_episode_external_ids_deserialize_optional_values() {
        let json = serde_json::json!({
            "imdb_id": "tt1234567",
            "tvdb_id": 42
        });

        let external_ids: EpisodeExternalIds = serde_json::from_value(json).unwrap();

        assert_eq!(external_ids.imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(external_ids.tvdb_id, Some(42));
    }
}
