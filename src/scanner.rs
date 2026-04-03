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

        let video_stem = match file_stem_lossy(video_path) {
            Some(s) => s,
            None => return subtitles,
        };

        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let filename = match file_name_lossy(&path) {
                    Some(s) => s,
                    None => continue,
                };

                if !filename.starts_with(&video_stem) {
                    continue;
                }

                let Some(suffix) = filename.strip_prefix(&video_stem) else {
                    continue;
                };

                let is_subtitle = SUBTITLE_EXTENSIONS
                    .iter()
                    .any(|ext| suffix.ends_with(&format!(".{ext}")));

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
        let subtitle_filename = file_name_lossy(subtitle_path)?;
        let new_video_stem = file_stem_lossy(new_video_path)?;
        let new_parent = new_video_path.parent()?;

        let suffix = subtitle_filename.strip_prefix(old_video_stem)?;
        let new_subtitle_filename = format!("{new_video_stem}{suffix}");

        Some(new_parent.join(new_subtitle_filename))
    }
}

fn file_name_lossy(path: &Path) -> Option<String> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
}

fn file_stem_lossy(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|value| value.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

    #[test]
    fn test_scan_non_recursive_only_returns_top_level_videos() {
        let dir = TestDir::new("scanner_non_recursive");
        let nested_dir = dir.path().join("nested");
        fs::create_dir_all(&nested_dir).unwrap();

        let top_level_video = dir.path().join("episode01.mkv");
        let nested_video = nested_dir.join("episode02.mp4");
        let ignored_file = dir.path().join("readme.txt");

        fs::write(&top_level_video, b"video").unwrap();
        fs::write(&nested_video, b"video").unwrap();
        fs::write(&ignored_file, b"text").unwrap();

        let scanner = FileScanner::new(false);
        let files = scanner.scan(dir.path().to_str().unwrap());

        assert_eq!(files, vec![top_level_video]);
    }

    #[test]
    fn test_scan_recursive_includes_nested_videos() {
        let dir = TestDir::new("scanner_recursive");
        let nested_dir = dir.path().join("season1");
        fs::create_dir_all(&nested_dir).unwrap();

        let top_level_video = dir.path().join("episode01.mkv");
        let nested_video = nested_dir.join("episode02.mp4");

        fs::write(&top_level_video, b"video").unwrap();
        fs::write(&nested_video, b"video").unwrap();

        let scanner = FileScanner::new(true);
        let files = scanner.scan(dir.path().to_str().unwrap());

        assert_eq!(files, vec![top_level_video, nested_video]);
    }

    #[test]
    fn test_find_associated_subtitles_only_returns_matching_supported_files() {
        let dir = TestDir::new("scanner_subtitles");
        let video = dir.path().join("My Show 01.mkv");

        fs::write(&video, b"video").unwrap();
        fs::write(dir.path().join("My Show 01.ass"), b"subtitle").unwrap();
        fs::write(dir.path().join("My Show 01.zh-Hans.srt"), b"subtitle").unwrap();
        fs::write(dir.path().join("My Show 01.txt"), b"text").unwrap();
        fs::write(dir.path().join("My Show 02.ass"), b"subtitle").unwrap();

        let subtitles = FileScanner::find_associated_subtitles(&video);

        assert_eq!(
            subtitles,
            vec![
                dir.path().join("My Show 01.ass"),
                dir.path().join("My Show 01.zh-Hans.srt")
            ]
        );
    }

    #[test]
    fn test_compute_subtitle_new_path_preserves_suffix() {
        let subtitle = Path::new("/tmp/Old Name.zh-Hans.ass");
        let new_video = Path::new("/media/Season 1/New Name.mkv");

        let new_path =
            FileScanner::compute_subtitle_new_path(subtitle, "Old Name", new_video).unwrap();

        assert_eq!(new_path, Path::new("/media/Season 1/New Name.zh-Hans.ass"));
    }
}
