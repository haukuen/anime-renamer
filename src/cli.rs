use anyhow::{Context, Result};
use clap::{Args, Parser as ClapParser, Subcommand};

const ROOT_HELP: &str = "\
用法:
  anime_renamer [OPTIONS] [PATH]
  anime_renamer nfo [OPTIONS] <PATH>

参数:
  [PATH]  要扫描的目录路径

选项:
  -r, --recursive            是否递归扫描子目录
  -n, --dry-run              预览模式（不实际重命名）
      --name <NAME>          指定番剧名称（跳过自动识别）
  -l, --language <LANGUAGE>  语言偏好 [默认: zh-CN]
      --keep-tags            保留所有标签
      --season-folders       为每一季创建单独的文件夹（Season 1, Season 2, ...）
      --use-anilist          使用 AniList API 而不是 TMDB（更好的罗马音支持）
  -s, --season <SEASON>      手动指定季度（跳过自动映射）
  -o, --offset <OFFSET>      集数偏移量 [默认: 0]
  -i, --tmdb-id <TMDB_ID>    直接指定 TMDB ID
  -h, --help                 显示帮助信息
  -V, --version              显示版本信息

子命令:
  nfo                        导出 Kodi / Jellyfin NFO 与图片元数据
";

const NFO_HELP: &str = "\
用法:
  anime_renamer nfo [OPTIONS] <PATH>

参数:
  <PATH>  要扫描的目录路径

选项:
  -r, --recursive            是否递归扫描子目录
  -n, --dry-run              预览模式（不实际写入）
  -l, --language <LANGUAGE>  语言偏好 [默认: zh-CN]
  -i, --tmdb-id <TMDB_ID>    直接指定 TMDB ID
      --force                覆盖已有 NFO 文件
  -h, --help                 显示帮助信息
";

#[derive(ClapParser, Debug, Clone)]
#[command(author, version, about, long_about = None, override_help = ROOT_HELP)]
#[command(
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    flatten_help = true,
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    #[command(flatten)]
    pub(crate) rename: RenameCliArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum Command {
    #[command(override_help = NFO_HELP, about = "导出 Kodi / Jellyfin NFO 与图片元数据")]
    Nfo(NfoArgs),
}

#[derive(Args, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub(crate) struct RenameCliArgs {
    /// 要扫描的目录路径
    pub(crate) path: Option<String>,

    /// 是否递归扫描子目录
    #[arg(short, long)]
    pub(crate) recursive: bool,

    /// 预览模式（不实际重命名）
    #[arg(short = 'n', long)]
    pub(crate) dry_run: bool,

    /// 指定番剧名称（跳过自动识别）
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// 语言偏好
    #[arg(short, long, default_value = "zh-CN")]
    pub(crate) language: String,

    /// 保留所有标签
    #[arg(long)]
    pub(crate) keep_tags: bool,

    /// 为每一季创建单独的文件夹（Season 1, Season 2, ...）
    #[arg(long)]
    pub(crate) season_folders: bool,

    /// 使用 AniList API 而不是 TMDB（更好的罗马音支持）
    #[arg(long)]
    pub(crate) use_anilist: bool,

    /// 手动指定季度（跳过自动映射）
    #[arg(short, long)]
    pub(crate) season: Option<u32>,

    /// 集数偏移量
    #[arg(short, long, default_value = "0", allow_hyphen_values = true)]
    pub(crate) offset: i32,

    /// 直接指定 TMDB ID
    #[arg(short = 'i', long)]
    pub(crate) tmdb_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct RenameArgs {
    pub(crate) path: String,
    pub(crate) recursive: bool,
    pub(crate) dry_run: bool,
    pub(crate) name: Option<String>,
    pub(crate) language: String,
    pub(crate) keep_tags: bool,
    pub(crate) season_folders: bool,
    pub(crate) use_anilist: bool,
    pub(crate) season: Option<u32>,
    pub(crate) offset: i32,
    pub(crate) tmdb_id: Option<u32>,
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
pub(crate) struct NfoArgs {
    /// 要扫描的目录路径
    pub(crate) path: String,

    /// 是否递归扫描子目录
    #[arg(short, long)]
    pub(crate) recursive: bool,

    /// 预览模式（不实际写入）
    #[arg(short = 'n', long)]
    pub(crate) dry_run: bool,

    /// 语言偏好
    #[arg(short, long, default_value = "zh-CN")]
    pub(crate) language: String,

    /// 直接指定 TMDB ID
    #[arg(short = 'i', long)]
    pub(crate) tmdb_id: Option<u32>,

    /// 覆盖已有 NFO 文件
    #[arg(long)]
    pub(crate) force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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
        assert!(help.contains("用法:"));
        assert!(help.contains("显示帮助信息"));
        assert!(!help.contains("Print help"));
    }
}
