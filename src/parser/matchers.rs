use regex::Regex;

/// 匹配结果
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub value: u32,
    pub matched_text: String,
    pub start_pos: usize,
    pub end_pos: usize,
}

/// 匹配器 trait - 责任链中的一环
pub trait Matcher: Send + Sync {
    fn try_match(&self, text: &str) -> Option<MatchResult>;

    fn priority(&self) -> u32;

    /// 用于调试
    #[allow(dead_code)]
    fn name(&self) -> &str;
}

/// 责任链管理器
pub struct MatcherChain {
    matchers: Vec<Box<dyn Matcher>>,
}

impl MatcherChain {
    pub fn new() -> Self {
        Self {
            matchers: Vec::new(),
        }
    }

    pub fn add_matcher(mut self, matcher: Box<dyn Matcher>) -> Self {
        self.matchers.push(matcher);
        // 按优先级排序
        self.matchers.sort_by_key(|m| m.priority());
        self
    }

    /// 执行匹配链，返回第一个成功的匹配
    pub fn execute(&self, text: &str, exclude_positions: &[(usize, usize)]) -> Option<MatchResult> {
        for matcher in &self.matchers {
            if let Some(result) = matcher.try_match(text) {
                // 检查是否与排除位置重叠
                let overlaps = exclude_positions
                    .iter()
                    .any(|(start, end)| result.start_pos < *end && result.end_pos > *start);

                if !overlaps {
                    return Some(result);
                }
            }
        }
        None
    }
}

/// S3, S04 等格式
pub struct SeasonNumberMatcher {
    regex: Regex,
}

impl SeasonNumberMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[Ss](\d{1,2})(?:\s|[\]\[]|$)").unwrap(),
        }
    }
}

impl Matcher for SeasonNumberMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        1
    }
    fn name(&self) -> &str {
        "SeasonNumber(S3)"
    }
}

/// Season 3 格式
pub struct SeasonWordMatcher {
    regex: Regex,
}

impl SeasonWordMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[Ss]eason\s*(\d{1,2})").unwrap(),
        }
    }
}

impl Matcher for SeasonWordMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        2
    }
    fn name(&self) -> &str {
        "SeasonWord(Season 3)"
    }
}

/// 第3季 格式
pub struct ChineseSeasonMatcher {
    regex: Regex,
}

impl ChineseSeasonMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"第\s*(\d{1,2})\s*季").unwrap(),
        }
    }
}

impl Matcher for ChineseSeasonMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        3
    }
    fn name(&self) -> &str {
        "ChineseSeason(第3季)"
    }
}

/// 罗马数字季度 (I, II, III, IV, V)
pub struct RomanSeasonMatcher {
    regex: Regex,
}

impl RomanSeasonMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"\b([IVX]+)\b").unwrap(),
        }
    }

    fn roman_to_number(&self, roman: &str) -> Option<u32> {
        match roman.to_uppercase().as_str() {
            "I" => Some(1),
            "II" => Some(2),
            "III" => Some(3),
            "IV" => Some(4),
            "V" => Some(5),
            "VI" => Some(6),
            "VII" => Some(7),
            "VIII" => Some(8),
            "IX" => Some(9),
            "X" => Some(10),
            _ => None,
        }
    }
}

impl Matcher for RomanSeasonMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let roman_match = cap.get(1)?;
            let roman = roman_match.as_str();
            let value = self.roman_to_number(roman)?;

            // 额外验证：前面应该有空格或方括号
            let before_ok = roman_match.start() == 0 || {
                text.chars()
                    .nth(roman_match.start().saturating_sub(1))
                    .is_some_and(|c| c.is_whitespace() || c == '[' || c == ']')
            };

            if before_ok {
                Some(MatchResult {
                    value,
                    matched_text: roman.to_string(),
                    start_pos: roman_match.start(),
                    end_pos: roman_match.end(),
                })
            } else {
                None
            }
        })
    }

    fn priority(&self) -> u32 {
        10
    }
    fn name(&self) -> &str {
        "RomanSeason(IV)"
    }
}

// ==================== 集数匹配器 ====================

/// S01E12 格式
pub struct SxEyMatcher {
    regex: Regex,
}

impl SxEyMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[Ss](\d{1,2})[Ee](\d{1,4})").unwrap(),
        }
    }
}

impl Matcher for SxEyMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(2)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        1
    }
    fn name(&self) -> &str {
        "SxEy(S01E12)"
    }
}

/// [01] 方括号格式
pub struct BracketEpisodeMatcher {
    regex: Regex,
}

impl BracketEpisodeMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"\[(\d{1,4})\]").unwrap(),
        }
    }
}

impl Matcher for BracketEpisodeMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        5
    }
    fn name(&self) -> &str {
        "Bracket([01])"
    }
}

/// 第01集/话 中文格式
pub struct ChineseEpisodeMatcher {
    regex: Regex,
}

impl ChineseEpisodeMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"第\s*(\d{1,4})\s*(?:集|话|話)").unwrap(),
        }
    }
}

impl Matcher for ChineseEpisodeMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        2
    }
    fn name(&self) -> &str {
        "ChineseEpisode(第01集)"
    }
}

/// EP01 格式
pub struct EpMatcher {
    regex: Regex,
}

impl EpMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[Ee][Pp]\s*(\d{1,4})").unwrap(),
        }
    }
}

impl Matcher for EpMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        3
    }
    fn name(&self) -> &str {
        "Ep(EP01)"
    }
}

/// - 04, : 04 等分隔符后的数字
pub struct DelimiterEpisodeMatcher {
    regex: Regex,
}

impl DelimiterEpisodeMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[\s\-_\.：:]+(\d{1,4})(?:\D|$)").unwrap(),
        }
    }
}

impl Matcher for DelimiterEpisodeMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        20
    }
    fn name(&self) -> &str {
        "Delimiter(- 04)"
    }
}

/// _S001 格式（老式动画编号）
pub struct UnderscoreSMatcher {
    regex: Regex,
}

impl UnderscoreSMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"_[Ss](\d{1,4})").unwrap(),
        }
    }
}

impl Matcher for UnderscoreSMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        6
    }
    fn name(&self) -> &str {
        "UnderscoreS(_S001)"
    }
}

/// E220 格式（单独的E+数字）
pub struct EMatcher {
    regex: Regex,
}

impl EMatcher {
    pub fn new() -> Self {
        Self {
            regex: Regex::new(r"[Ee](\d{1,4})(?:\D|$)").unwrap(),
        }
    }
}

impl Matcher for EMatcher {
    fn try_match(&self, text: &str) -> Option<MatchResult> {
        self.regex.captures(text).and_then(|cap| {
            let num_match = cap.get(1)?;
            let value = num_match.as_str().parse::<u32>().ok()?;
            Some(MatchResult {
                value,
                matched_text: cap.get(0)?.as_str().to_string(),
                start_pos: num_match.start(),
                end_pos: num_match.end(),
            })
        })
    }

    fn priority(&self) -> u32 {
        4
    }
    fn name(&self) -> &str {
        "E(E220)"
    }
}
