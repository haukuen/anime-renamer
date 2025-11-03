use std::path::PathBuf;
use walkdir::WalkDir;

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "flv", "rmvb", "mov"];

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
                if entry.file_type().is_file() {
                    if let Some(ext) = entry.path().extension() {
                        if VIDEO_EXTENSIONS.contains(&ext.to_str().unwrap_or("")) {
                            video_files.push(entry.path().to_path_buf());
                        }
                    }
                }
            }
        } else {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext) = path.extension() {
                            if VIDEO_EXTENSIONS.contains(&ext.to_str().unwrap_or("")) {
                                video_files.push(path);
                            }
                        }
                    }
                }
            }
        }

        video_files.sort();

        video_files
    }
}
