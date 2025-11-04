mod anilist;
mod parser;
mod scanner;
mod tmdb;

use anilist::AniListClient;
use anyhow::{Context, Result};
use clap::Parser as ClapParser;
use parser::{EpisodeType, FileParser, extract_tmdb_id};
use scanner::FileScanner;
use tmdb::TmdbClient;

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
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

/// 处理 AniList 模式的重命名（不依赖 TMDB 季度信息）
fn handle_anilist_renaming(
    args: &Args,
    parsed_files: &[(std::path::PathBuf, parser::ParsedFile)],
    anime_name: &str,
) -> Result<()> {
    use std::io::{self, Write};

    let mut rename_map = Vec::new();

    for (file_path, parsed) in parsed_files {
        let parent = file_path.parent().unwrap();

        // AniList 模式必须依赖文件名中的季度信息
        let season = parsed.season_number.unwrap_or(1);
        let episode = parsed.episode_number;

        let new_name = if args.keep_tags && !parsed.tags.is_empty() {
            let tags_str = parsed
                .tags
                .iter()
                .map(|tag| format!("[{}]", tag))
                .collect::<Vec<_>>()
                .join("");
            format!(
                "{} S{:02}E{:02}{}.{}",
                anime_name, season, episode, tags_str, parsed.extension
            )
        } else {
            format!(
                "{} S{:02}E{:02}.{}",
                anime_name, season, episode, parsed.extension
            )
        };

        let new_path = if args.season_folders {
            let season_folder = if season == 0 {
                "Season 0".to_string()
            } else {
                format!("Season {}", season)
            };
            parent.join(&season_folder).join(&new_name)
        } else {
            parent.join(&new_name)
        };

        rename_map.push((file_path.clone(), new_path, season, episode));
    }

    println!("重命名预览:\n");
    for (i, (old_path, new_path, season, episode)) in rename_map.iter().enumerate() {
        println!("[{}] S{:02}E{:02}", i + 1, season, episode);
        println!(
            "  原文件: {}",
            old_path.file_name().unwrap().to_str().unwrap()
        );

        if args.season_folders {
            if let Some(old_parent) = old_path.parent() {
                let relative_path = new_path.strip_prefix(old_parent).unwrap_or(new_path);
                println!("  新路径: {}\n", relative_path.display());
            } else {
                println!(
                    "  新文件: {}\n",
                    new_path.file_name().unwrap().to_str().unwrap()
                );
            }
        } else {
            println!(
                "  新文件: {}\n",
                new_path.file_name().unwrap().to_str().unwrap()
            );
        }
    }

    if args.dry_run {
        println!("预览模式，未实际重命名");
    } else {
        print!("继续重命名？[Y/n] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
            let mut success = 0;
            for (old_path, new_path, _, _) in &rename_map {
                if let Some(parent_dir) = new_path.parent()
                    && !parent_dir.exists()
                    && let Err(e) = std::fs::create_dir_all(parent_dir)
                {
                    println!("创建目录失败: {} - {}", parent_dir.display(), e);
                    continue;
                }

                if let Err(e) = std::fs::rename(old_path, new_path) {
                    println!("重命名失败: {} - {}", old_path.display(), e);
                } else {
                    success += 1;
                }
            }
            println!("\n成功重命名 {} 个文件", success);
        } else {
            println!("已取消");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!("扫描目录: {}", args.path);

    let scanner = FileScanner::new(args.recursive);
    let files = scanner.scan(&args.path);

    if files.is_empty() {
        println!("未找到视频文件");
        return Ok(());
    }

    println!("找到 {} 个视频文件\n", files.len());

    let parser = FileParser::new();
    let mut parsed_files = Vec::new();
    let mut skipped_formatted = 0;

    for file in &files {
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

    if parsed_files.is_empty() {
        println!("没有可解析的文件");
        return Ok(());
    }

    let anime_name = if let Some(ref name) = args.name {
        name.clone()
    } else {
        parsed_files[0].1.anime_name.clone()
    };

    println!("检测到番剧: {}", anime_name);

    // 检查路径中是否包含 TMDB ID
    let tmdb_id = extract_tmdb_id(&args.path);

    if let Some(id) = tmdb_id {
        println!("检测到 TMDB ID: {}, 直接使用该 ID 查询", id);
        let client = TmdbClient::new();

        let details = client
            .get_tv_details(id, &args.language)
            .await
            .context("通过 ID 获取详情失败")?;

        println!("找到匹配: {} (TMDB ID: {})", details.name, id);
        println!("共 {} 季，开始分析集数映射...\n", details.number_of_seasons);

        let normal_seasons: Vec<_> = details
            .seasons
            .iter()
            .filter(|s| s.season_number > 0)
            .cloned()
            .collect();

        let season_zero = details
            .seasons
            .iter()
            .find(|s| s.season_number == 0)
            .cloned();

        let mut rename_map = Vec::new();
        let mut special_counter = 1u32;

        for (file_path, parsed) in &parsed_files {
            let parent = file_path.parent().unwrap();

            let (season, episode) = match parsed.episode_type {
                EpisodeType::Normal => {
                    // 如果文件名中有季度信息，直接使用
                    if let Some(s) = parsed.season_number {
                        (s, parsed.episode_number)
                    } else {
                        // 否则按连续集数映射
                        match map_episode_to_season(parsed.episode_number, &normal_seasons) {
                            Some((s, e)) => (s, e),
                            None => {
                                println!("无法映射第 {} 集到任何季", parsed.episode_number);
                                continue;
                            }
                        }
                    }
                }
                EpisodeType::OVA | EpisodeType::Special => {
                    if season_zero.is_some() {
                        (0, special_counter)
                    } else {
                        (0, parsed.episode_number)
                    }
                }
                EpisodeType::Movie => {
                    println!(
                        "跳过剧场版: {}",
                        file_path.file_name().unwrap().to_str().unwrap()
                    );
                    continue;
                }
                EpisodeType::OAD => {
                    if season_zero.is_some() {
                        (0, special_counter)
                    } else {
                        (0, parsed.episode_number)
                    }
                }
            };

            if parsed.episode_type != EpisodeType::Normal {
                special_counter += 1;
            }

            let new_name = if args.keep_tags && !parsed.tags.is_empty() {
                let tags_str = parsed
                    .tags
                    .iter()
                    .map(|tag| format!("[{}]", tag))
                    .collect::<Vec<_>>()
                    .join("");
                format!(
                    "{} S{:02}E{:02}{}.{}",
                    details.name, season, episode, tags_str, parsed.extension
                )
            } else {
                format!(
                    "{} S{:02}E{:02}.{}",
                    details.name, season, episode, parsed.extension
                )
            };

            let new_path = if args.season_folders {
                let season_folder = if season == 0 {
                    "Season 0".to_string()
                } else {
                    format!("Season {}", season)
                };
                parent.join(&season_folder).join(&new_name)
            } else {
                parent.join(&new_name)
            };

            rename_map.push((file_path.clone(), new_path, season, episode));
        }

        println!("重命名预览:\n");
        for (i, (old_path, new_path, season, episode)) in rename_map.iter().enumerate() {
            println!("[{}] S{:02}E{:02}", i + 1, season, episode);
            println!(
                "  原文件: {}",
                old_path.file_name().unwrap().to_str().unwrap()
            );

            if args.season_folders {
                if let Some(old_parent) = old_path.parent() {
                    let relative_path = new_path.strip_prefix(old_parent).unwrap_or(new_path);
                    println!("  新路径: {}\n", relative_path.display());
                } else {
                    println!(
                        "  新文件: {}\n",
                        new_path.file_name().unwrap().to_str().unwrap()
                    );
                }
            } else {
                println!(
                    "  新文件: {}\n",
                    new_path.file_name().unwrap().to_str().unwrap()
                );
            }
        }

        if args.dry_run {
            println!("预览模式，未实际重命名");
        } else {
            print!("继续重命名？[Y/n] ");
            use std::io::{self, Write};
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
                let mut success = 0;
                for (old_path, new_path, _, _) in &rename_map {
                    if let Some(parent_dir) = new_path.parent()
                        && !parent_dir.exists()
                        && let Err(e) = std::fs::create_dir_all(parent_dir)
                    {
                        println!("创建目录失败: {} - {}", parent_dir.display(), e);
                        continue;
                    }

                    if let Err(e) = std::fs::rename(old_path, new_path) {
                        println!("重命名失败: {} - {}", old_path.display(), e);
                    } else {
                        success += 1;
                    }
                }
                println!("\n成功重命名 {} 个文件", success);
            } else {
                println!("已取消");
            }
        }

        return Ok(());
    }

    // 尝试 TMDB
    let client = TmdbClient::new();
    println!("搜索 TMDB...");

    let results = client
        .search_tv(&anime_name, &args.language)
        .await
        .context("搜索失败")?;

    // 如果 TMDB 没找到，尝试 AniList
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

        // 显示所有可用的标题选项
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
            println!("未找到可用的标题");
            return Ok(());
        }

        // 让用户选择
        use std::io::{self, Write};
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

        println!("找到匹配: {} ({})", display_name, anime.format_date());

        println!("\n注意: AniList 不提供季度信息，将使用文件名中的季度标记");
        println!("如果文件名没有季度标记（如 'V', 'Season 5'），可能会映射错误\n");

        handle_anilist_renaming(&args, &parsed_files, &display_name)?;
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

    let normal_seasons: Vec<_> = details
        .seasons
        .iter()
        .filter(|s| s.season_number > 0)
        .cloned()
        .collect();

    let season_zero = details
        .seasons
        .iter()
        .find(|s| s.season_number == 0)
        .cloned();

    let mut rename_map = Vec::new();
    let mut special_counter = 1u32;

    for (file_path, parsed) in &parsed_files {
        let parent = file_path.parent().unwrap();

        let (season, episode) = match parsed.episode_type {
            EpisodeType::Normal => {
                // 如果文件名中有季度信息，直接使用
                if let Some(s) = parsed.season_number {
                    (s, parsed.episode_number)
                } else {
                    // 否则按连续集数映射
                    match map_episode_to_season(parsed.episode_number, &normal_seasons) {
                        Some((s, e)) => (s, e),
                        None => {
                            println!("无法映射第 {} 集到任何季", parsed.episode_number);
                            continue;
                        }
                    }
                }
            }
            EpisodeType::OVA | EpisodeType::Special => {
                if season_zero.is_some() {
                    (0, special_counter)
                } else {
                    (0, parsed.episode_number)
                }
            }
            EpisodeType::Movie => {
                println!(
                    "跳过剧场版: {}",
                    file_path.file_name().unwrap().to_str().unwrap()
                );
                continue;
            }
            EpisodeType::OAD => {
                if season_zero.is_some() {
                    (0, special_counter)
                } else {
                    (0, parsed.episode_number)
                }
            }
        };

        if parsed.episode_type != EpisodeType::Normal {
            special_counter += 1;
        }

        let new_name = if args.keep_tags && !parsed.tags.is_empty() {
            let tags_str = parsed
                .tags
                .iter()
                .map(|tag| format!("[{}]", tag))
                .collect::<Vec<_>>()
                .join("");
            format!(
                "{} S{:02}E{:02}{}.{}",
                tv_show.name, season, episode, tags_str, parsed.extension
            )
        } else {
            format!(
                "{} S{:02}E{:02}.{}",
                tv_show.name, season, episode, parsed.extension
            )
        };

        let new_path = if args.season_folders {
            let season_folder = if season == 0 {
                "Season 0".to_string()
            } else {
                format!("Season {}", season)
            };
            parent.join(&season_folder).join(&new_name)
        } else {
            parent.join(&new_name)
        };

        rename_map.push((file_path.clone(), new_path, season, episode));
    }

    println!("重命名预览:\n");
    for (i, (old_path, new_path, season, episode)) in rename_map.iter().enumerate() {
        println!("[{}] S{:02}E{:02}", i + 1, season, episode);
        println!(
            "  原文件: {}",
            old_path.file_name().unwrap().to_str().unwrap()
        );

        if args.season_folders {
            if let Some(old_parent) = old_path.parent() {
                let relative_path = new_path.strip_prefix(old_parent).unwrap_or(new_path);
                println!("  新路径: {}\n", relative_path.display());
            } else {
                println!(
                    "  新文件: {}\n",
                    new_path.file_name().unwrap().to_str().unwrap()
                );
            }
        } else {
            println!(
                "  新文件: {}\n",
                new_path.file_name().unwrap().to_str().unwrap()
            );
        }
    }

    if args.dry_run {
        println!("预览模式，未实际重命名");
    } else {
        print!("继续重命名？[Y/n] ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
            let mut success = 0;
            for (old_path, new_path, _, _) in &rename_map {
                if let Some(parent_dir) = new_path.parent()
                    && !parent_dir.exists()
                    && let Err(e) = std::fs::create_dir_all(parent_dir)
                {
                    println!("创建目录失败: {} - {}", parent_dir.display(), e);
                    continue;
                }

                if let Err(e) = std::fs::rename(old_path, new_path) {
                    println!("重命名失败: {} - {}", old_path.display(), e);
                } else {
                    success += 1;
                }
            }
            println!("\n成功重命名 {} 个文件", success);
        } else {
            println!("已取消");
        }
    }

    Ok(())
}
