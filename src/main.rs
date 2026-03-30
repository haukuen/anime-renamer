mod anilist;
mod nfo;
mod parser;
mod scanner;
mod tmdb;

use anilist::AniListClient;
use anyhow::{Context, Result, bail};
use clap::Parser as ClapParser;
use nfo::{EpisodeNfo, NfoWriter, Rating, TvShowNfo, UniqueId, WriteAction};
use parser::{EpisodeType, FileParser, ParsedFile, extract_tmdb_id};
use scanner::FileScanner;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tmdb::{Episode, TmdbClient, TvDetails};

type ParsedEntry = (PathBuf, ParsedFile);
type RenameEntry = (PathBuf, PathBuf, u32, u32);

#[derive(ClapParser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct RenameArgs {
    /// 要扫描的目录路径
    path: String,

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

#[derive(ClapParser, Debug, Clone)]
#[command(
    name = "anime_renamer nfo",
    author,
    version,
    about = "导出 Kodi / Jellyfin 兼容的 NFO 文件",
    long_about = None
)]
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

fn print_rename_preview(rename_map: &[RenameEntry], season_folders: bool) {
    println!("重命名预览:\n");
    for (i, (old_path, new_path, season, episode)) in rename_map.iter().enumerate() {
        println!("[{}] S{:02}E{:02}", i + 1, season, episode);
        println!(
            "  原文件: {}",
            old_path.file_name().unwrap().to_str().unwrap()
        );

        if season_folders {
            if let Some(old_parent) = old_path.parent() {
                let relative_path = new_path.strip_prefix(old_parent).unwrap_or(new_path);
                println!("  新路径: {}", relative_path.display());
            } else {
                println!(
                    "  新文件: {}",
                    new_path.file_name().unwrap().to_str().unwrap()
                );
            }
        } else {
            println!(
                "  新文件: {}",
                new_path.file_name().unwrap().to_str().unwrap()
            );
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
        let filename = file.file_name().unwrap().to_str().unwrap();
        if let Some(parsed) = parser.parse(filename) {
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
        let filename = file.file_name().unwrap().to_str().unwrap();
        match parser.parse(filename) {
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

    let client = TmdbClient::new();
    println!("搜索 TMDB...");

    let results = client
        .search_tv(&anime_name, &args.language)
        .await
        .context("搜索失败")?;

    if results.is_empty() || args.use_anilist {
        if args.use_anilist {
            println!("按参数要求使用 AniList...");
        } else {
            println!("TMDB 未找到结果，尝试 AniList...");
        }

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
                    println!(
                        "跳过剧场版: {}",
                        file_path.file_name().unwrap().to_str().unwrap()
                    );
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

async fn fetch_episode_lookup(
    client: &TmdbClient,
    tv_id: u32,
    seasons: &BTreeSet<u32>,
    language: &str,
) -> Result<HashMap<(u32, u32), Episode>> {
    let mut episodes = HashMap::new();

    for season in seasons {
        let details = client
            .get_season_details(tv_id, *season, language)
            .await
            .with_context(|| format!("获取第 {} 季详情失败", season))?;

        for episode in details.episodes {
            episodes.insert((details.season_number, episode.episode_number), episode);
        }
    }

    Ok(episodes)
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
        aired: episode.air_date.clone(),
        rating: build_rating(episode.vote_average, episode.vote_count),
        unique_ids: vec![UniqueId {
            id_type: "tmdb".to_string(),
            value: episode.id.to_string(),
            is_default: true,
        }],
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
    let episode_lookup = fetch_episode_lookup(&client, show_id, &seasons, &args.language).await?;
    let writer = NfoWriter::new(args.dry_run, args.force);

    let tvshow_outcome = writer.write_tvshow(Path::new(&args.path), &build_tvshow_nfo(&details))?;
    print_nfo_outcome(&tvshow_outcome.path, tvshow_outcome.action);

    let mut written = 0;
    let mut skipped_existing = usize::from(tvshow_outcome.action == WriteAction::SkippedExisting);
    let mut missing_metadata = 0;

    if matches!(
        tvshow_outcome.action,
        WriteAction::Written | WriteAction::WouldWrite
    ) {
        written += 1;
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

        let episode_nfo = build_episode_nfo(&details.name, season, parsed.episode_number, episode);
        let outcome = writer.write_episode(video_path, &episode_nfo)?;
        print_nfo_outcome(&outcome.path, outcome.action);

        match outcome.action {
            WriteAction::WouldWrite | WriteAction::Written => written += 1,
            WriteAction::SkippedExisting => skipped_existing += 1,
        }
    }

    println!("\nNFO 导出摘要:");
    println!("  计划/成功写入: {}", written);
    if skipped_existing > 0 {
        println!("  已跳过已有文件: {}", skipped_existing);
    }
    if missing_metadata > 0 {
        println!("  缺少剧集元数据: {}", missing_metadata);
    }
    if args.dry_run {
        println!("  当前为预览模式，未实际写入文件");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let raw_args: Vec<OsString> = std::env::args_os().collect();

    if raw_args.get(1).and_then(|arg| arg.to_str()) == Some("nfo") {
        let mut nfo_args = vec![raw_args[0].clone()];
        nfo_args.extend(raw_args.into_iter().skip(2));
        handle_nfo_export(&NfoArgs::parse_from(nfo_args)).await
    } else {
        run_rename(&RenameArgs::parse()).await
    }
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

    fn make_season(season_number: u32, episode_count: u32) -> tmdb::Season {
        tmdb::Season {
            season_number,
            episode_count,
            name: format!("Season {}", season_number),
        }
    }

    fn make_tv_details(networks: Vec<&str>, production_companies: Vec<&str>) -> TvDetails {
        TvDetails {
            id: 123,
            name: "Show".to_string(),
            original_name: "Show".to_string(),
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
    fn test_build_episode_nfo_uses_tmdb_episode_metadata() {
        let episode = Episode {
            id: 999,
            episode_number: 3,
            name: "Episode 3".to_string(),
            air_date: Some("2024-01-15".to_string()),
            overview: Some("Overview".to_string()),
            vote_average: 7.8,
            vote_count: 11,
        };

        let nfo = build_episode_nfo("Show", 1, 3, &episode);

        assert_eq!(nfo.title, "Episode 3");
        assert_eq!(nfo.showtitle, "Show");
        assert_eq!(nfo.season, 1);
        assert_eq!(nfo.episode, 3);
        assert_eq!(nfo.unique_ids[0].value, "999");
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
}
