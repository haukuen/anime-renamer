use crate::anilist::{AniListClient, Media};
use crate::cli::RenameArgs;
use crate::parser::{EpisodeType, FileParser, ParsedFile, extract_tmdb_id};
use crate::scanner::FileScanner;
use crate::tmdb::{Season, TmdbClient, TvDetails};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

type ParsedEntry = (PathBuf, ParsedFile);
type RenameEntry = (PathBuf, PathBuf, u32, u32);

fn apply_offset(episode: u32, offset: i32) -> u32 {
    (episode as i32 + offset).max(1) as u32
}

fn compute_season_episode(
    episode_type: &EpisodeType,
    episode_number: u32,
    season_number: Option<u32>,
    args_season: Option<u32>,
    offset: i32,
    normal_seasons: &[Season],
) -> Option<(u32, u32)> {
    match episode_type {
        EpisodeType::Normal => {
            let ep = apply_offset(episode_number, offset);
            if let Some(s) = args_season {
                Some((s, ep))
            } else if let Some(s) = season_number {
                Some((s, ep))
            } else {
                map_episode_to_season(ep, normal_seasons)
            }
        }
        EpisodeType::OVA | EpisodeType::Special | EpisodeType::OAD => {
            let ep = apply_offset(episode_number, offset);
            Some((0, ep))
        }
        EpisodeType::Movie => None,
    }
}

fn map_episode_to_season(episode_num: u32, seasons: &[Season]) -> Option<(u32, u32)> {
    let mut accumulated = 0u32;

    for season in seasons {
        if season.season_number == 0 {
            continue;
        }

        if episode_num <= accumulated + season.episode_count {
            let season_episode = episode_num - accumulated;
            return Some((season.season_number, season_episode));
        }

        accumulated += season.episode_count;
    }

    None
}

fn file_name_lossy(path: &Path) -> Option<String> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
}

fn display_file_name(path: &Path) -> String {
    file_name_lossy(path).unwrap_or_else(|| path.display().to_string())
}

fn print_rename_preview(rename_map: &[RenameEntry], season_folders: bool) {
    println!("重命名预览:\n");
    for (i, (old_path, new_path, season, episode)) in rename_map.iter().enumerate() {
        println!("[{}] S{:02}E{:02}", i + 1, season, episode);
        println!("  原文件: {}", display_file_name(old_path));

        if season_folders {
            if let Some(old_parent) = old_path.parent() {
                let relative_path = new_path.strip_prefix(old_parent).unwrap_or(new_path);
                println!("  新路径: {}", relative_path.display());
            } else {
                println!("  新文件: {}", display_file_name(new_path));
            }
        } else {
            println!("  新文件: {}", display_file_name(new_path));
        }

        let subtitles = FileScanner::find_associated_subtitles(old_path);
        if !subtitles.is_empty() {
            let old_stem = old_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let suffixes: Vec<String> = subtitles
                .iter()
                .filter_map(|p| {
                    let name = p.file_name()?.to_str()?;
                    Some(name[old_stem.len()..].to_string())
                })
                .collect();
            println!("  字幕: {}", suffixes.join(", "));
        }
        println!();
    }
}

fn execute_rename(rename_map: &[RenameEntry], dry_run: bool) -> Result<()> {
    use std::io::{self, Write};

    if dry_run {
        println!("预览模式，未实际重命名");
        return Ok(());
    }

    print!("继续重命名？[Y/n] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().is_empty() && !input.trim().eq_ignore_ascii_case("y") {
        println!("已取消");
        return Ok(());
    }

    let mut video_success = 0;
    let mut subtitle_success = 0;

    for (old_path, new_path, _, _) in rename_map {
        if let Some(parent_dir) = new_path.parent()
            && !parent_dir.exists()
            && let Err(e) = std::fs::create_dir_all(parent_dir)
        {
            println!("创建目录失败: {} - {}", parent_dir.display(), e);
            continue;
        }

        let old_video_stem = old_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let subtitles = FileScanner::find_associated_subtitles(old_path);

        if let Err(e) = std::fs::rename(old_path, new_path) {
            println!("重命名失败: {} - {}", old_path.display(), e);
            continue;
        }
        video_success += 1;

        for subtitle_path in &subtitles {
            if let Some(new_subtitle_path) =
                FileScanner::compute_subtitle_new_path(subtitle_path, old_video_stem, new_path)
            {
                if let Err(e) = std::fs::rename(subtitle_path, &new_subtitle_path) {
                    println!(
                        "字幕重命名失败: {} - {}",
                        subtitle_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        e
                    );
                } else {
                    subtitle_success += 1;
                }
            }
        }
    }

    println!("\n成功重命名 {} 个视频文件", video_success);
    if subtitle_success > 0 {
        println!("成功重命名 {} 个字幕文件", subtitle_success);
    }

    Ok(())
}

fn collect_rename_candidates(files: &[PathBuf], parser: &FileParser) -> Vec<ParsedEntry> {
    let mut parsed_files = Vec::new();
    let mut skipped_formatted = 0;

    for file in files {
        let Some(filename) = file_name_lossy(file) else {
            println!("无法获取文件名: {}", file.display());
            continue;
        };

        if let Some(parsed) = parser.parse(&filename) {
            if parsed.is_already_formatted {
                skipped_formatted += 1;
                continue;
            }
            parsed_files.push((file.clone(), parsed));
        } else {
            println!("无法解析: {}", filename);
        }
    }

    if skipped_formatted > 0 {
        println!("跳过 {} 个已规范化的文件\n", skipped_formatted);
    }

    parsed_files
}

fn build_output_name(
    show_name: &str,
    season: u32,
    episode: u32,
    extension: &str,
    keep_tags: bool,
    tags: &[String],
) -> String {
    if keep_tags && !tags.is_empty() {
        let tags_str = tags
            .iter()
            .map(|tag| format!("[{}]", tag))
            .collect::<Vec<_>>()
            .join("");
        format!(
            "{} S{:02}E{:02}{}.{}",
            show_name, season, episode, tags_str, extension
        )
    } else {
        format!("{} S{:02}E{:02}.{}", show_name, season, episode, extension)
    }
}

fn season_folder_name(season: u32) -> String {
    if season == 0 {
        "Season 0".to_string()
    } else {
        format!("Season {}", season)
    }
}

fn build_rename_target(
    parent: &Path,
    new_name: &str,
    season: u32,
    season_folders: bool,
) -> PathBuf {
    if season_folders {
        parent.join(season_folder_name(season)).join(new_name)
    } else {
        parent.join(new_name)
    }
}

fn handle_anilist_renaming(
    args: &RenameArgs,
    parsed_files: &[ParsedEntry],
    anime_name: &str,
) -> Result<()> {
    let mut rename_map = Vec::new();

    for (file_path, parsed) in parsed_files {
        let parent = file_path.parent().unwrap();
        let season = args
            .season
            .unwrap_or_else(|| parsed.season_number.unwrap_or(1));
        let episode = apply_offset(parsed.episode_number, args.offset);
        let new_name = build_output_name(
            anime_name,
            season,
            episode,
            &parsed.extension,
            args.keep_tags,
            &parsed.tags,
        );
        let new_path = build_rename_target(parent, &new_name, season, args.season_folders);

        rename_map.push((file_path.clone(), new_path, season, episode));
    }

    print_rename_preview(&rename_map, args.season_folders);
    execute_rename(&rename_map, args.dry_run)
}

async fn prompt_anilist_title(anime: &Media) -> Result<Option<String>> {
    use std::io::{self, Write};

    println!("\n找到番剧，请选择使用哪个标题:");
    let mut title_options = Vec::new();

    if let Some(ref native) = anime.title.native {
        title_options.push(native.clone());
        println!("  [{}] {} (原语言)", title_options.len(), native);
    }

    if let Some(ref romaji) = anime.title.romaji {
        title_options.push(romaji.clone());
        println!("  [{}] {} (罗马音)", title_options.len(), romaji);
    }

    if let Some(ref english) = anime.title.english {
        title_options.push(english.clone());
        println!("  [{}] {} (英文)", title_options.len(), english);
    }

    if title_options.is_empty() {
        return Ok(None);
    }

    print!(
        "\n请输入数字选择标题 [1-{}]，或输入自定义名称: ",
        title_options.len()
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    let display_name = if let Ok(choice) = input.parse::<usize>() {
        if choice > 0 && choice <= title_options.len() {
            title_options[choice - 1].clone()
        } else {
            println!("无效选择，使用第一个选项");
            title_options[0].clone()
        }
    } else if !input.is_empty() {
        input.to_string()
    } else {
        title_options[0].clone()
    };

    Ok(Some(display_name))
}

fn build_tmdb_rename_map(
    args: &RenameArgs,
    parsed_files: &[ParsedEntry],
    details: &TvDetails,
) -> Vec<RenameEntry> {
    let normal_seasons: Vec<_> = details
        .seasons
        .iter()
        .filter(|s| s.season_number > 0)
        .cloned()
        .collect();

    let mut rename_map = Vec::new();

    for (file_path, parsed) in parsed_files {
        let parent = file_path.parent().unwrap();
        let (season, episode) = match compute_season_episode(
            &parsed.episode_type,
            parsed.episode_number,
            parsed.season_number,
            args.season,
            args.offset,
            &normal_seasons,
        ) {
            Some(result) => result,
            None => {
                if parsed.episode_type == EpisodeType::Movie {
                    println!("跳过剧场版: {}", display_file_name(file_path));
                } else {
                    let ep = apply_offset(parsed.episode_number, args.offset);
                    println!("无法映射第 {} 集到任何季", ep);
                }
                continue;
            }
        };

        let new_name = build_output_name(
            &details.name,
            season,
            episode,
            &parsed.extension,
            args.keep_tags,
            &parsed.tags,
        );
        let new_path = build_rename_target(parent, &new_name, season, args.season_folders);

        rename_map.push((file_path.clone(), new_path, season, episode));
    }

    rename_map
}

async fn rename_with_tmdb_id(
    args: &RenameArgs,
    parsed_files: &[ParsedEntry],
    tmdb_id: u32,
) -> Result<()> {
    println!("使用 TMDB ID: {}", tmdb_id);
    let client = TmdbClient::new();

    let details = client
        .get_tv_details(tmdb_id, &args.language)
        .await
        .context("通过 ID 获取详情失败")?;

    println!("找到匹配: {} (TMDB ID: {})", details.name, tmdb_id);
    println!("共 {} 季，开始分析集数映射...\n", details.number_of_seasons);

    let rename_map = build_tmdb_rename_map(args, parsed_files, &details);
    print_rename_preview(&rename_map, args.season_folders);
    execute_rename(&rename_map, args.dry_run)
}

async fn rename_with_anilist(
    args: &RenameArgs,
    parsed_files: &[ParsedEntry],
    anime_name: &str,
) -> Result<()> {
    println!("按参数要求使用 AniList...");

    let anilist_client = AniListClient::new();
    let anilist_results = anilist_client
        .search_anime(anime_name)
        .await
        .context("AniList 搜索失败")?;

    if anilist_results.is_empty() {
        println!("AniList 未找到匹配的番剧");
        return Ok(());
    }

    let anime = &anilist_results[0];
    let Some(display_name) = prompt_anilist_title(anime).await? else {
        println!("未找到可用的标题");
        return Ok(());
    };

    println!("找到匹配: {} ({})", display_name, anime.format_date());
    println!("\n注意: AniList 不提供季度信息，将使用文件名中的季度标记");
    println!("如果文件名没有季度标记（如 'V', 'Season 5'），可能会映射错误\n");

    handle_anilist_renaming(args, parsed_files, &display_name)
}

pub(crate) async fn run(args: &RenameArgs) -> Result<()> {
    let path = args.path.as_str();

    println!("扫描目录: {}", path);

    let scanner = FileScanner::new(args.recursive);
    let files = scanner.scan(path);

    if files.is_empty() {
        println!("未找到视频文件");
        return Ok(());
    }

    println!("找到 {} 个视频文件\n", files.len());

    let parser = FileParser::new();
    let parsed_files = collect_rename_candidates(&files, &parser);

    if parsed_files.is_empty() {
        println!("没有可解析的文件");
        return Ok(());
    }

    let anime_name = args
        .name
        .clone()
        .unwrap_or_else(|| parsed_files[0].1.anime_name.clone());

    println!("检测到番剧: {}", anime_name);

    if let Some(id) = args.tmdb_id.or_else(|| extract_tmdb_id(path)) {
        return rename_with_tmdb_id(args, &parsed_files, id).await;
    }

    if args.use_anilist {
        return rename_with_anilist(args, &parsed_files, &anime_name).await;
    }

    let client = TmdbClient::new();
    println!("搜索 TMDB...");

    let results = client
        .search_tv(&anime_name, &args.language)
        .await
        .context("搜索失败")?;

    if results.is_empty() {
        println!("TMDB 未找到结果，尝试 AniList...");
        return rename_with_anilist(args, &parsed_files, &anime_name).await;
    }

    let tv_show = &results[0];
    println!(
        "找到匹配: {} ({})",
        tv_show.name,
        tv_show.first_air_date.as_deref().unwrap_or("未知")
    );

    let details = client
        .get_tv_details(tv_show.id, &args.language)
        .await
        .context("获取详情失败")?;

    println!("共 {} 季，开始分析集数映射...\n", details.number_of_seasons);

    let rename_map = build_tmdb_rename_map(args, &parsed_files, &details);
    print_rename_preview(&rename_map, args.season_folders);
    execute_rename(&rename_map, args.dry_run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_season(season_number: u32, episode_count: u32) -> Season {
        Season {
            season_number,
            episode_count,
            name: format!("Season {}", season_number),
            poster_path: None,
        }
    }

    #[test]
    fn test_apply_offset_never_drops_below_one() {
        assert_eq!(apply_offset(1, -10), 1);
        assert_eq!(apply_offset(3, -1), 2);
    }

    #[test]
    fn test_map_episode_to_season_skips_special_season_zero() {
        let seasons = vec![make_season(0, 2), make_season(1, 12), make_season(2, 12)];

        assert_eq!(map_episode_to_season(13, &seasons), Some((2, 1)));
    }

    #[test]
    fn test_compute_season_episode_prefers_explicit_season_arg() {
        let seasons = vec![make_season(1, 12), make_season(2, 12)];

        let result = compute_season_episode(&EpisodeType::Normal, 5, Some(1), Some(3), 0, &seasons);

        assert_eq!(result, Some((3, 5)));
    }

    #[test]
    fn test_compute_season_episode_uses_parsed_season_when_present() {
        let seasons = vec![make_season(1, 12), make_season(2, 12)];

        let result = compute_season_episode(&EpisodeType::Normal, 7, Some(2), None, 0, &seasons);

        assert_eq!(result, Some((2, 7)));
    }

    #[test]
    fn test_compute_season_episode_maps_absolute_episode_across_seasons() {
        let seasons = vec![make_season(1, 12), make_season(2, 12)];

        let result = compute_season_episode(&EpisodeType::Normal, 14, None, None, 0, &seasons);

        assert_eq!(result, Some((2, 2)));
    }

    #[test]
    fn test_compute_season_episode_maps_specials_to_season_zero() {
        let seasons = vec![make_season(1, 12)];

        let result = compute_season_episode(&EpisodeType::OVA, 3, None, None, -1, &seasons);

        assert_eq!(result, Some((0, 2)));
    }

    #[test]
    fn test_compute_season_episode_returns_none_for_movie() {
        let seasons = vec![make_season(1, 12)];

        let result = compute_season_episode(&EpisodeType::Movie, 1, None, None, 0, &seasons);

        assert_eq!(result, None);
    }
}
