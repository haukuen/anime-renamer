mod anilist;
mod nfo;
mod parser;
mod scanner;
mod tmdb;

use anilist::AniListClient;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser as ClapParser, Subcommand};
use nfo::{ActorNfo, EpisodeNfo, NfoWriter, PersonNfo, Rating, TvShowNfo, UniqueId, WriteAction};
use parser::{EpisodeType, FileParser, ParsedFile, extract_tmdb_id};
use scanner::FileScanner;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tmdb::{Episode, EpisodeCredits, EpisodeExternalIds, SeasonDetails, TmdbClient, TvDetails};

type ParsedEntry = (PathBuf, ParsedFile);
type RenameEntry = (PathBuf, PathBuf, u32, u32);

#[derive(ClapParser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
#[command(
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    flatten_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    rename: RenameCliArgs,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    Nfo(NfoArgs),
}

#[derive(Args, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct RenameCliArgs {
    /// 要扫描的目录路径
    path: Option<String>,

    /// 是否递归扫描子目录
    #[arg(short, long)]
    recursive: bool,

    /// 预览模式（不实际重命名）
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// 指定番剧名称（跳过自动识别）
    #[arg(long)]
    name: Option<String>,

    /// 语言偏好
    #[arg(short, long, default_value = "zh-CN")]
    language: String,

    /// 保留所有标签
    #[arg(long)]
    keep_tags: bool,

    /// 为每一季创建单独的文件夹（Season 1, Season 2, ...）
    #[arg(long)]
    season_folders: bool,

    /// 使用 AniList API 而不是 TMDB（更好的罗马音支持）
    #[arg(long)]
    use_anilist: bool,

    /// 手动指定季度（跳过自动映射）
    #[arg(short, long)]
    season: Option<u32>,

    /// 集数偏移量
    #[arg(short, long, default_value = "0", allow_hyphen_values = true)]
    offset: i32,

    /// 直接指定 TMDB ID
    #[arg(short = 'i', long)]
    tmdb_id: Option<u32>,
}

#[derive(Debug, Clone)]
struct RenameArgs {
    path: String,
    recursive: bool,
    dry_run: bool,
    name: Option<String>,
    language: String,
    keep_tags: bool,
    season_folders: bool,
    use_anilist: bool,
    season: Option<u32>,
    offset: i32,
    tmdb_id: Option<u32>,
}

impl TryFrom<RenameCliArgs> for RenameArgs {
    type Error = anyhow::Error;

    fn try_from(value: RenameCliArgs) -> Result<Self> {
        Ok(Self {
            path: value.path.context("缺少要扫描的目录路径")?,
            recursive: value.recursive,
            dry_run: value.dry_run,
            name: value.name,
            language: value.language,
            keep_tags: value.keep_tags,
            season_folders: value.season_folders,
            use_anilist: value.use_anilist,
            season: value.season,
            offset: value.offset,
            tmdb_id: value.tmdb_id,
        })
    }
}

#[derive(Args, Debug, Clone)]
struct NfoArgs {
    /// 要扫描的目录路径
    path: String,

    /// 是否递归扫描子目录
    #[arg(short, long)]
    recursive: bool,

    /// 预览模式（不实际写入）
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// 语言偏好
    #[arg(short, long, default_value = "zh-CN")]
    language: String,

    /// 直接指定 TMDB ID
    #[arg(short = 'i', long)]
    tmdb_id: Option<u32>,

    /// 覆盖已有 NFO 文件
    #[arg(long)]
    force: bool,
}

fn apply_offset(episode: u32, offset: i32) -> u32 {
    (episode as i32 + offset).max(1) as u32
}

fn compute_season_episode(
    episode_type: &EpisodeType,
    episode_number: u32,
    season_number: Option<u32>,
    args_season: Option<u32>,
    offset: i32,
    normal_seasons: &[tmdb::Season],
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

/// 根据总集数映射到季和集
fn map_episode_to_season(episode_num: u32, seasons: &[tmdb::Season]) -> Option<(u32, u32)> {
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

fn collect_nfo_candidates(files: &[PathBuf], parser: &FileParser) -> Vec<ParsedEntry> {
    let mut parsed_files = Vec::new();

    for file in files {
        let Some(filename) = file_name_lossy(file) else {
            println!("无法获取文件名: {}", file.display());
            continue;
        };

        match parser.parse(&filename) {
            Some(parsed) if !parsed.is_already_formatted => {
                println!("跳过非规范命名文件: {}", filename);
            }
            Some(parsed) if parsed.episode_type == EpisodeType::Movie => {
                println!("跳过剧场版: {}", filename);
            }
            Some(parsed) => match parsed.season_number {
                Some(_) => parsed_files.push((file.clone(), parsed)),
                None => println!("跳过缺少季度信息的文件: {}", filename),
            },
            None => println!("无法解析: {}", filename),
        }
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

/// 处理 AniList 模式的重命名（不依赖 TMDB 季度信息）
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

    if args.dry_run {
        println!("预览模式，未实际重命名");
    } else {
        execute_rename(&rename_map, args.dry_run)?;
    }

    Ok(())
}

async fn prompt_anilist_title(anime: &anilist::Media) -> Result<Option<String>> {
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

async fn run_rename(args: &RenameArgs) -> Result<()> {
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

    let tmdb_id = args.tmdb_id.or_else(|| extract_tmdb_id(path));

    if let Some(id) = tmdb_id {
        println!("使用 TMDB ID: {}", id);
        let client = TmdbClient::new();

        let details = client
            .get_tv_details(id, &args.language)
            .await
            .context("通过 ID 获取详情失败")?;

        println!("找到匹配: {} (TMDB ID: {})", details.name, id);
        println!("共 {} 季，开始分析集数映射...\n", details.number_of_seasons);

        let rename_map = build_tmdb_rename_map(args, &parsed_files, &details);
        print_rename_preview(&rename_map, args.season_folders);

        if args.dry_run {
            println!("预览模式，未实际重命名");
        } else {
            execute_rename(&rename_map, args.dry_run)?;
        }

        return Ok(());
    }

    if args.use_anilist {
        println!("按参数要求使用 AniList...");

        let anilist_client = AniListClient::new();
        let anilist_results = anilist_client
            .search_anime(&anime_name)
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

        handle_anilist_renaming(args, &parsed_files, &display_name)?;
        return Ok(());
    }

    let client = TmdbClient::new();
    println!("搜索 TMDB...");

    let results = client
        .search_tv(&anime_name, &args.language)
        .await
        .context("搜索失败")?;

    if results.is_empty() {
        println!("TMDB 未找到结果，尝试 AniList...");

        let anilist_client = AniListClient::new();
        let anilist_results = anilist_client
            .search_anime(&anime_name)
            .await
            .context("AniList 搜索失败")?;

        if anilist_results.is_empty() {
            println!("AniList 也未找到匹配的番剧");
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

        handle_anilist_renaming(args, &parsed_files, &display_name)?;
        return Ok(());
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

    if args.dry_run {
        println!("预览模式，未实际重命名");
    } else {
        execute_rename(&rename_map, args.dry_run)?;
    }

    Ok(())
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

async fn resolve_tmdb_details(
    client: &TmdbClient,
    explicit_tmdb_id: Option<u32>,
    path: &str,
    anime_name: &str,
    language: &str,
) -> Result<(u32, TvDetails)> {
    if let Some(id) = explicit_tmdb_id.or_else(|| extract_tmdb_id(path)) {
        let details = client
            .get_tv_details(id, language)
            .await
            .context("通过 ID 获取详情失败")?;
        return Ok((id, details));
    }

    let results = client
        .search_tv(anime_name, language)
        .await
        .context("搜索 TMDB 失败")?;

    let Some(first) = results.first() else {
        bail!("TMDB 未找到匹配的番剧");
    };

    let details = client
        .get_tv_details(first.id, language)
        .await
        .context("获取详情失败")?;

    Ok((first.id, details))
}

fn required_seasons_for_nfo(parsed_files: &[ParsedEntry]) -> BTreeSet<u32> {
    parsed_files
        .iter()
        .filter_map(|(_, parsed)| parsed.season_number)
        .collect()
}

fn season_image_targets(parsed_files: &[ParsedEntry]) -> HashMap<u32, Vec<PathBuf>> {
    let mut seasons_by_dir: HashMap<PathBuf, BTreeSet<u32>> = HashMap::new();

    for (video_path, parsed) in parsed_files {
        let Some(season) = parsed.season_number else {
            continue;
        };
        let Some(parent) = video_path.parent() else {
            continue;
        };

        seasons_by_dir
            .entry(parent.to_path_buf())
            .or_default()
            .insert(season);
    }

    let mut targets = HashMap::new();

    for (dir, seasons) in seasons_by_dir {
        if seasons.len() != 1 {
            continue;
        }

        let season = *seasons
            .iter()
            .next()
            .expect("single season set is non-empty");
        targets.entry(season).or_insert_with(Vec::new).push(dir);
    }

    targets
}

async fn fetch_season_details_map(
    client: &TmdbClient,
    tv_id: u32,
    seasons: &BTreeSet<u32>,
    language: &str,
) -> Result<HashMap<u32, SeasonDetails>> {
    let mut season_details = HashMap::new();
    let mut failed_seasons = Vec::new();

    for season in seasons {
        match client.get_season_details(tv_id, *season, language).await {
            Ok(details) => {
                season_details.insert(details.season_number, details);
            }
            Err(error) => {
                println!("跳过第 {} 季元数据: {error}", season);
                failed_seasons.push(*season);
            }
        }
    }

    if season_details.is_empty() {
        if failed_seasons.is_empty() {
            bail!("未获取到任何季度详情");
        }
        bail!(
            "所有请求季度的详情都获取失败: {}",
            failed_seasons
                .iter()
                .map(|season| season.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(season_details)
}

fn build_episode_lookup(
    season_details_map: &HashMap<u32, SeasonDetails>,
) -> HashMap<(u32, u32), Episode> {
    let mut episodes = HashMap::new();

    for details in season_details_map.values() {
        for episode in &details.episodes {
            episodes.insert(
                (details.season_number, episode.episode_number),
                episode.clone(),
            );
        }
    }

    episodes
}

fn record_write_action(action: WriteAction, written: &mut usize, skipped_existing: &mut usize) {
    match action {
        WriteAction::WouldWrite | WriteAction::Written => *written += 1,
        WriteAction::SkippedExisting => *skipped_existing += 1,
    }
}

fn build_rating(value: f64, votes: u32) -> Option<Rating> {
    if votes == 0 && value <= 0.0 {
        return None;
    }

    Some(Rating {
        provider: "themoviedb".to_string(),
        value,
        votes,
        is_default: true,
    })
}

fn extract_year(date: Option<&str>) -> Option<u32> {
    date.and_then(|value| value.get(..4)?.parse::<u32>().ok())
}

fn collect_studios(details: &TvDetails) -> Vec<String> {
    let source = if !details.networks.is_empty() {
        &details.networks
    } else {
        &details.production_companies
    };

    source.iter().map(|item| item.name.clone()).collect()
}

fn resolve_season_poster_path<'a>(
    season_number: u32,
    season_details_map: &'a HashMap<u32, SeasonDetails>,
    tv_details: &'a TvDetails,
) -> Option<&'a str> {
    season_details_map
        .get(&season_number)
        .and_then(|details| details.poster_path.as_deref())
        .or_else(|| {
            tv_details
                .seasons
                .iter()
                .find(|item| item.season_number == season_number)
                .and_then(|season| season.poster_path.as_deref())
        })
}

fn build_episode_unique_ids(
    episode: &Episode,
    external_ids: Option<&EpisodeExternalIds>,
) -> Vec<UniqueId> {
    let imdb_id = external_ids
        .and_then(|ids| ids.imdb_id.clone())
        .filter(|value| !value.trim().is_empty());
    let tvdb_id = external_ids.and_then(|ids| ids.tvdb_id);

    let mut unique_ids = vec![UniqueId {
        id_type: "tmdb".to_string(),
        value: episode.id.to_string(),
        is_default: imdb_id.is_none(),
    }];

    if let Some(imdb_id) = imdb_id {
        unique_ids.push(UniqueId {
            id_type: "imdb".to_string(),
            value: imdb_id,
            is_default: true,
        });
    }

    if let Some(tvdb_id) = tvdb_id {
        unique_ids.push(UniqueId {
            id_type: "tvdb".to_string(),
            value: tvdb_id.to_string(),
            is_default: false,
        });
    }

    unique_ids
}

fn build_episode_credits(credits: Option<&EpisodeCredits>) -> Vec<PersonNfo> {
    let Some(credits) = credits else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut people = Vec::new();

    for crew in &credits.crew {
        let is_writing = crew.department.as_deref() == Some("Writing");
        if !is_writing || !seen.insert(crew.id) {
            continue;
        }

        people.push(PersonNfo {
            name: crew.name.clone(),
            tmdb_id: Some(crew.id),
        });
    }

    people
}

fn build_episode_directors(credits: Option<&EpisodeCredits>) -> Vec<PersonNfo> {
    let Some(credits) = credits else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut people = Vec::new();

    for crew in &credits.crew {
        if crew.job.as_deref() != Some("Director") || !seen.insert(crew.id) {
            continue;
        }

        people.push(PersonNfo {
            name: crew.name.clone(),
            tmdb_id: Some(crew.id),
        });
    }

    people
}

fn build_episode_actors(credits: Option<&EpisodeCredits>) -> Vec<ActorNfo> {
    let Some(credits) = credits else {
        return Vec::new();
    };

    let mut actors = Vec::new();

    for cast in &credits.cast {
        actors.push(ActorNfo {
            name: cast.name.clone(),
            role: cast
                .character
                .clone()
                .filter(|value| !value.trim().is_empty()),
            tmdb_id: Some(cast.id),
            actor_type: None,
        });
    }

    for guest_star in &credits.guest_stars {
        actors.push(ActorNfo {
            name: guest_star.name.clone(),
            role: guest_star
                .character
                .clone()
                .filter(|value| !value.trim().is_empty()),
            tmdb_id: Some(guest_star.id),
            actor_type: Some("GuestStar".to_string()),
        });
    }

    actors
}

fn build_tvshow_nfo(details: &TvDetails) -> TvShowNfo {
    TvShowNfo {
        title: details.name.clone(),
        plot: details
            .overview
            .clone()
            .filter(|value| !value.trim().is_empty()),
        premiered: details.first_air_date.clone(),
        year: extract_year(details.first_air_date.as_deref()),
        status: details
            .status
            .clone()
            .filter(|value| !value.trim().is_empty()),
        rating: build_rating(details.vote_average, details.vote_count),
        unique_ids: vec![UniqueId {
            id_type: "tmdb".to_string(),
            value: details.id.to_string(),
            is_default: true,
        }],
        tmdb_id: details.id,
        genres: details
            .genres
            .iter()
            .map(|genre| genre.name.clone())
            .collect(),
        studios: collect_studios(details),
        episodeguide: format!(r#"{{"tmdb":"{}"}}"#, details.id),
    }
}

fn build_episode_nfo(
    show_title: &str,
    season_number: u32,
    episode_number: u32,
    episode: &Episode,
    external_ids: Option<&EpisodeExternalIds>,
    credits: Option<&EpisodeCredits>,
) -> EpisodeNfo {
    EpisodeNfo {
        title: episode.name.clone(),
        showtitle: show_title.to_string(),
        season: season_number,
        episode: episode_number,
        plot: episode
            .overview
            .clone()
            .filter(|value| !value.trim().is_empty()),
        premiered: episode.air_date.clone(),
        aired: episode.air_date.clone(),
        rating: build_rating(episode.vote_average, episode.vote_count),
        unique_ids: build_episode_unique_ids(episode, external_ids),
        credits: build_episode_credits(credits),
        directors: build_episode_directors(credits),
        actors: build_episode_actors(credits),
    }
}

fn print_nfo_outcome(path: &Path, action: WriteAction) {
    let label = match action {
        WriteAction::WouldWrite => "预览写入",
        WriteAction::Written => "已写入",
        WriteAction::SkippedExisting => "已跳过（文件已存在）",
    };

    println!("{}: {}", label, path.display());
}

async fn handle_nfo_export(args: &NfoArgs) -> Result<()> {
    println!("扫描目录: {}", args.path);

    let scanner = FileScanner::new(args.recursive);
    let files = scanner.scan(&args.path);

    if files.is_empty() {
        println!("未找到视频文件");
        return Ok(());
    }

    println!("找到 {} 个视频文件\n", files.len());

    let parser = FileParser::new();
    let parsed_files = collect_nfo_candidates(&files, &parser);

    if parsed_files.is_empty() {
        println!("没有可用于导出 NFO 的规范化文件");
        return Ok(());
    }

    let anime_name = parsed_files[0].1.anime_name.clone();
    println!("检测到番剧: {}", anime_name);

    let client = TmdbClient::new();
    let (show_id, details) = resolve_tmdb_details(
        &client,
        args.tmdb_id,
        &args.path,
        &anime_name,
        &args.language,
    )
    .await?;

    println!("找到匹配: {} (TMDB ID: {})", details.name, show_id);

    let seasons = required_seasons_for_nfo(&parsed_files);
    let season_details_map =
        fetch_season_details_map(&client, show_id, &seasons, &args.language).await?;
    let episode_lookup = build_episode_lookup(&season_details_map);
    let season_targets = season_image_targets(&parsed_files);
    let writer = NfoWriter::new(args.dry_run, args.force);

    let tvshow_outcome = writer.write_tvshow(Path::new(&args.path), &build_tvshow_nfo(&details))?;
    print_nfo_outcome(&tvshow_outcome.path, tvshow_outcome.action);

    let mut nfo_written = 0;
    let mut nfo_skipped_existing = 0;
    record_write_action(
        tvshow_outcome.action,
        &mut nfo_written,
        &mut nfo_skipped_existing,
    );

    let mut image_written = 0;
    let mut image_skipped_existing = 0;
    let mut missing_images = 0;
    let mut image_failures = 0;
    let mut missing_metadata = 0;
    let mut metadata_enrichment_failures = 0;

    if let Some(poster_path) = details.poster_path.as_deref() {
        match client.download_image(poster_path).await {
            Ok(bytes) => {
                let outcome = writer.write_tvshow_primary_image(
                    Path::new(&args.path),
                    tmdb::image_extension(poster_path),
                    &bytes,
                )?;
                print_nfo_outcome(&outcome.path, outcome.action);
                record_write_action(
                    outcome.action,
                    &mut image_written,
                    &mut image_skipped_existing,
                );
            }
            Err(error) => {
                println!("跳过剧集海报下载失败: {error}");
                image_failures += 1;
            }
        }
    } else {
        missing_images += 1;
    }

    if let Some(backdrop_path) = details.backdrop_path.as_deref() {
        match client.download_image(backdrop_path).await {
            Ok(bytes) => {
                let outcome = writer.write_tvshow_backdrop_image(
                    Path::new(&args.path),
                    tmdb::image_extension(backdrop_path),
                    &bytes,
                )?;
                print_nfo_outcome(&outcome.path, outcome.action);
                record_write_action(
                    outcome.action,
                    &mut image_written,
                    &mut image_skipped_existing,
                );
            }
            Err(error) => {
                println!("跳过剧集背景图下载失败: {error}");
                image_failures += 1;
            }
        }
    } else {
        missing_images += 1;
    }

    for season in &seasons {
        let Some(target_dirs) = season_targets.get(season) else {
            continue;
        };
        let Some(poster_path) = resolve_season_poster_path(*season, &season_details_map, &details)
        else {
            missing_images += target_dirs.len();
            continue;
        };

        match client.download_image(poster_path).await {
            Ok(bytes) => {
                for target_dir in target_dirs {
                    let outcome = writer.write_season_primary_image(
                        target_dir,
                        tmdb::image_extension(poster_path),
                        &bytes,
                    )?;
                    print_nfo_outcome(&outcome.path, outcome.action);
                    record_write_action(
                        outcome.action,
                        &mut image_written,
                        &mut image_skipped_existing,
                    );
                }
            }
            Err(error) => {
                println!("跳过第 {} 季海报下载失败: {error}", season);
                image_failures += target_dirs.len();
            }
        }
    }

    for (video_path, parsed) in &parsed_files {
        let season = parsed
            .season_number
            .expect("collect_nfo_candidates ensures season");
        let Some(episode) = episode_lookup.get(&(season, parsed.episode_number)) else {
            println!(
                "跳过缺少 TMDB 剧集元数据的文件: {}",
                video_path.file_name().unwrap().to_string_lossy()
            );
            missing_metadata += 1;
            continue;
        };

        let (external_ids_result, credits_result) = tokio::join!(
            client.get_episode_external_ids(show_id, season, parsed.episode_number),
            client.get_episode_credits(show_id, season, parsed.episode_number, &args.language)
        );

        let external_ids = match external_ids_result {
            Ok(value) => Some(value),
            Err(error) => {
                println!(
                    "跳过单集外部 ID 增强: {} ({error})",
                    video_path.file_name().unwrap().to_string_lossy()
                );
                metadata_enrichment_failures += 1;
                None
            }
        };

        let credits = match credits_result {
            Ok(value) => Some(value),
            Err(error) => {
                println!(
                    "跳过单集演职员增强: {} ({error})",
                    video_path.file_name().unwrap().to_string_lossy()
                );
                metadata_enrichment_failures += 1;
                None
            }
        };

        let episode_nfo = build_episode_nfo(
            &details.name,
            season,
            parsed.episode_number,
            episode,
            external_ids.as_ref(),
            credits.as_ref(),
        );
        let outcome = writer.write_episode(video_path, &episode_nfo)?;
        print_nfo_outcome(&outcome.path, outcome.action);
        record_write_action(outcome.action, &mut nfo_written, &mut nfo_skipped_existing);

        if let Some(still_path) = episode.still_path.as_deref() {
            match client.download_image(still_path).await {
                Ok(bytes) => {
                    let outcome = writer.write_episode_thumb_image(
                        video_path,
                        tmdb::image_extension(still_path),
                        &bytes,
                    )?;
                    print_nfo_outcome(&outcome.path, outcome.action);
                    record_write_action(
                        outcome.action,
                        &mut image_written,
                        &mut image_skipped_existing,
                    );
                }
                Err(error) => {
                    println!(
                        "跳过单集缩略图下载失败: {} ({error})",
                        video_path.file_name().unwrap().to_string_lossy()
                    );
                    image_failures += 1;
                }
            }
        } else {
            missing_images += 1;
        }
    }

    println!("\nNFO 导出摘要:");
    println!("  NFO 计划/成功写入: {}", nfo_written);
    if nfo_skipped_existing > 0 {
        println!("  NFO 已跳过已有文件: {}", nfo_skipped_existing);
    }
    println!("  图片计划/成功写入: {}", image_written);
    if image_skipped_existing > 0 {
        println!("  图片已跳过已有文件: {}", image_skipped_existing);
    }
    if missing_metadata > 0 {
        println!("  缺少剧集元数据: {}", missing_metadata);
    }
    if missing_images > 0 {
        println!("  缺少图片源数据: {}", missing_images);
    }
    if image_failures > 0 {
        println!("  图片下载失败: {}", image_failures);
    }
    if metadata_enrichment_failures > 0 {
        println!("  单集增强信息获取失败: {}", metadata_enrichment_failures);
    }
    if args.dry_run {
        println!("  当前为预览模式，未实际写入文件");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Nfo(args)) => handle_nfo_export(&args).await,
        None => run_rename(&RenameArgs::try_from(cli.rename)?).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
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

    fn make_season(season_number: u32, episode_count: u32) -> tmdb::Season {
        tmdb::Season {
            season_number,
            episode_count,
            name: format!("Season {}", season_number),
            poster_path: None,
        }
    }

    fn make_tv_details(networks: Vec<&str>, production_companies: Vec<&str>) -> TvDetails {
        TvDetails {
            id: 123,
            name: "Show".to_string(),
            original_name: "Show".to_string(),
            poster_path: None,
            backdrop_path: None,
            overview: Some("Overview".to_string()),
            first_air_date: Some("2024-01-01".to_string()),
            status: Some("Ended".to_string()),
            vote_average: 8.2,
            vote_count: 10,
            number_of_seasons: 1,
            seasons: vec![make_season(1, 12)],
            genres: vec![tmdb::NamedValue {
                name: "Animation".to_string(),
            }],
            networks: networks
                .into_iter()
                .map(|name| tmdb::NamedValue {
                    name: name.to_string(),
                })
                .collect(),
            production_companies: production_companies
                .into_iter()
                .map(|name| tmdb::NamedValue {
                    name: name.to_string(),
                })
                .collect(),
        }
    }

    fn make_season_details(season_number: u32, poster_path: Option<&str>) -> SeasonDetails {
        SeasonDetails {
            season_number,
            poster_path: poster_path.map(str::to_string),
            episodes: Vec::new(),
        }
    }

    #[test]
    fn test_cli_parses_default_rename_command() {
        let cli = Cli::try_parse_from(["anime_renamer", "/tmp/show"]).unwrap();

        assert!(cli.command.is_none());
        assert_eq!(cli.rename.path.as_deref(), Some("/tmp/show"));
    }

    #[test]
    fn test_cli_parses_nfo_subcommand() {
        let cli = Cli::try_parse_from(["anime_renamer", "nfo", "/tmp/show", "--force"]).unwrap();

        match cli.command {
            Some(Command::Nfo(args)) => {
                assert_eq!(args.path, "/tmp/show");
                assert!(args.force);
            }
            None => panic!("应当解析为 nfo 子命令"),
        }
    }

    #[test]
    fn test_cli_help_lists_nfo_subcommand() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("nfo"));
        assert!(help.contains("anime_renamer nfo"));
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

    #[test]
    fn test_collect_nfo_candidates_only_keeps_formatted_files() {
        let parser = FileParser::new();
        let files = vec![
            PathBuf::from("/tmp/Show S01E01.mkv"),
            PathBuf::from("/tmp/Show 02.mkv"),
        ];

        let parsed = collect_nfo_candidates(&files, &parser);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1.anime_name, "Show");
        assert_eq!(parsed[0].1.season_number, Some(1));
    }

    #[test]
    fn test_collect_nfo_candidates_keeps_formatted_large_episode_numbers() {
        let parser = FileParser::new();
        let files = vec![PathBuf::from("/tmp/Show S01E120.mkv")];

        let parsed = collect_nfo_candidates(&files, &parser);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1.season_number, Some(1));
        assert_eq!(parsed[0].1.episode_number, 120);
    }

    #[test]
    fn test_collect_nfo_candidates_keeps_special_season_zero() {
        let parser = FileParser::new();
        let files = vec![PathBuf::from("/tmp/Show S00E01.mkv")];

        let parsed = collect_nfo_candidates(&files, &parser);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1.season_number, Some(0));
        assert_eq!(required_seasons_for_nfo(&parsed), BTreeSet::from([0]));
    }

    #[test]
    fn test_build_tvshow_nfo_prefers_networks_for_studio_tags() {
        let details = make_tv_details(vec!["Tokyo MX"], vec!["Studio A"]);
        let nfo = build_tvshow_nfo(&details);

        assert_eq!(nfo.tmdb_id, 123);
        assert_eq!(nfo.studios, vec!["Tokyo MX"]);
        assert_eq!(nfo.year, Some(2024));
    }

    #[test]
    fn test_build_tvshow_nfo_falls_back_to_production_companies() {
        let details = make_tv_details(Vec::new(), vec!["Studio A"]);
        let nfo = build_tvshow_nfo(&details);

        assert_eq!(nfo.studios, vec!["Studio A"]);
    }

    #[test]
    fn test_resolve_season_poster_path_prefers_season_details() {
        let mut details = make_tv_details(Vec::new(), vec!["Studio A"]);
        details.seasons = vec![tmdb::Season {
            season_number: 2,
            episode_count: 12,
            name: "Season 2".to_string(),
            poster_path: Some("/tv-details.jpg".to_string()),
        }];

        let season_details_map = HashMap::from([(2, make_season_details(2, Some("/season.jpg")))]);

        assert_eq!(
            resolve_season_poster_path(2, &season_details_map, &details),
            Some("/season.jpg")
        );
    }

    #[test]
    fn test_resolve_season_poster_path_falls_back_to_tv_details() {
        let mut details = make_tv_details(Vec::new(), vec!["Studio A"]);
        details.seasons = vec![tmdb::Season {
            season_number: 2,
            episode_count: 12,
            name: "Season 2".to_string(),
            poster_path: Some("/tv-details.jpg".to_string()),
        }];

        let season_details_map = HashMap::from([(2, make_season_details(2, None))]);

        assert_eq!(
            resolve_season_poster_path(2, &season_details_map, &details),
            Some("/tv-details.jpg")
        );
    }

    #[test]
    fn test_build_episode_nfo_uses_tmdb_episode_metadata() {
        let episode = Episode {
            id: 999,
            episode_number: 3,
            name: "Episode 3".to_string(),
            still_path: Some("/still.jpg".to_string()),
            air_date: Some("2024-01-15".to_string()),
            overview: Some("Overview".to_string()),
            vote_average: 7.8,
            vote_count: 11,
        };
        let external_ids = EpisodeExternalIds {
            imdb_id: Some("tt1234567".to_string()),
            tvdb_id: Some(42),
        };
        let credits = EpisodeCredits {
            cast: vec![tmdb::CastMember {
                id: 1,
                name: "Actor".to_string(),
                character: Some("Hero".to_string()),
                profile_path: None,
            }],
            crew: vec![
                tmdb::CrewMember {
                    id: 2,
                    name: "Writer".to_string(),
                    department: Some("Writing".to_string()),
                    job: Some("Writer".to_string()),
                    profile_path: None,
                },
                tmdb::CrewMember {
                    id: 3,
                    name: "Director".to_string(),
                    department: Some("Directing".to_string()),
                    job: Some("Director".to_string()),
                    profile_path: None,
                },
            ],
            guest_stars: vec![tmdb::CastMember {
                id: 4,
                name: "Guest".to_string(),
                character: Some("Guest Role".to_string()),
                profile_path: None,
            }],
        };

        let nfo = build_episode_nfo("Show", 1, 3, &episode, Some(&external_ids), Some(&credits));

        assert_eq!(nfo.title, "Episode 3");
        assert_eq!(nfo.showtitle, "Show");
        assert_eq!(nfo.season, 1);
        assert_eq!(nfo.episode, 3);
        assert_eq!(nfo.premiered.as_deref(), Some("2024-01-15"));
        assert_eq!(nfo.unique_ids[0].id_type, "tmdb");
        assert_eq!(nfo.unique_ids[0].is_default, false);
        assert_eq!(nfo.unique_ids[1].value, "tt1234567");
        assert_eq!(nfo.unique_ids[1].is_default, true);
        assert_eq!(nfo.unique_ids[2].value, "42");
        assert_eq!(nfo.credits[0].name, "Writer");
        assert_eq!(nfo.directors[0].name, "Director");
        assert_eq!(nfo.actors.len(), 2);
        assert_eq!(nfo.actors[1].actor_type.as_deref(), Some("GuestStar"));
    }

    #[test]
    fn test_extract_year_parses_first_four_digits() {
        assert_eq!(extract_year(Some("2024-06-30")), Some(2024));
        assert_eq!(extract_year(Some("bad")), None);
        assert_eq!(extract_year(None), None);
    }

    #[test]
    fn test_file_scanner_and_nfo_candidates_support_recursive_season_folders() {
        let dir = TestDir::new("nfo_recursive");
        let season_dir = dir.path().join("Season 1");
        fs::create_dir_all(&season_dir).unwrap();
        fs::write(season_dir.join("Show S01E01.mkv"), b"video").unwrap();
        fs::write(season_dir.join("Show 02.mkv"), b"video").unwrap();

        let scanner = FileScanner::new(true);
        let files = scanner.scan(dir.path().to_str().unwrap());
        let parser = FileParser::new();
        let parsed = collect_nfo_candidates(&files, &parser);

        assert_eq!(parsed.len(), 1);
        assert_eq!(required_seasons_for_nfo(&parsed), BTreeSet::from([1]));
    }

    #[test]
    fn test_season_image_targets_only_uses_single_season_directories() {
        let entries = vec![
            (
                PathBuf::from("/tmp/Show/Season 01/Show S01E01.mkv"),
                ParsedFile {
                    anime_name: "Show".to_string(),
                    episode_number: 1,
                    season_number: Some(1),
                    episode_type: EpisodeType::Normal,
                    tags: Vec::new(),
                    extension: "mkv".to_string(),
                    is_already_formatted: true,
                },
            ),
            (
                PathBuf::from("/tmp/Show/Season 02/Show S02E01.mkv"),
                ParsedFile {
                    anime_name: "Show".to_string(),
                    episode_number: 1,
                    season_number: Some(2),
                    episode_type: EpisodeType::Normal,
                    tags: Vec::new(),
                    extension: "mkv".to_string(),
                    is_already_formatted: true,
                },
            ),
            (
                PathBuf::from("/tmp/Show/Mixed/Show S01E02.mkv"),
                ParsedFile {
                    anime_name: "Show".to_string(),
                    episode_number: 2,
                    season_number: Some(1),
                    episode_type: EpisodeType::Normal,
                    tags: Vec::new(),
                    extension: "mkv".to_string(),
                    is_already_formatted: true,
                },
            ),
            (
                PathBuf::from("/tmp/Show/Mixed/Show S02E02.mkv"),
                ParsedFile {
                    anime_name: "Show".to_string(),
                    episode_number: 2,
                    season_number: Some(2),
                    episode_type: EpisodeType::Normal,
                    tags: Vec::new(),
                    extension: "mkv".to_string(),
                    is_already_formatted: true,
                },
            ),
        ];

        let targets = season_image_targets(&entries);

        assert_eq!(
            targets.get(&1).unwrap(),
            &vec![PathBuf::from("/tmp/Show/Season 01")]
        );
        assert_eq!(
            targets.get(&2).unwrap(),
            &vec![PathBuf::from("/tmp/Show/Season 02")]
        );
    }
}
