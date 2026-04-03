use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const API_URL: &str = "https://graphql.anilist.co";
const HTTP_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Serialize)]
struct GraphQLRequest {
    query: String,
    variables: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<GraphQLError>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
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
            client: build_http_client(),
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
            .context("AniList 请求发送失败")?
            .error_for_status()
            .context("AniList 请求返回错误状态")?;

        let graphql_response = response.json().await.context("AniList 响应解析失败")?;

        let data = extract_graphql_data(graphql_response)?;

        Ok(data
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
            .context("AniList 请求发送失败")?
            .error_for_status()
            .context("AniList 请求返回错误状态")?;

        let graphql_response = response.json().await.context("AniList 响应解析失败")?;

        Ok(extract_graphql_data(graphql_response)?.and_then(|d| d.media))
    }
}

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .build()
        .expect("创建 AniList HTTP 客户端失败")
}

fn extract_graphql_data(response: GraphQLResponse) -> Result<Option<Data>> {
    if !response.errors.is_empty() {
        let message = response
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("；");
        bail!("AniList GraphQL 错误: {message}");
    }

    Ok(response.data)
}

impl Media {
    #[allow(dead_code)]
    pub fn get_display_title(&self, prefer_english: bool) -> String {
        if prefer_english && let Some(ref english) = self.title.english {
            return english.clone();
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
                return format!("{year:04}-{month:02}-{day:02}");
            } else if let (Some(year), Some(month)) = (date.year, date.month) {
                return format!("{year:04}-{month:02}");
            } else if let Some(year) = date.year {
                return format!("{year}");
            }
        }
        "未知".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_media(title: Title, start_date: Option<FuzzyDate>) -> Media {
        Media {
            id: 1,
            title,
            start_date,
            format: Some("TV".to_string()),
            episodes: Some(12),
        }
    }

    #[test]
    fn test_get_display_title_prefers_english_when_requested() {
        let media = make_media(
            Title {
                romaji: Some("Bocchi the Rock!".to_string()),
                english: Some("Bocchi the Rock!".to_string()),
                native: Some("ぼっち・ざ・ろっく！".to_string()),
            },
            None,
        );

        assert_eq!(media.get_display_title(true), "Bocchi the Rock!");
    }

    #[test]
    fn test_get_display_title_falls_back_to_native_then_romaji() {
        let media = make_media(
            Title {
                romaji: Some("Sousou no Frieren".to_string()),
                english: None,
                native: Some("葬送のフリーレン".to_string()),
            },
            None,
        );
        let romaji_only = make_media(
            Title {
                romaji: Some("K-On!".to_string()),
                english: None,
                native: None,
            },
            None,
        );

        assert_eq!(media.get_display_title(false), "葬送のフリーレン");
        assert_eq!(romaji_only.get_display_title(false), "K-On!");
    }

    #[test]
    fn test_format_date_supports_full_partial_and_missing_dates() {
        let full = make_media(
            Title {
                romaji: None,
                english: None,
                native: None,
            },
            Some(FuzzyDate {
                year: Some(2024),
                month: Some(10),
                day: Some(5),
            }),
        );
        let partial = make_media(
            Title {
                romaji: None,
                english: None,
                native: None,
            },
            Some(FuzzyDate {
                year: Some(2024),
                month: Some(10),
                day: None,
            }),
        );
        let missing = make_media(
            Title {
                romaji: None,
                english: None,
                native: None,
            },
            None,
        );

        assert_eq!(full.format_date(), "2024-10-05");
        assert_eq!(partial.format_date(), "2024-10");
        assert_eq!(missing.format_date(), "未知");
    }

    #[test]
    fn test_extract_graphql_data_returns_error_when_response_contains_errors() {
        let response = GraphQLResponse {
            data: None,
            errors: vec![GraphQLError {
                message: "Too Many Requests.".to_string(),
            }],
        };

        let err = extract_graphql_data(response).unwrap_err();
        assert!(err.to_string().contains("AniList GraphQL 错误"));
        assert!(err.to_string().contains("Too Many Requests."));
    }
}
