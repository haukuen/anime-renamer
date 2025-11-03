# anime_renamer

番剧自动化重命名工具 - 一键批量改名，对接 TMDB，方便媒体服务器自动刮削。
> 当前为 MVP 版本，支持基础的文件扫描和 TMDB 对接功能。

## 功能特点

- 自动识别番剧文件名并提取集数
- 对接 TMDB API 获取正确的剧集信息
- 自动处理多季番剧的集数映射
- 支持预览模式，安全可靠

## 使用方法

```bash
anime_renamer /path/to/anime/folder

# 预览模式（推荐先使用）
anime_renamer /path/to/anime/folder --dry-run

# 递归扫描子目录
anime_renamer /path/to/anime/folder --recursive

# 保留文件标签
anime_renamer /path/to/anime/folder --keep-tags
```

## 使用示例

**输入文件：**
```
[LoliHouse] 孤独搖滾！- 01 [WebRip 1080p].mkv
鬼灭之刃 27.mkv
```

**输出文件：**
```
孤独搖滾！ S01E01.mkv
鬼灭之刃 S02E01.mkv
```

## 支持的视频格式

mkv, mp4, avi, flv, rmvb, mov

## 许可证

MIT
