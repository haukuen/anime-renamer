use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const API_URL: &str = "https://graphql.anilist.co";

#[derive(Debug, Serialize)]
struct GraphQLRequest {
    query: String,
    variables: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<Data>,
}

#[derive(Debug, Deserialize)]
struct Data {
    #[serde(rename = "Page")]
    page: Option<Page>,
    #[serde(rename = "Media")]
    #[allow(dead_code)]
    media: Option<Media>,
}

#[derive(Debug, Deserialize)]
struct Page {
    media: Vec<Media>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Media {
    pub id: u32,
    pub title: Title,
    #[serde(rename = "startDate")]
    pub start_date: Option<FuzzyDate>,
    pub format: Option<String>,
    pub episodes: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Title {
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FuzzyDate {
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

pub struct AniListClient {
    client: reqwest::Client,
}

impl AniListClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn search_anime(&self, query: &str) -> Result<Vec<Media>> {
        let graphql_query = r#"
            query ($search: String) {
                Page(page: 1, perPage: 10) {
                    media(search: $search, type: ANIME, sort: POPULARITY_DESC) {
                        id
                        title {
                            romaji
                            english
                            native
                        }
                        startDate {
                            year
                            month
                            day
                        }
                        format
                        episodes
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "search": query
        });

        let request = GraphQLRequest {
            query: graphql_query.to_string(),
            variables,
        };

        let response = self
            .client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send AniList request")?;

        let graphql_response: GraphQLResponse = response
            .json()
            .await
            .context("Failed to parse AniList response")?;

        Ok(graphql_response
            .data
            .and_then(|d| d.page)
            .map(|p| p.media)
            .unwrap_or_default())
    }

    #[allow(dead_code)]
    pub async fn get_anime_by_id(&self, id: u32) -> Result<Option<Media>> {
        let graphql_query = r#"
            query ($id: Int) {
                Media(id: $id, type: ANIME) {
                    id
                    title {
                        romaji
                        english
                        native
                    }
                    startDate {
                        year
                        month
                        day
                    }
                    format
                    episodes
                }
            }
        "#;

        let variables = serde_json::json!({
            "id": id
        });

        let request = GraphQLRequest {
            query: graphql_query.to_string(),
            variables,
        };

        let response = self
            .client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send AniList request")?;

        let graphql_response: GraphQLResponse = response
            .json()
            .await
            .context("Failed to parse AniList response")?;

        Ok(graphql_response.data.and_then(|d| d.media))
    }
}

impl Media {
    #[allow(dead_code)]
    pub fn get_display_title(&self, prefer_english: bool) -> String {
        if prefer_english {
            if let Some(ref english) = self.title.english {
                return english.clone();
            }
        }

        if let Some(ref native) = self.title.native {
            return native.clone();
        }

        if let Some(ref romaji) = self.title.romaji {
            return romaji.clone();
        }

        if let Some(ref english) = self.title.english {
            return english.clone();
        }

        "Unknown".to_string()
    }

    pub fn format_date(&self) -> String {
        if let Some(ref date) = self.start_date {
            if let (Some(year), Some(month), Some(day)) = (date.year, date.month, date.day) {
                return format!("{:04}-{:02}-{:02}", year, month, day);
            } else if let (Some(year), Some(month)) = (date.year, date.month) {
                return format!("{:04}-{:02}", year, month);
            } else if let Some(year) = date.year {
                return format!("{}", year);
            }
        }
        "未知".to_string()
    }
}
