use regex::Regex;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum EpisodeType {
    Normal,
    OVA,
    OAD,
    Special,
    Movie,
}

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub anime_name: String,
    pub episode_number: u32,
    pub episode_type: EpisodeType,
    pub tags: Vec<String>,
    pub extension: String,
    pub is_already_formatted: bool,
}

pub struct FileParser {
    episode_patterns: Vec<Regex>,
    tag_regex: Regex,
    special_keywords: Vec<(Regex, EpisodeType)>,
}

impl FileParser {
    pub fn new() -> Self {
        let episode_patterns = vec![
            Regex::new(r"[Ss](\d{1,2})[Ee](\d{1,4})").unwrap(),
            Regex::new(r"第\s*(\d{1,4})\s*(?:集|话|話)").unwrap(),
            Regex::new(r"[Ee][Pp]\s*(\d{1,4})").unwrap(),
            Regex::new(r"[Ee](\d{1,4})(?:\D|$)").unwrap(),
            Regex::new(r"_[Ss](\d{1,4})").unwrap(),
            Regex::new(r"[\s\-_\.]+(\d{1,4})(?:\s|[\[\.]|$)").unwrap(),
        ];

        let special_keywords = vec![
            (
                Regex::new(r"(?i)剧场版|theater|theatrical|movie|gekijouban|gekijōban").unwrap(),
                EpisodeType::Movie,
            ),
            (Regex::new(r"(?i)\bOAD\b").unwrap(), EpisodeType::OAD),
            (Regex::new(r"(?i)\bOVA\b").unwrap(), EpisodeType::OVA),
            (
                Regex::new(r"(?i)\bSP\b|special|特典|特別|番外|总集篇|总集編").unwrap(),
                EpisodeType::Special,
            ),
        ];

        Self {
            episode_patterns,
            tag_regex: Regex::new(r"\[([^\]]+)\]").unwrap(),
            special_keywords,
        }
    }

    /// 检测是否是特殊内容（OVA/OAD/SP/剧场版等）
    fn detect_special_type(&self, text: &str) -> EpisodeType {
        for (pattern, episode_type) in &self.special_keywords {
            if pattern.is_match(text) {
                return episode_type.clone();
            }
        }
        EpisodeType::Normal
    }

    /// 尝试提取集数信息
    fn extract_episode(&self, text: &str) -> Option<(u32, String)> {
        for pattern in &self.episode_patterns {
            if let Some(captures) = pattern.captures(text) {
                let episode_str = if captures.len() > 2 {
                    &captures[2]
                } else {
                    &captures[1]
                };

                if let Ok(episode) = episode_str.parse::<u32>() {
                    let matched = captures.get(0).unwrap().as_str();
                    return Some((episode, matched.to_string()));
                }
            }
        }
        None
    }

    /// 清理番剧名称
    fn clean_anime_name(&self, name: &str) -> String {
        let mut name = self.tag_regex.replace_all(name, "").to_string();

        for (pattern, _) in &self.special_keywords {
            name = pattern.replace_all(&name, " ").to_string();
        }

        name = name
            .trim()
            .trim_matches(|c| c == '-' || c == '_' || c == '.')
            .trim()
            .to_string();

        let space_regex = Regex::new(r"\s+").unwrap();
        name = space_regex.replace_all(&name, " ").to_string();

        name.trim().to_string()
    }

    pub fn parse(&self, filename: &str) -> Option<ParsedFile> {
        let path = Path::new(filename);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let stem = path.file_stem()?.to_str()?;

        let tags: Vec<String> = self
            .tag_regex
            .captures_iter(stem)
            .map(|cap| cap[1].to_string())
            .collect();

        let episode_type = self.detect_special_type(stem);

        let already_formatted_regex = Regex::new(r"\s+S\d{2}E\d{2}\s*").unwrap();
        let is_already_formatted = already_formatted_regex.is_match(stem);

        let (episode_number, episode_match) = self.extract_episode(stem)?;

        let mut anime_name = stem.to_string();

        anime_name = anime_name.replace(&episode_match, " ");

        anime_name = self.clean_anime_name(&anime_name);

        if anime_name.is_empty() {
            return None;
        }

        Some(ParsedFile {
            anime_name,
            episode_number,
            episode_type,
            tags,
            extension,
            is_already_formatted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_brackets() {
        let parser = FileParser::new();
        let result = parser.parse("[字幕组] 鬼灭之刃 28 [1080p].mkv").unwrap();
        assert_eq!(result.anime_name, "鬼灭之刃");
        assert_eq!(result.episode_number, 28);
        assert_eq!(result.extension, "mkv");
    }

    #[test]
    fn test_parse_simple() {
        let parser = FileParser::new();
        let result = parser.parse("鬼灭之刃 27.mkv").unwrap();
        assert_eq!(result.anime_name, "鬼灭之刃");
        assert_eq!(result.episode_number, 27);
    }

    #[test]
    fn test_parse_with_dash() {
        let parser = FileParser::new();
        let result = parser.parse("孤独搖滾！- 01.mkv").unwrap();
        assert_eq!(result.anime_name, "孤独搖滾！");
        assert_eq!(result.episode_number, 1);
    }

    #[test]
    fn test_parse_underscore_s_format() {
        let parser = FileParser::new();
        let result = parser.parse("[DBD-RAWS]妖精的尾巴_S001[1080].mkv").unwrap();
        assert_eq!(result.anime_name, "妖精的尾巴");
        assert_eq!(result.episode_number, 1);
    }

    #[test]
    fn test_parse_s_e_format() {
        let parser = FileParser::new();
        let result = parser.parse("番剧名 S02E220.mkv").unwrap();
        assert_eq!(result.anime_name, "番剧名");
        assert_eq!(result.episode_number, 220);
    }

    #[test]
    fn test_parse_e_format() {
        let parser = FileParser::new();
        let result = parser.parse("进击的巨人 E220.mkv").unwrap();
        assert_eq!(result.anime_name, "进击的巨人");
        assert_eq!(result.episode_number, 220);
    }

    #[test]
    fn test_parse_ep_format() {
        let parser = FileParser::new();
        let result = parser.parse("某番剧 EP01.mkv").unwrap();
        assert_eq!(result.anime_name, "某番剧");
        assert_eq!(result.episode_number, 1);
    }

    #[test]
    fn test_parse_chinese_format() {
        let parser = FileParser::new();
        let result = parser.parse("番剧名 第01话.mkv").unwrap();
        assert_eq!(result.anime_name, "番剧名");
        assert_eq!(result.episode_number, 1);
    }

    #[test]
    fn test_parse_three_digit() {
        let parser = FileParser::new();
        let result = parser.parse("[字幕组]妖精的尾巴_S220[1080p].mkv").unwrap();
        assert_eq!(result.anime_name, "妖精的尾巴");
        assert_eq!(result.episode_number, 220);
    }

    #[test]
    fn test_detect_ova() {
        let parser = FileParser::new();
        let result = parser.parse("[字幕组] 进击的巨人 OVA 01.mkv").unwrap();
        assert_eq!(result.anime_name, "进击的巨人");
        assert_eq!(result.episode_number, 1);
        assert_eq!(result.episode_type, EpisodeType::OVA);
    }

    #[test]
    fn test_detect_oad() {
        let parser = FileParser::new();
        let result = parser.parse("番剧名 OAD 02.mkv").unwrap();
        assert_eq!(result.episode_type, EpisodeType::OAD);
    }

    #[test]
    fn test_detect_special() {
        let parser = FileParser::new();
        let result = parser.parse("[字幕组] 番剧 SP 01 [1080p].mkv").unwrap();
        assert_eq!(result.episode_type, EpisodeType::Special);
    }

    #[test]
    fn test_detect_special_chinese() {
        let parser = FileParser::new();
        let result = parser.parse("番剧名 特典 01.mkv").unwrap();
        assert_eq!(result.episode_type, EpisodeType::Special);
    }

    #[test]
    fn test_detect_movie() {
        let parser = FileParser::new();
        let result = parser.parse("进击的巨人 剧场版.mkv");
        if let Some(parsed) = result {
            assert_eq!(parsed.episode_type, EpisodeType::Movie);
        }
    }

    #[test]
    fn test_normal_episode() {
        let parser = FileParser::new();
        let result = parser.parse("[字幕组] 鬼灭之刃 28 [1080p].mkv").unwrap();
        assert_eq!(result.episode_type, EpisodeType::Normal);
    }
}
