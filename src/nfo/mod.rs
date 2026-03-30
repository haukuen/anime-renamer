use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    pub provider: String,
    pub value: f64,
    pub votes: u32,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UniqueId {
    pub id_type: String,
    pub value: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TvShowNfo {
    pub title: String,
    pub plot: Option<String>,
    pub premiered: Option<String>,
    pub year: Option<u32>,
    pub status: Option<String>,
    pub rating: Option<Rating>,
    pub unique_ids: Vec<UniqueId>,
    pub tmdb_id: u32,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub episodeguide: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeNfo {
    pub title: String,
    pub showtitle: String,
    pub season: u32,
    pub episode: u32,
    pub plot: Option<String>,
    pub aired: Option<String>,
    pub rating: Option<Rating>,
    pub unique_ids: Vec<UniqueId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteAction {
    WouldWrite,
    Written,
    SkippedExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub path: PathBuf,
    pub action: WriteAction,
}

pub struct NfoWriter {
    dry_run: bool,
    force: bool,
}

impl NfoWriter {
    pub fn new(dry_run: bool, force: bool) -> Self {
        Self { dry_run, force }
    }

    pub fn write_tvshow(&self, root: &Path, nfo: &TvShowNfo) -> Result<WriteOutcome> {
        let path = root.join("tvshow.nfo");
        self.write_file(&path, &nfo.render())
    }

    pub fn write_episode(&self, video_path: &Path, nfo: &EpisodeNfo) -> Result<WriteOutcome> {
        let path = episode_nfo_path(video_path);
        self.write_file(&path, &nfo.render())
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<WriteOutcome> {
        if path.exists() && !self.force {
            return Ok(WriteOutcome {
                path: path.to_path_buf(),
                action: WriteAction::SkippedExisting,
            });
        }

        if self.dry_run {
            return Ok(WriteOutcome {
                path: path.to_path_buf(),
                action: WriteAction::WouldWrite,
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)?;

        Ok(WriteOutcome {
            path: path.to_path_buf(),
            action: WriteAction::Written,
        })
    }
}

impl TvShowNfo {
    pub fn render(&self) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push('\n');
        xml.push_str("<tvshow>\n");
        push_tag(&mut xml, "title", Some(self.title.as_str()), 1);
        push_tag(&mut xml, "plot", self.plot.as_deref(), 1);
        push_tag(&mut xml, "premiered", self.premiered.as_deref(), 1);

        if let Some(year) = self.year {
            push_tag(&mut xml, "year", Some(&year.to_string()), 1);
        }

        push_tag(&mut xml, "status", self.status.as_deref(), 1);
        push_ratings(&mut xml, self.rating.as_ref(), 1);
        push_unique_ids(&mut xml, &self.unique_ids, 1);
        push_tag(&mut xml, "tmdbid", Some(&self.tmdb_id.to_string()), 1);

        for genre in &self.genres {
            push_tag(&mut xml, "genre", Some(genre.as_str()), 1);
        }

        for studio in &self.studios {
            push_tag(&mut xml, "studio", Some(studio.as_str()), 1);
        }

        push_tag(
            &mut xml,
            "episodeguide",
            Some(self.episodeguide.as_str()),
            1,
        );
        xml.push_str("</tvshow>\n");
        xml
    }
}

impl EpisodeNfo {
    pub fn render(&self) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push('\n');
        xml.push_str("<episodedetails>\n");
        push_tag(&mut xml, "title", Some(self.title.as_str()), 1);
        push_tag(&mut xml, "showtitle", Some(self.showtitle.as_str()), 1);
        push_tag(&mut xml, "season", Some(&self.season.to_string()), 1);
        push_tag(&mut xml, "episode", Some(&self.episode.to_string()), 1);
        push_tag(&mut xml, "plot", self.plot.as_deref(), 1);
        push_tag(&mut xml, "aired", self.aired.as_deref(), 1);
        push_ratings(&mut xml, self.rating.as_ref(), 1);
        push_unique_ids(&mut xml, &self.unique_ids, 1);
        xml.push_str("</episodedetails>\n");
        xml
    }
}

pub fn episode_nfo_path(video_path: &Path) -> PathBuf {
    video_path.with_extension("nfo")
}

fn push_tag(xml: &mut String, tag: &str, value: Option<&str>, indent_level: usize) {
    let indent = "    ".repeat(indent_level);
    xml.push_str(&indent);
    xml.push('<');
    xml.push_str(tag);
    xml.push('>');
    xml.push_str(&escape_xml(value.unwrap_or("")));
    xml.push_str("</");
    xml.push_str(tag);
    xml.push_str(">\n");
}

fn push_ratings(xml: &mut String, rating: Option<&Rating>, indent_level: usize) {
    let Some(rating) = rating else {
        return;
    };

    let indent = "    ".repeat(indent_level);
    let child_indent = "    ".repeat(indent_level + 1);
    xml.push_str(&indent);
    xml.push_str("<ratings>\n");
    xml.push_str(&child_indent);
    xml.push_str(&format!(
        r#"<rating name="{}" max="10" default="{}">"#,
        escape_xml(&rating.provider),
        if rating.is_default { "true" } else { "false" }
    ));
    xml.push('\n');
    push_tag(
        xml,
        "value",
        Some(&format!("{:.6}", rating.value)),
        indent_level + 2,
    );
    push_tag(
        xml,
        "votes",
        Some(&rating.votes.to_string()),
        indent_level + 2,
    );
    xml.push_str(&child_indent);
    xml.push_str("</rating>\n");
    xml.push_str(&indent);
    xml.push_str("</ratings>\n");
}

fn push_unique_ids(xml: &mut String, unique_ids: &[UniqueId], indent_level: usize) {
    for unique_id in unique_ids {
        let indent = "    ".repeat(indent_level);
        xml.push_str(&indent);
        xml.push_str(&format!(
            r#"<uniqueid type="{}" default="{}">{}"#,
            escape_xml(&unique_id.id_type),
            if unique_id.is_default {
                "true"
            } else {
                "false"
            },
            escape_xml(&unique_id.value)
        ));
        xml.push_str("</uniqueid>\n");
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn sample_tvshow_nfo() -> TvShowNfo {
        TvShowNfo {
            title: "A & B <Show>".to_string(),
            plot: Some("Plot > story".to_string()),
            premiered: Some("2024-01-01".to_string()),
            year: Some(2024),
            status: Some("Ended".to_string()),
            rating: Some(Rating {
                provider: "themoviedb".to_string(),
                value: 8.75,
                votes: 128,
                is_default: true,
            }),
            unique_ids: vec![UniqueId {
                id_type: "tmdb".to_string(),
                value: "123".to_string(),
                is_default: true,
            }],
            tmdb_id: 123,
            genres: vec!["Animation".to_string()],
            studios: vec!["Studio".to_string()],
            episodeguide: r#"{"tmdb":"123"}"#.to_string(),
        }
    }

    fn sample_episode_nfo() -> EpisodeNfo {
        EpisodeNfo {
            title: "Episode & 1".to_string(),
            showtitle: "Series".to_string(),
            season: 1,
            episode: 2,
            plot: Some("Plot <episode>".to_string()),
            aired: Some("2024-02-03".to_string()),
            rating: Some(Rating {
                provider: "themoviedb".to_string(),
                value: 7.5,
                votes: 42,
                is_default: true,
            }),
            unique_ids: vec![UniqueId {
                id_type: "tmdb".to_string(),
                value: "456".to_string(),
                is_default: true,
            }],
        }
    }

    #[test]
    fn test_tvshow_render_contains_core_tags() {
        let xml = sample_tvshow_nfo().render();

        assert!(xml.contains("<tvshow>"));
        assert!(xml.contains("<title>A &amp; B &lt;Show&gt;</title>"));
        assert!(xml.contains("<plot>Plot &gt; story</plot>"));
        assert!(xml.contains(r#"<uniqueid type="tmdb" default="true">123</uniqueid>"#));
        assert!(xml.contains("<tmdbid>123</tmdbid>"));
        assert!(xml.contains("<episodeguide>{&quot;tmdb&quot;:&quot;123&quot;}</episodeguide>"));
    }

    #[test]
    fn test_episode_render_contains_core_tags() {
        let xml = sample_episode_nfo().render();

        assert!(xml.contains("<episodedetails>"));
        assert!(xml.contains("<showtitle>Series</showtitle>"));
        assert!(xml.contains("<season>1</season>"));
        assert!(xml.contains("<episode>2</episode>"));
        assert!(xml.contains("<aired>2024-02-03</aired>"));
        assert!(xml.contains(r#"<uniqueid type="tmdb" default="true">456</uniqueid>"#));
    }

    #[test]
    fn test_render_omits_optional_sections_when_absent() {
        let xml = TvShowNfo {
            rating: None,
            genres: Vec::new(),
            studios: Vec::new(),
            plot: None,
            premiered: None,
            year: None,
            status: None,
            ..sample_tvshow_nfo()
        }
        .render();

        assert!(!xml.contains("<ratings>"));
        assert!(xml.contains("<plot></plot>"));
    }

    #[test]
    fn test_episode_nfo_path_reuses_video_stem() {
        let path = episode_nfo_path(Path::new("/media/Season 1/Show S01E01.mkv"));

        assert_eq!(path, Path::new("/media/Season 1/Show S01E01.nfo"));
    }

    #[test]
    fn test_writer_skips_existing_files_without_force() {
        let dir = TestDir::new("nfo_skip");
        let path = dir.path().join("tvshow.nfo");
        fs::write(&path, "existing").unwrap();

        let writer = NfoWriter::new(false, false);
        let outcome = writer
            .write_tvshow(dir.path(), &sample_tvshow_nfo())
            .unwrap();

        assert_eq!(outcome.action, WriteAction::SkippedExisting);
        assert_eq!(fs::read_to_string(path).unwrap(), "existing");
    }

    #[test]
    fn test_writer_force_overwrites_existing_files() {
        let dir = TestDir::new("nfo_force");
        let video_path = dir.path().join("Show S01E01.mkv");
        let nfo_path = episode_nfo_path(&video_path);
        fs::write(&nfo_path, "existing").unwrap();

        let writer = NfoWriter::new(false, true);
        let outcome = writer
            .write_episode(&video_path, &sample_episode_nfo())
            .unwrap();

        assert_eq!(outcome.action, WriteAction::Written);
        let content = fs::read_to_string(nfo_path).unwrap();
        assert!(content.contains("<episodedetails>"));
    }

    #[test]
    fn test_writer_dry_run_does_not_write_files() {
        let dir = TestDir::new("nfo_dry_run");
        let video_path = dir.path().join("Show S01E01.mkv");

        let writer = NfoWriter::new(true, false);
        let outcome = writer
            .write_episode(&video_path, &sample_episode_nfo())
            .unwrap();

        assert_eq!(outcome.action, WriteAction::WouldWrite);
        assert!(!episode_nfo_path(&video_path).exists());
    }
}
