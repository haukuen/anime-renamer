mod matchers;

use matchers::*;
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
    pub season_number: Option<u32>,
    pub episode_type: EpisodeType,
    pub tags: Vec<String>,
    pub extension: String,
    pub is_already_formatted: bool,
}

pub struct FileParser {
    season_chain: MatcherChain,
    episode_chain: MatcherChain,
    tag_regex: Regex,
    special_keywords: Vec<(Regex, EpisodeType)>,
}

impl FileParser {
    pub fn new() -> Self {
        let season_chain = MatcherChain::new()
            .add_matcher(Box::new(SeasonNumberMatcher::new())) // S3
            .add_matcher(Box::new(SeasonWordMatcher::new())) // Season 3
            .add_matcher(Box::new(ChineseSeasonMatcher::new())) // 第3季
            .add_matcher(Box::new(RomanSeasonMatcher::new())); // IV

        let episode_chain = MatcherChain::new()
            .add_matcher(Box::new(SxEyMatcher::new())) // S01E12
            .add_matcher(Box::new(ChineseEpisodeMatcher::new())) // 第01集
            .add_matcher(Box::new(EpMatcher::new())) // EP01
            .add_matcher(Box::new(EMatcher::new())) // E220
            .add_matcher(Box::new(BracketEpisodeMatcher::new())) // [01]
            .add_matcher(Box::new(UnderscoreSMatcher::new())) // _S001
            .add_matcher(Box::new(DelimiterEpisodeMatcher::new())); // - 04

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
            season_chain,
            episode_chain,
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

    fn extract_season(&self, text: &str) -> Option<u32> {
        self.season_chain
            .execute(text, &[])
            .map(|result| result.value)
    }

    fn extract_episode(&self, text: &str) -> Option<(u32, String)> {
        // 先获取季度的数字位置，避免重叠
        let mut exclude_positions = Vec::new();

        if let Some(season_result) = self.season_chain.execute(text, &[]) {
            exclude_positions.push((season_result.start_pos, season_result.end_pos));
        }

        // 排除分辨率标签的位置（如 [1080], [720]）
        let resolution_regex = Regex::new(r"\[(1080|720|480|2160|4K)[^\]]*\]").unwrap();
        for cap in resolution_regex.captures_iter(text) {
            if let Some(m) = cap.get(0) {
                exclude_positions.push((m.start(), m.end()));
            }
        }

        self.episode_chain
            .execute(text, &exclude_positions)
            .map(|result| (result.value, result.matched_text))
    }

    fn clean_anime_name(&self, name: &str) -> String {
        let mut name = self.tag_regex.replace_all(name, "").to_string();

        for (pattern, _) in &self.special_keywords {
            name = pattern.replace_all(&name, " ").to_string();
        }

        let season_cleanup = vec![
            Regex::new(r"[Ss]eason\s*\d{1,2}").unwrap(),
            Regex::new(r"第\s*\d{1,2}\s*季").unwrap(),
            Regex::new(r"\b[IVX]+\b").unwrap(),
            Regex::new(r"[Ss]\d{1,2}(?:\s|[\]\[]|$)").unwrap(),
            Regex::new(r"_[Ss]\d{1,4}").unwrap(), // 添加 _S001 格式
        ];

        for pattern in &season_cleanup {
            name = pattern.replace_all(&name, " ").to_string();
        }

        // 移除括号及其内容（通常是总集数）
        let paren_regex = Regex::new(r"\([^)]*\)").unwrap();
        name = paren_regex.replace_all(&name, " ").to_string();

        if let Some(pos) = name.find('：') {
            name = name[..pos].to_string();
        }

        if let Some(pos) = name.find(':') {
            name = name[..pos].to_string();
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

        let season_number = self.extract_season(stem);

        let (episode_number, episode_match) = self.extract_episode(stem)?;

        // 找到包含集数的方括号标签的索引（排除分辨率标签）
        let episode_tag_index = tags.iter().position(|tag| {
            // 排除常见的分辨率标签
            if tag.contains("1080")
                || tag.contains("720")
                || tag.contains("480")
                || tag.contains("2160")
                || tag.contains("4K")
            {
                return false;
            }
            tag.parse::<u32>().is_ok()
        });

        let mut anime_name = if let Some(idx) = episode_tag_index {
            // 找到集数标签，现在要从前面的标签中提取番剧名
            // 策略：找到最长的有意义的标签（通常是番剧名）
            let candidate_tags: Vec<&String> = tags[..idx]
                .iter()
                .filter(|tag| {
                    // 过滤掉字幕组、分辨率等标签
                    let tag_lower = tag.to_lowercase();
                    !tag_lower.contains("字幕")
                        && !tag_lower.contains("新番")
                        && !tag.contains("1080")
                        && !tag.contains("720")
                        && tag.len() > 2 // 至少3个字符
                })
                .collect();

            if !candidate_tags.is_empty() {
                // 找到最长的标签，通常是番剧名
                let longest_tag = candidate_tags.iter().max_by_key(|tag| tag.len()).unwrap();
                longest_tag.to_string()
            } else if idx > 0 {
                tags[..idx].join(" ")
            } else {
                stem.to_string()
            }
        } else {
            // 否则使用原逻辑
            let mut name = stem.to_string();

            // 移除集数匹配及后面的括号内容（如果有）
            let remove_pattern = if episode_match.ends_with('(') {
                // 如果集数匹配以 ( 结尾，移除到对应的 )
                let start_pos = name.find(&episode_match).unwrap_or(0);
                let after_match = &name[start_pos + episode_match.len()..];
                if let Some(close_pos) = after_match.find(')') {
                    format!("{}{}", episode_match, &after_match[..=close_pos])
                } else {
                    episode_match.clone()
                }
            } else {
                episode_match.clone()
            };

            name = name.replace(&remove_pattern, " ");
            name
        };

        anime_name = self.clean_anime_name(&anime_name);

        if anime_name.is_empty() {
            return None;
        }

        Some(ParsedFile {
            anime_name,
            episode_number,
            season_number,
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

    #[test]
    fn test_complex_filename_with_season_and_episode() {
        let parser = FileParser::new();
        let result = parser.parse("[爱恋字幕社][1月新番][在地下城寻求邂逅是否搞错了什么 IV 深章 灾厄篇][Dungeon ni Deai wo Motomeru no wa Machigatteiru Darou ka S4][22][1080P][MP4][繁中].mkv");

        if let Some(parsed) = result {
            println!("解析成功!");
            println!("  番剧名: {}", parsed.anime_name);
            println!("  集数: {}", parsed.episode_number);
            println!("  季度: {:?}", parsed.season_number);
        } else {
            panic!("解析失败");
        }
    }

    #[test]
    fn test_one_punch_man_format() {
        let parser = FileParser::new();

        let filename =
            "[LoliHouse] One-Punch Man S3 - 04(28) [WebRip 1080p HEVC-10bit AAC SRTx2].mkv";
        println!("测试文件: {}", filename);

        let stem = "[LoliHouse] One-Punch Man S3 - 04(28) [WebRip 1080p HEVC-10bit AAC SRTx2]";

        // 测试季度提取
        let season = parser.extract_season(stem);
        println!("\n提取到的季度: {:?}", season);

        // 测试集数提取
        let episode = parser.extract_episode(stem);
        println!("提取到的集数: {:?}", episode);

        let result = parser.parse(filename);

        match result {
            Some(parsed) => {
                println!("\n解析成功!");
                println!("  番剧名: {}", parsed.anime_name);
                println!("  集数: {}", parsed.episode_number);
                println!("  季度: {:?}", parsed.season_number);
                assert_eq!(parsed.season_number, Some(3));
                assert_eq!(parsed.episode_number, 4);
            }
            None => {
                panic!("解析失败");
            }
        }
    }
}
