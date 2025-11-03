use anyhow::{Context, Result};
use serde::Deserialize;

const API_KEY: &str = "454dec4903d35bb318ab2ad9e578c615";
const BASE_URL: &str = "https://api.themoviedb.org/3";

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
}

impl TmdbClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn search_tv(&self, query: &str, language: &str) -> Result<Vec<TvShow>> {
        let url = format!("{}/search/tv", BASE_URL);

        let response = self
            .client
            .get(&url)
            .query(&[
                ("api_key", API_KEY),
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
        let url = format!("{}/tv/{}", BASE_URL, tv_id);

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", API_KEY), ("language", language)])
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
        let url = format!("{}/tv/{}/season/{}", BASE_URL, tv_id, season_number);

        let response = self
            .client
            .get(&url)
            .query(&[("api_key", API_KEY), ("language", language)])
            .send()
            .await
            .context("Failed to send season details request")?;

        let season: SeasonDetails = response
            .json()
            .await
            .context("Failed to parse season details response")?;

        Ok(season)
    }
}
