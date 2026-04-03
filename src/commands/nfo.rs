use crate::cli::NfoArgs;
use crate::nfo::{
    ActorNfo, EpisodeNfo, NfoWriter, PersonNfo, Rating, SeasonNfo, TvShowNfo, UniqueId,
    WriteAction, episode_nfo_path, episode_thumb_image_path, season_nfo_path,
    season_primary_image_path, tvshow_backdrop_image_path, tvshow_primary_image_path,
};
use crate::parser::{EpisodeType, FileParser, ParsedFile, extract_tmdb_id};
use crate::scanner::FileScanner;
use crate::tmdb::{
    self, Episode, EpisodeCredits, EpisodeExternalIds, SeasonDetails, TmdbClient, TvDetails,
};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::task::JoinSet;

type ParsedEntry = (PathBuf, ParsedFile);
const MAX_CONCURRENT_EPISODE_EXPORTS: usize = tmdb::MAX_CONCURRENT_REQUESTS;

fn file_name_lossy(path: &Path) -> Option<String> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
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
    let language = language.to_string();
    let mut season_details = HashMap::new();
    let mut failed_seasons = Vec::new();
    let mut tasks = JoinSet::new();

    for season in seasons {
        let client = client.clone();
        let language = language.clone();
        let season_number = *season;
        tasks.spawn(async move {
            (
                season_number,
                client
                    .get_season_details(tv_id, season_number, &language)
                    .await,
            )
        });
    }

    while let Some(result) = tasks.join_next().await {
        let (season, details_result) = result.context("季度详情任务执行失败")?;
        match details_result {
            Ok(details) => {
                season_details.insert(details.season_number, details);
            }
            Err(error) => {
                println!("跳过第 {} 季元数据: {error}", season);
                failed_seasons.push(season);
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

fn should_write_path(path: &Path, force: bool) -> bool {
    force || !path.exists()
}

fn print_skipped_existing(path: &Path, written: &mut usize, skipped_existing: &mut usize) {
    print_nfo_outcome(path, WriteAction::SkippedExisting);
    record_write_action(WriteAction::SkippedExisting, written, skipped_existing);
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

fn build_season_nfo(details: &SeasonDetails) -> SeasonNfo {
    SeasonNfo {
        title: details.name.clone(),
        plot: details
            .overview
            .clone()
            .filter(|value| !value.trim().is_empty()),
        premiered: details.air_date.clone(),
        aired: details.air_date.clone(),
        season: details.season_number,
        unique_ids: vec![UniqueId {
            id_type: "tmdb".to_string(),
            value: details.id.to_string(),
            is_default: true,
        }],
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

#[derive(Default)]
struct EpisodeExportStats {
    nfo_written: usize,
    nfo_skipped_existing: usize,
    image_written: usize,
    image_skipped_existing: usize,
    missing_images: usize,
    image_failures: usize,
    missing_metadata: usize,
    metadata_enrichment_failures: usize,
}

impl EpisodeExportStats {
    fn record_nfo_action(&mut self, action: WriteAction) {
        record_write_action(
            action,
            &mut self.nfo_written,
            &mut self.nfo_skipped_existing,
        );
    }

    fn record_image_action(&mut self, action: WriteAction) {
        record_write_action(
            action,
            &mut self.image_written,
            &mut self.image_skipped_existing,
        );
    }
}

struct EpisodeExportJob {
    client: TmdbClient,
    writer: NfoWriter,
    force: bool,
    language: String,
    show_id: u32,
    show_title: String,
    video_path: PathBuf,
    parsed: ParsedFile,
    episode: Option<Episode>,
}

impl EpisodeExportJob {
    async fn run(self) -> Result<EpisodeExportStats> {
        let Self {
            client,
            writer,
            force,
            language,
            show_id,
            show_title,
            video_path,
            parsed,
            episode,
        } = self;

        let mut stats = EpisodeExportStats::default();
        let season = parsed
            .season_number
            .expect("collect_nfo_candidates ensures season");

        let Some(episode) = episode else {
            println!(
                "跳过缺少 TMDB 剧集元数据的文件: {}",
                video_path.file_name().unwrap().to_string_lossy()
            );
            stats.missing_metadata += 1;
            return Ok(stats);
        };

        let episode_nfo_target = episode_nfo_path(&video_path);
        if should_write_path(&episode_nfo_target, force) {
            // Keep one TMDB request in flight per episode job. Running both calls at once causes
            // the shared request semaphore to complete work in small bursts (for example 4 episodes
            // at a time with an 8-request limit), which makes NFO output look "stuck" between batches.
            let external_ids = match client
                .get_episode_external_ids(show_id, season, parsed.episode_number)
                .await
            {
                Ok(value) => Some(value),
                Err(error) => {
                    println!(
                        "跳过单集外部 ID 增强: {} ({error})",
                        video_path.file_name().unwrap().to_string_lossy()
                    );
                    stats.metadata_enrichment_failures += 1;
                    None
                }
            };

            let credits = match client
                .get_episode_credits(show_id, season, parsed.episode_number, &language)
                .await
            {
                Ok(value) => Some(value),
                Err(error) => {
                    println!(
                        "跳过单集演职员增强: {} ({error})",
                        video_path.file_name().unwrap().to_string_lossy()
                    );
                    stats.metadata_enrichment_failures += 1;
                    None
                }
            };

            let episode_nfo = build_episode_nfo(
                &show_title,
                season,
                parsed.episode_number,
                &episode,
                external_ids.as_ref(),
                credits.as_ref(),
            );
            let outcome = writer.write_episode(&video_path, &episode_nfo)?;
            print_nfo_outcome(&outcome.path, outcome.action);
            stats.record_nfo_action(outcome.action);
        } else {
            print_nfo_outcome(&episode_nfo_target, WriteAction::SkippedExisting);
            stats.record_nfo_action(WriteAction::SkippedExisting);
        }

        if let Some(still_path) = episode.still_path.as_deref() {
            let extension = tmdb::image_extension(still_path);
            let target_path = episode_thumb_image_path(&video_path, extension);
            if should_write_path(&target_path, force) {
                match client.download_image(still_path).await {
                    Ok(bytes) => {
                        let outcome =
                            writer.write_episode_thumb_image(&video_path, extension, &bytes)?;
                        print_nfo_outcome(&outcome.path, outcome.action);
                        stats.record_image_action(outcome.action);
                    }
                    Err(error) => {
                        println!(
                            "跳过单集缩略图下载失败: {} ({error})",
                            video_path.file_name().unwrap().to_string_lossy()
                        );
                        stats.image_failures += 1;
                    }
                }
            } else {
                print_nfo_outcome(&target_path, WriteAction::SkippedExisting);
                stats.record_image_action(WriteAction::SkippedExisting);
            }
        } else {
            stats.missing_images += 1;
        }

        Ok(stats)
    }
}

fn spawn_episode_export_job(
    episode_tasks: &mut JoinSet<Result<EpisodeExportStats>>,
    client: &TmdbClient,
    writer: NfoWriter,
    force: bool,
    language: &str,
    show_id: u32,
    show_title: &str,
    video_path: &Path,
    parsed: &ParsedFile,
    episode: Option<Episode>,
) {
    let client = client.clone();
    let language = language.to_string();
    let show_title = show_title.to_string();
    let video_path = video_path.to_path_buf();
    let parsed = parsed.clone();

    episode_tasks.spawn(async move {
        EpisodeExportJob {
            client,
            writer,
            force,
            language,
            show_id,
            show_title,
            video_path,
            parsed,
            episode,
        }
        .run()
        .await
    });
}

pub(crate) async fn run(args: &NfoArgs) -> Result<()> {
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
    let force = args.force;

    let mut nfo_written = 0;
    let mut nfo_skipped_existing = 0;
    let root = Path::new(&args.path);
    let tvshow_nfo_path = root.join("tvshow.nfo");
    if should_write_path(&tvshow_nfo_path, args.force) {
        let tvshow_outcome = writer.write_tvshow(root, &build_tvshow_nfo(&details))?;
        print_nfo_outcome(&tvshow_outcome.path, tvshow_outcome.action);
        record_write_action(
            tvshow_outcome.action,
            &mut nfo_written,
            &mut nfo_skipped_existing,
        );
    } else {
        print_skipped_existing(
            &tvshow_nfo_path,
            &mut nfo_written,
            &mut nfo_skipped_existing,
        );
    }

    let mut image_written = 0;
    let mut image_skipped_existing = 0;
    let mut missing_images = 0;
    let mut image_failures = 0;
    let mut missing_metadata = 0;
    let mut metadata_enrichment_failures = 0;

    if let Some(poster_path) = details.poster_path.as_deref() {
        let extension = tmdb::image_extension(poster_path);
        let target_path = tvshow_primary_image_path(root, extension);
        if should_write_path(&target_path, args.force) {
            match client.download_image(poster_path).await {
                Ok(bytes) => {
                    let outcome = writer.write_tvshow_primary_image(root, extension, &bytes)?;
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
            print_skipped_existing(
                &target_path,
                &mut image_written,
                &mut image_skipped_existing,
            );
        }
    } else {
        missing_images += 1;
    }

    if let Some(backdrop_path) = details.backdrop_path.as_deref() {
        let extension = tmdb::image_extension(backdrop_path);
        let target_path = tvshow_backdrop_image_path(root, extension);
        if should_write_path(&target_path, args.force) {
            match client.download_image(backdrop_path).await {
                Ok(bytes) => {
                    let outcome = writer.write_tvshow_backdrop_image(root, extension, &bytes)?;
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
            print_skipped_existing(
                &target_path,
                &mut image_written,
                &mut image_skipped_existing,
            );
        }
    } else {
        missing_images += 1;
    }

    for season in &seasons {
        let Some(target_dirs) = season_targets.get(season) else {
            continue;
        };

        // Write season.nfo for each season directory
        if let Some(season_detail) = season_details_map.get(season) {
            let season_nfo = build_season_nfo(season_detail);
            for target_dir in target_dirs {
                let nfo_target = season_nfo_path(target_dir);
                if should_write_path(&nfo_target, args.force) {
                    let outcome = writer.write_season(target_dir, &season_nfo)?;
                    print_nfo_outcome(&outcome.path, outcome.action);
                    record_write_action(
                        outcome.action,
                        &mut nfo_written,
                        &mut nfo_skipped_existing,
                    );
                } else {
                    print_skipped_existing(
                        &nfo_target,
                        &mut nfo_written,
                        &mut nfo_skipped_existing,
                    );
                }
            }
        }

        let Some(poster_path) = resolve_season_poster_path(*season, &season_details_map, &details)
        else {
            missing_images += target_dirs.len();
            continue;
        };

        let extension = tmdb::image_extension(poster_path);
        let mut pending_dirs = Vec::new();
        for target_dir in target_dirs {
            let target_path = season_primary_image_path(target_dir, extension);
            if should_write_path(&target_path, args.force) {
                pending_dirs.push(target_dir);
            } else {
                print_skipped_existing(
                    &target_path,
                    &mut image_written,
                    &mut image_skipped_existing,
                );
            }
        }

        if pending_dirs.is_empty() {
            continue;
        }

        match client.download_image(poster_path).await {
            Ok(bytes) => {
                for target_dir in pending_dirs {
                    let outcome =
                        writer.write_season_primary_image(target_dir, extension, &bytes)?;
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
                image_failures += pending_dirs.len();
            }
        }
    }

    let mut episode_tasks = JoinSet::new();
    let mut parsed_iter = parsed_files.iter();

    while episode_tasks.len() < MAX_CONCURRENT_EPISODE_EXPORTS {
        let Some((video_path, parsed)) = parsed_iter.next() else {
            break;
        };
        let season = parsed
            .season_number
            .expect("collect_nfo_candidates ensures season");
        let episode = episode_lookup
            .get(&(season, parsed.episode_number))
            .cloned();
        spawn_episode_export_job(
            &mut episode_tasks,
            &client,
            writer,
            force,
            &args.language,
            show_id,
            &details.name,
            video_path,
            parsed,
            episode,
        );
    }

    while let Some(result) = episode_tasks.join_next().await {
        let stats = result.context("单集导出任务执行失败")??;
        nfo_written += stats.nfo_written;
        nfo_skipped_existing += stats.nfo_skipped_existing;
        image_written += stats.image_written;
        image_skipped_existing += stats.image_skipped_existing;
        missing_images += stats.missing_images;
        image_failures += stats.image_failures;
        missing_metadata += stats.missing_metadata;
        metadata_enrichment_failures += stats.metadata_enrichment_failures;

        if let Some((video_path, parsed)) = parsed_iter.next() {
            let season = parsed
                .season_number
                .expect("collect_nfo_candidates ensures season");
            let episode = episode_lookup
                .get(&(season, parsed.episode_number))
                .cloned();
            spawn_episode_export_job(
                &mut episode_tasks,
                &client,
                writer,
                force,
                &args.language,
                show_id,
                &details.name,
                video_path,
                parsed,
                episode,
            );
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
            id: 0,
            name: format!("Season {}", season_number),
            season_number,
            overview: None,
            air_date: None,
            poster_path: poster_path.map(str::to_string),
            episodes: Vec::new(),
        }
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
