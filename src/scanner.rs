use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "flv", "rmvb", "mov"];
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "srt", "ssa", "sub", "idx", "vtt"];

pub struct FileScanner {
    recursive: bool,
}

impl FileScanner {
    pub fn new(recursive: bool) -> Self {
        Self { recursive }
    }

    pub fn scan(&self, path: &str) -> Vec<PathBuf> {
        let mut video_files = Vec::new();

        if self.recursive {
            for entry in WalkDir::new(path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file()
                    && let Some(ext) = entry.path().extension()
                    && VIDEO_EXTENSIONS.contains(&ext.to_str().unwrap_or(""))
                {
                    video_files.push(entry.path().to_path_buf());
                }
            }
        } else if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_file()
                    && let Some(ext) = path.extension()
                    && VIDEO_EXTENSIONS.contains(&ext.to_str().unwrap_or(""))
                {
                    video_files.push(path);
                }
            }
        }

        video_files.sort();

        video_files
    }

    pub fn find_associated_subtitles(video_path: &Path) -> Vec<PathBuf> {
        let mut subtitles = Vec::new();

        let parent = match video_path.parent() {
            Some(p) => p,
            None => return subtitles,
        };

        let video_stem = match video_path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => return subtitles,
        };

        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let filename = match path.file_name().and_then(|s| s.to_str()) {
                    Some(s) => s,
                    None => continue,
                };

                if !filename.starts_with(video_stem) {
                    continue;
                }

                let suffix = &filename[video_stem.len()..];

                let is_subtitle = SUBTITLE_EXTENSIONS
                    .iter()
                    .any(|ext| suffix.ends_with(&format!(".{}", ext)));

                if is_subtitle {
                    subtitles.push(path);
                }
            }
        }

        subtitles.sort();
        subtitles
    }

    pub fn compute_subtitle_new_path(
        subtitle_path: &Path,
        old_video_stem: &str,
        new_video_path: &Path,
    ) -> Option<PathBuf> {
        let subtitle_filename = subtitle_path.file_name()?.to_str()?;
        let new_video_stem = new_video_path.file_stem()?.to_str()?;
        let new_parent = new_video_path.parent()?;

        let suffix = &subtitle_filename[old_video_stem.len()..];
        let new_subtitle_filename = format!("{}{}", new_video_stem, suffix);

        Some(new_parent.join(new_subtitle_filename))
    }
}
