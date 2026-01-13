# anime_renamer

番剧自动化重命名工具 - 一键批量改名，对接 TMDB / AniList，方便媒体服务器自动刮削。

## 功能特点

- 自动识别番剧文件名并提取集数
- 对接 TMDB / AniList API 获取正确的剧集信息
- 自动处理多季番剧的集数映射
- 支持 OVA / SP / OAD 等特殊类型
- 自动重命名关联的字幕文件
- 支持预览模式，安全可靠

## 安装

从 [Releases](https://github.com/your-repo/anime_renamer/releases) 下载对应平台的二进制文件，或使用 Cargo 编译：

```bash
cargo build --release
```

## 使用方法

```bash
anime_renamer [OPTIONS] <PATH>
```

### 参数说明

| 参数 | 短参数 | 说明 | 默认值 |
|------|--------|------|--------|
| `--recursive` | `-r` | 递归扫描子目录 | - |
| `--dry-run` | `-n` | 预览模式（不实际重命名） | - |
| `--name <NAME>` | - | 指定番剧名称（跳过自动识别） | - |
| `--language <LANG>` | `-l` | 语言偏好 | `zh-CN` |
| `--keep-tags` | - | 保留文件名中的标签（如 `[1080p]`） | - |
| `--season-folders` | - | 为每季创建单独文件夹 | - |
| `--use-anilist` | - | 使用 AniList API（更好的罗马音支持） | - |
| `--season <N>` | `-s` | 手动指定季度（跳过自动映射） | - |
| `--offset <N>` | `-o` | 集数偏移量（正数增加，负数减少） | `0` |

### 常用示例

```bash
# 基本用法
anime_renamer /path/to/anime/folder

# 预览模式（推荐先使用）
anime_renamer /path/to/anime/folder --dry-run

# 递归扫描子目录
anime_renamer /path/to/anime/folder -r

# 保留文件标签
anime_renamer /path/to/anime/folder --keep-tags

# 手动指定季度和集数偏移（适用于续作）
anime_renamer /path/to/anime/folder --season 2 --offset -12

# 使用 AniList 并指定罗马音名称
anime_renamer /path/to/anime/folder --use-anilist

# 为每季创建单独文件夹
anime_renamer /path/to/anime/folder --season-folders
```

## 使用示例

**输入文件：**
```
[LoliHouse] 孤独搖滾！- 01 [WebRip 1080p].mkv
鬼灭之刃 27.mkv
进击的巨人 OVA 01.mkv
```

**输出文件：**
```
孤独搖滾！ S01E01.mkv
鬼灭之刃 S02E01.mkv
进击的巨人 S00E01.mkv
```

## TMDB ID 支持

如果文件夹名包含 `[tmdbid=12345]` 格式，将直接使用该 ID 查询，跳过搜索步骤。

## 支持的格式

**视频：** mkv, mp4, avi, flv, rmvb, mov

**字幕：** ass, srt, ssa, sub, idx, vtt

## 许可证

MIT
