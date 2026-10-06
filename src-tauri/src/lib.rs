mod metadata;

use metadata::{Cover, SongMetadata};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent};

#[derive(Debug, Serialize)]
struct AppError(String);

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(error: tauri::Error) -> Self {
        Self(error.to_string())
    }
}

type AppResult<T> = Result<T, AppError>;

#[derive(Default)]
struct MediaScope {
    allowed: Mutex<Vec<PathBuf>>,
}

impl MediaScope {
    fn allow_file(&self, path: &Path) -> AppResult<()> {
        let canonical = fs::canonicalize(path)?;
        let mut allowed = self
            .allowed
            .lock()
            .map_err(|_| AppError("媒体文件状态已损坏".into()))?;
        if !allowed.contains(&canonical) {
            allowed.push(canonical);
        }
        Ok(())
    }

    fn allow_directory(&self, path: &Path) -> AppResult<()> {
        let canonical = fs::canonicalize(path)?;
        let mut allowed = self
            .allowed
            .lock()
            .map_err(|_| AppError("媒体目录状态已损坏".into()))?;
        if !allowed.contains(&canonical) {
            allowed.push(canonical);
        }
        Ok(())
    }

    fn contains(&self, path: &Path) -> bool {
        let Ok(canonical) = fs::canonicalize(path) else {
            return false;
        };
        self.allowed
            .lock()
            .map(|allowed| allowed.iter().any(|root| canonical.starts_with(root)))
            .unwrap_or(false)
    }
}

#[derive(Serialize)]
struct AppPaths {
    download_dir: String,
    cache_dir: String,
}

#[derive(Serialize)]
struct ImportedSong {
    title: String,
    artist: String,
    album: String,
    duration: u64,
    file_path: String,
    original_path: String,
    cover_path: Option<String>,
    lyrics: String,
    lyrics_path: Option<String>,
    source_hash: String,
}

#[derive(Serialize)]
struct ImportOutcome {
    songs: Vec<ImportedSong>,
    errors: Vec<String>,
}

// 程序所在目录：缓存与下载默认放在程序当前目录下的 cache / download
fn program_directory() -> AppResult<PathBuf> {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            return Ok(directory.to_path_buf());
        }
    }
    std::env::current_dir().map_err(Into::into)
}

fn default_paths() -> AppResult<AppPaths> {
    let base = program_directory()?;
    let download_dir = base.join("download");
    let cache_dir = base.join("cache");
    ensure_directory(&download_dir)?;
    ensure_directory(&cache_dir)?;
    Ok(AppPaths {
        download_dir: path_string(&download_dir),
        cache_dir: path_string(&cache_dir),
    })
}

fn ensure_directory(path: &Path) -> AppResult<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read_count = file.read(&mut buffer)?;
        if read_count == 0 {
            break;
        }
        hasher.update(&buffer[..read_count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn allow_media_directory(app: &tauri::AppHandle, path: &Path) -> AppResult<()> {
    app.state::<MediaScope>().allow_directory(path)?;
    Ok(())
}

fn decode_uri_path(raw: &str) -> PathBuf {
    let raw = raw.strip_prefix('/').unwrap_or(raw);
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&raw[index + 1..index + 3], 16) {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&decoded).into_owned())
}

fn media_mime(path: &Path) -> &'static str {
    if let Some(extension) = detect_audio_extension(path) {
        return match extension {
            "mp3" => "audio/mpeg",
            "flac" => "audio/flac",
            "wav" => "audio/wav",
            "m4a" => "audio/mp4",
            "aac" => "audio/aac",
            "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        };
    }
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "image/jpeg",
    }
}

fn media_range(value: &str, length: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        length.checked_sub(1)?
    } else {
        end.parse::<u64>().ok()?
    };
    if start >= length || end < start {
        return None;
    }
    Some((start, end.min(length - 1)))
}

fn media_response(
    app: &tauri::AppHandle,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let path = decode_uri_path(request.uri().path());
    if !app.state::<MediaScope>().contains(&path) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }

    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(Vec::new())
                .unwrap_or_else(|_| Response::new(Vec::new()));
        }
    };
    let Ok(metadata) = file.metadata() else {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    };
    let length = metadata.len();
    let range_header = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());

    let range = match range_header {
        Some(value) => match media_range(value, length) {
            Some(range) => Some(range),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{length}"))
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(Vec::new())
                    .unwrap_or_else(|_| Response::new(Vec::new()));
            }
        },
        None => None,
    };

    let (start, end) = range.unwrap_or((0, length.saturating_sub(1)));
    let mut body = Vec::with_capacity((end - start + 1) as usize);
    if length > 0 {
        if let Err(_) = file.rewind().and_then(|_| file.seek(std::io::SeekFrom::Start(start))) {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(Vec::new())
                .unwrap_or_else(|_| Response::new(Vec::new()));
        }
        if let Err(_) = (&mut file).take(end - start + 1).read_to_end(&mut body) {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(Vec::new())
                .unwrap_or_else(|_| Response::new(Vec::new()));
        }
    }

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, media_mime(&path))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if range.is_some() {
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{length}"));
    }
    builder
        .header(header::CONTENT_LENGTH, body.len())
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

fn unique_copy_path(directory: &Path, source: &Path) -> PathBuf {
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("audio.mp3");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    let mut candidate = directory.join(file_name);
    let mut counter = 1;
    while candidate.exists() {
        let suffix = if extension.is_empty() {
            format!(" ({counter})")
        } else {
            format!(" ({counter}).{extension}")
        };
        candidate = directory.join(format!("{stem}{suffix}"));
        counter += 1;
    }
    candidate
}

fn cache_base_name(title: &str, artist: &str) -> String {
    let title = title.trim();
    let artist = artist.trim();
    let value = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{title}-{artist}")
    };
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                character
            }
        })
        .collect();
    let sanitized = sanitized.trim().trim_end_matches(['.', ' ']);
    if sanitized.is_empty() {
        "未知歌曲-未知歌手".to_string()
    } else {
        sanitized.to_string()
    }
}

fn save_cover(cache_dir: &Path, cover: &Cover, base_name: &str) -> Option<String> {
    let cover_path = unique_copy_path(
        cache_dir,
        Path::new(&format!("{base_name}.{}", cover.extension)),
    );
    let mut file = File::create(&cover_path).ok()?;
    file.write_all(&cover.data).ok()?;
    Some(path_string(&cover_path))
}

fn is_supported_audio(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "oga"
    )
}

fn detect_audio_extension(path: &Path) -> Option<&'static str> {
    let mut header = [0u8; 16];
    let mut file = File::open(path).ok()?;
    let count = file.read(&mut header).ok()?;
    if count < 4 {
        return None;
    }
    let bytes = &header[..count];
    if count >= 7 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0 {
        return Some("aac");
    }
    if bytes.starts_with(b"ID3") || (bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) {
        return Some("mp3");
    }
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }
    if count >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return Some("wav");
    }
    if count >= 8 && &bytes[4..8] == b"ftyp" {
        return Some("m4a");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    None
}

#[tauri::command]
fn get_app_paths(app: tauri::AppHandle) -> AppResult<AppPaths> {
    let paths = default_paths()?;
    allow_media_directory(&app, Path::new(&paths.download_dir))?;
    allow_media_directory(&app, Path::new(&paths.cache_dir))?;
    Ok(paths)
}

#[tauri::command]
fn set_app_paths(
    app: tauri::AppHandle,
    download_dir: String,
    cache_dir: String,
) -> AppResult<AppPaths> {
    let download = PathBuf::from(download_dir);
    let cache = PathBuf::from(cache_dir);
    if download.as_os_str().is_empty() || cache.as_os_str().is_empty() {
        return Err(AppError("目录不能为空".into()));
    }
    ensure_directory(&download)?;
    ensure_directory(&cache)?;
    allow_media_directory(&app, &download)?;
    allow_media_directory(&app, &cache)?;
    Ok(AppPaths {
        download_dir: path_string(&download),
        cache_dir: path_string(&cache),
    })
}

#[tauri::command]
fn allow_media_files(app: tauri::AppHandle, paths: Vec<String>) -> AppResult<()> {
    let scope = app.state::<MediaScope>();
    for path in paths {
        let file = PathBuf::from(path);
        if file.is_file() {
            scope.allow_file(&file)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn import_songs(
    app: tauri::AppHandle,
    download_dir: String,
    cache_dir: String,
    source_paths: Vec<String>,
    existing_hashes: Vec<String>,
) -> AppResult<ImportOutcome> {
    let download = PathBuf::from(download_dir);
    let cache = PathBuf::from(cache_dir);
    ensure_directory(&download)?;
    ensure_directory(&cache)?;
    allow_media_directory(&app, &download)?;
    allow_media_directory(&app, &cache)?;
    let mut songs = Vec::new();
    let mut errors = Vec::new();
    let mut known_hashes: HashSet<String> = existing_hashes
        .into_iter()
        .filter(|hash| !hash.is_empty())
        .collect();
    for source_text in source_paths {
        let source = PathBuf::from(&source_text);
        if !source.exists() {
            errors.push(format!("文件不存在：{source_text}"));
            continue;
        }
        if !is_supported_audio(&source) {
            errors.push(format!("不支持的音频格式：{source_text}"));
            continue;
        }
        let source_hash = match sha256_file(&source) {
            Ok(hash) => hash,
            Err(error) => {
                errors.push(format!("计算文件哈希失败：{source_text}（{error}）"));
                continue;
            }
        };
        if known_hashes.contains(&source_hash) {
            errors.push(format!("音源重复，已跳过：{source_text}"));
            continue;
        }
        let destination = unique_copy_path(&download, &source);
        if let Err(error) = fs::copy(&source, &destination) {
            errors.push(format!("复制失败：{source_text}（{error}）"));
            continue;
        }
        let parsed = SongMetadata::from_path(&source);
        let base_name = cache_base_name(&parsed.title, &parsed.artist);
        let cover_path = parsed
            .cover
            .as_ref()
            .and_then(|cover| save_cover(&cache, cover, &base_name));
        songs.push(ImportedSong {
            title: parsed.title,
            artist: parsed.artist,
            album: parsed.album,
            duration: parsed.duration_ms,
            file_path: path_string(&destination),
            original_path: source_text,
            cover_path,
            lyrics: String::new(),
            lyrics_path: None,
            source_hash: source_hash.clone(),
        });
        known_hashes.insert(source_hash);
    }
    Ok(ImportOutcome { songs, errors })
}

fn escape_ps_single_quote(value: &str) -> String {
    value.replace('\r', " ").replace('\n', " ").replace('\'', "''")
}

fn json_u64(value: Option<&serde_json::Value>) -> u64 {
    match value {
        Some(serde_json::Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(text)) => text.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

// 从各平台返回的字段推算该曲目支持的音质（高 → 低）
fn song_qualities(song: &serde_json::Value) -> Vec<String> {
    let mut levels = Vec::new();

    // 网易云（cloudsearch 的 privilege）
    if let Some(privilege) = song.get("privilege") {
        let max_br = json_u64(privilege.get("playMaxbr").or_else(|| privilege.get("maxbr")));
        let level = privilege
            .get("maxBrLevel")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if max_br >= 1_900_000 || level == "hires" {
            levels.push(4u8);
        }
        if max_br >= 999_000 || level == "lossless" || levels.contains(&4) {
            levels.push(3);
        }
        if max_br >= 320_000 {
            levels.push(2);
        }
        if max_br >= 128_000 {
            levels.push(1);
        }
    }

    // 酷狗（各音质文件大小）
    let kugou = |field: &str| json_u64(song.get(field));
    if kugou("ResFileSize") > 0 || kugou("SuperFileSize") > 0 {
        levels.push(4);
    }
    if kugou("SQFileSize") > 0 || levels.contains(&4) {
        levels.push(3);
    }
    if kugou("HQFileSize") > 0 {
        levels.push(2);
    }
    if kugou("FileSize") > 0 {
        levels.push(1);
    }

    // 酷我（MINFO 形如 level:ff,bitrate:2000,format:flac;...）
    if let Some(minfo) = song.get("MINFO").and_then(|value| value.as_str()) {
        for entry in minfo.split(';') {
            let bitrate = entry
                .split(',')
                .find_map(|part| part.strip_prefix("bitrate:"))
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0);
            if bitrate == 0 {
                continue;
            }
            let format = entry
                .split(',')
                .find_map(|part| part.strip_prefix("format:"))
                .unwrap_or("")
                .trim();
            if format.eq_ignore_ascii_case("flac") && bitrate >= 1000 {
                levels.push(3);
            } else if bitrate >= 1000 {
                levels.push(4);
            } else if bitrate >= 320 {
                levels.push(2);
            } else if bitrate >= 100 {
                levels.push(1);
            }
        }
    }

    // QQ 音乐（各音质文件大小字段）
    if json_u64(song.get("sizehires")) > 0 {
        levels.push(4);
    }
    if json_u64(song.get("sizeflac")) > 0 || json_u64(song.get("sizeape")) > 0 {
        levels.push(3);
    }
    if json_u64(song.get("size320")) > 0 {
        levels.push(2);
    }
    if json_u64(song.get("size128")) > 0 {
        levels.push(1);
    }

    levels.sort_unstable();
    levels.dedup();
    levels.reverse();
    let mut qualities: Vec<String> = levels
        .into_iter()
        .map(|level| match level {
            4 => "flac24bit",
            3 => "flac",
            2 => "320k",
            _ => "128k",
        })
        .map(str::to_string)
        .collect();
    if qualities.is_empty() {
        qualities.push("320k".to_string());
        qualities.push("128k".to_string());
    }
    qualities
}

fn song_list_mut(value: &mut serde_json::Value) -> Option<&mut Vec<serde_json::Value>> {
    for pointer in ["/result/songs", "/playlist/tracks", "/result/playlist/tracks", "/tracks"] {
        if value.pointer(pointer).map(|node| node.is_array()).unwrap_or(false) {
            return value.pointer_mut(pointer).and_then(|node| node.as_array_mut());
        }
    }
    None
}

// 给搜索结果 / 歌单曲目补充 qualities 字段
fn attach_qualities(raw: &str, extra_privileges: Option<&serde_json::Value>) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let privileges = extra_privileges
        .cloned()
        .or_else(|| value.get("playlist").and_then(|playlist| playlist.get("privileges")).cloned());
    let privilege_by_id: std::collections::HashMap<String, serde_json::Value> = privileges
        .as_ref()
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").map(|value| value.to_string()).unwrap_or_default();
                    if id.is_empty() {
                        None
                    } else {
                        Some((id, item.clone()))
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if let Some(songs) = song_list_mut(&mut value) {
        for song in songs.iter_mut() {
            let mut probe = song.clone();
            if probe.get("privilege").is_none() {
                if let Some(id) = song.get("id") {
                    if let Some(privilege) = privilege_by_id.get(&id.to_string()) {
                        if let Some(object) = probe.as_object_mut() {
                            object.insert("privilege".into(), privilege.clone());
                        }
                    }
                }
            }
            if let Some(object) = song.as_object_mut() {
                object.insert("qualities".into(), serde_json::json!(song_qualities(&probe)));
            }
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
}

fn platform_search_script(
    platform: &str,
    keyword: &str,
    source_id: &str,
    page: u8,
    limit: u8,
) -> AppResult<String> {
    let keyword = escape_ps_single_quote(keyword);
    let source_id = escape_ps_single_quote(source_id);
    match platform {
        "tencent" => Ok(format!(
            r#"
$ProgressPreference = 'SilentlyContinue'
$keyword = '{}'
$encoded = [uri]::EscapeDataString($keyword)
$uri = "https://c.y.qq.com/soso/fcgi-bin/client_search_cp?w=$encoded&p=$({}+1)&n={}&format=json"
$response = Invoke-RestMethod -Uri $uri -Headers @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://y.qq.com/' }} -TimeoutSec 20
$songs = @()
foreach ($item in @($response.data.song.list)) {{
  $singer = if ($item.singer -is [array]) {{ (@($item.singer | ForEach-Object {{ $_.name }}) -join ' / ') }} elseif ($item.singer.name) {{ [string]$item.singer.name }} else {{ [string]$item.singer }}
  $songs += [ordered]@{{ id = [string]$item.songmid; songmid = [string]$item.songmid; name = [string]$item.songname; singer = $singer; albumName = [string]$item.albumname; duration = ([int]$item.interval * 1000); size128 = [int]$item.size128; size320 = [int]$item.size320; sizeflac = [int]$item.sizeflac; sizeape = [int]$item.sizeape; sizehires = [int]$item.sizehires; platform = 'tencent'; sourceId = '{}' }}
}}
$payload = [ordered]@{{ result = [ordered]@{{ songs = $songs; songCount = [int]$response.data.song.totalnum; playlists = @(); playlistCount = 0 }} }}
ConvertTo-Json -InputObject $payload -Depth 8 -Compress
"#,
            keyword, page, limit, source_id
        )),
        "kugou" => Ok(format!(
            r#"
$ProgressPreference = 'SilentlyContinue'
$keyword = '{}'
$encoded = [uri]::EscapeDataString($keyword)
$uri = "http://songsearch.kugou.com/song_search_v2?platform=AndroidFilter&iscorrection=1&keyword=$encoded&hifiquality=0&pagesize={}&PrivilegeFilter=0&page=$({}+1)"
$response = Invoke-RestMethod -Uri $uri -Headers @{{ 'User-Agent' = 'Mozilla/5.0' }} -TimeoutSec 20
$songs = @()
foreach ($item in @($response.data.lists)) {{
  $singer = (@($item.Singers | ForEach-Object {{ $_.name }}) -join ' / ')
  $songs += [ordered]@{{ id = [string]$item.Audioid; songmid = [string]$item.Audioid; hash = [string]$item.FileHash; albumId = [string]$item.AlbumID; name = [string]$item.OriSongName; singer = $singer; albumName = [string]$item.AlbumName; duration = ([int]$item.Duration * 1000); FileSize = [int64]$item.FileSize; HQFileSize = [int64]$item.HQFileSize; SQFileSize = [int64]$item.SQFileSize; ResFileSize = [int64]$item.ResFileSize; SuperFileSize = [int64]$item.SuperFileSize; platform = 'kugou'; sourceId = '{}' }}
}}
$payload = [ordered]@{{ result = [ordered]@{{ songs = $songs; songCount = [int]$response.data.total; playlists = @(); playlistCount = 0 }} }}
ConvertTo-Json -InputObject $payload -Depth 8 -Compress
"#,
            keyword, limit, page, source_id
        )),
        "kuwo" => Ok(format!(
            r#"
$ProgressPreference = 'SilentlyContinue'
$keyword = '{}'
$encoded = [uri]::EscapeDataString($keyword)
$uri = "http://search.kuwo.cn/r.s?client=kt&all=$encoded&pn={}&rn={}&uid=794762570&ver=kwplayer_ar_9.2.2.1&vipver=1&show_copyright_off=1&newver=1&ft=music&cluster=0&strategy=2012&encoding=utf8&rformat=json&vermerge=1&mobi=1&issubtitle=1"
$response = Invoke-RestMethod -Uri $uri -Headers @{{ 'User-Agent' = 'Mozilla/5.0' }} -TimeoutSec 20
$songs = @()
foreach ($item in @($response.abslist)) {{
  $id = ([string]$item.MUSICRID -replace '^MUSIC_', '')
  $songs += [ordered]@{{ id = $id; songmid = $id; name = [string]$item.SONGNAME; singer = [string]$item.ARTIST; albumName = [string]$item.ALBUM; duration = ([int]$item.DURATION * 1000); MINFO = [string]$item.MINFO; platform = 'kuwo'; sourceId = '{}' }}
}}
$payload = [ordered]@{{ result = [ordered]@{{ songs = $songs; songCount = [int]$response.TOTAL; playlists = @(); playlistCount = 0 }} }}
ConvertTo-Json -InputObject $payload -Depth 8 -Compress
"#,
            keyword, page, limit, source_id
        )),
        _ => Err(AppError("不支持的搜索来源".into())),
    }
}

#[tauri::command]
fn search_music(
    keyword: String,
    source_id: String,
    kind: u16,
    page: u8,
    limit: u8,
    platform: Option<String>,
) -> AppResult<String> {
    let platform = platform.unwrap_or_else(|| "netease".to_string());
    if kind != 1 && platform != "netease" {
        return Err(AppError("该来源暂不支持歌单搜索".into()));
    }
    if kind == 1 && platform != "netease" {
        let script = platform_search_script(&platform, &keyword, &source_id, page, limit)?;
        let lines = run_powershell(&script)?;
        return Ok(attach_qualities(&lines.join(""), None));
    }
    // 用 cloudsearch 接口，响应里带 privilege（可推断每首歌支持的音质）
    const SEARCH_PREFIX: &str = concat!(
        "\x68\x74\x74\x70\x73\x3a\x2f\x2f\x6d\x75\x73\x69\x63\x2e\x31\x36\x33\x2e\x63\x6f\x6d",
        "\x2f\x61\x70\x69\x2f\x63\x6c\x6f\x75\x64\x73\x65\x61\x72\x63\x68\x2f\x70\x63\x3f",
        "\x73\x3d"
    );
    let offset = page.saturating_mul(limit);
    let script = format!(
        r#"
$keyword = '{}'
$uri = "{}" + [uri]::EscapeDataString($keyword) + "&type={}&offset={}&limit={}"
$response = Invoke-RestMethod -Uri $uri -Headers @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://music.163.com/' }} -TimeoutSec 20
ConvertTo-Json -InputObject $response -Depth 14 -Compress
"#,
        escape_ps_single_quote(&keyword),
        SEARCH_PREFIX,
        kind,
        offset,
        limit
    );
    let lines = run_powershell(&script)?;
    Ok(attach_qualities(&lines.join(""), None))
}

#[tauri::command]
fn get_playlist(
    playlist_id: String,
    source_id: String,
    offset: Option<u64>,
    limit: Option<u64>,
) -> AppResult<String> {
    let _source_id = source_id;
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(1000).max(1);
    let playlist_id = escape_ps_single_quote(&playlist_id);
    let script = format!(
        r#"
$ProgressPreference = 'SilentlyContinue'
$detail = Invoke-RestMethod -Uri 'https://music.163.com/api/v6/playlist/detail?id={}' -Headers @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://music.163.com/' }} -TimeoutSec 20
$playlist = $detail.playlist
$trackIds = @($playlist.trackIds | ForEach-Object {{ $_.id }})
if ($trackIds.Count -eq 0) {{ $trackIds = @($playlist.tracks | ForEach-Object {{ $_.id }}) }}
$start = [Math]::Min({}, $trackIds.Count)
$end = [Math]::Min($start + {}, $trackIds.Count)
$pageTracks = @()
if ($end -gt $start) {{
  $pageIds = @($trackIds[$start..($end - 1)])
  for ($index = 0; $index -lt $pageIds.Count; $index += 50) {{
    $lastIndex = [Math]::Min($index + 49, $pageIds.Count - 1)
    $chunk = @($pageIds[$index..$lastIndex])
    $idJson = '[' + ($chunk -join ',') + ']'
    $songResponse = Invoke-RestMethod -Uri ('https://music.163.com/api/song/detail?ids=' + [uri]::EscapeDataString($idJson)) -Headers @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://music.163.com/' }} -TimeoutSec 20
    $pageTracks += @($songResponse.songs)
  }}
}}
$result = [ordered]@{{ playlist = [ordered]@{{ id = $playlist.id; name = $playlist.name; description = $playlist.description; trackCount = $playlist.trackCount; coverImgUrl = $playlist.coverImgUrl; creator = $playlist.creator; tracks = $pageTracks; privileges = $detail.privileges; offset = {}; total = $trackIds.Count }} }}
ConvertTo-Json -InputObject $result -Depth 16 -Compress
"#,
        playlist_id, offset, limit, offset
    );
    let lines = run_powershell(&script)?;
    Ok(attach_qualities(&lines.join(""), None))
}

#[derive(Serialize, serde::Deserialize)]
struct CloudMedia {
    url: String,
    cover_url: String,
    lyrics: String,
    title: String,
    artist: String,
    album: String,
    duration: u64,
}

fn quality_bitrate(quality: &str) -> u32 {
    match quality {
        "128k" => 128_000,
        "flac" => 999_000,
        "flac24bit" => 999_000,
        _ => 320_000,
    }
}

#[tauri::command]
fn get_cloud_media(
    song_id: String,
    source_id: Option<String>,
    quality: String,
    fallback_url: Option<String>,
) -> AppResult<CloudMedia> {
    let _source_id = source_id;
    let song_id = song_id.trim().to_string();
    if song_id.is_empty() {
        return Err(AppError("云库歌曲缺少音源 ID".into()));
    }
    let bitrate = quality_bitrate(&quality);
    let escaped_id = escape_ps_single_quote(&song_id);
    let script = format!(
        r#"
$ProgressPreference = 'SilentlyContinue'
$headers = @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://music.163.com/'; 'Origin' = 'https://music.163.com' }}
$id = '{escaped_id}'
$encodedId = [uri]::EscapeDataString($id)
$result = [ordered]@{{ url = ''; cover_url = ''; lyrics = ''; title = ''; artist = ''; album = ''; duration = 0 }}
try {{
  $detail = Invoke-RestMethod -Uri ('https://music.163.com/api/song/detail?ids=' + [uri]::EscapeDataString('[' + $id + ']')) -Headers $headers -TimeoutSec 20
  $song = $detail.songs[0]
  if ($song) {{
    $result.title = [string]$song.name
    $result.album = [string]$song.album.name
    $result.duration = [int64]$song.duration
    $result.cover_url = [string]$song.album.picUrl
    $artists = @($song.artists | ForEach-Object {{ $_.name }}) -join ' / '
    $result.artist = $artists
  }}
}} catch {{}}
try {{
  $player = Invoke-RestMethod -Uri ('https://music.163.com/api/song/enhance/player/url?id=' + $encodedId + '&ids=' + [uri]::EscapeDataString('[' + $id + ']') + '&br={bitrate}') -Headers $headers -TimeoutSec 20
  $result.url = [string]$player.data[0].url
}} catch {{}}
try {{
  $lyric = Invoke-RestMethod -Uri ('https://music.163.com/api/song/lyric?id=' + $encodedId + '&lv=-1&kv=-1&tv=-1') -Headers $headers -TimeoutSec 20
  $result.lyrics = [string]$lyric.lrc.lyric
}} catch {{}}
ConvertTo-Json -InputObject $result -Depth 8 -Compress
"#,
        escaped_id = escaped_id,
        bitrate = bitrate
    );
    let lines = run_powershell(&script)?;
    let raw = lines.join("");
    let mut media: CloudMedia = serde_json::from_str(&raw)
        .map_err(|error| AppError(format!("解析云库歌曲信息失败：{error}")))?;
    if media.url.trim().is_empty() {
        media.url = fallback_url.unwrap_or_default();
    }
    if media.title.trim().is_empty() {
        media.title = "未知歌曲".into();
    }
    if media.artist.trim().is_empty() {
        media.artist = "未知歌手".into();
    }
    if media.album.trim().is_empty() {
        media.album = "未知专辑".into();
    }
    Ok(media)
}

fn download_remote_cover(
    app: &tauri::AppHandle,
    url: &str,
    cache_dir: &Path,
    base_name: &str,
) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    let clean_url = url.split('?').next().unwrap_or(url);
    let extension = Path::new(clean_url)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| matches!(value.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
        .unwrap_or("jpg");
    let destination = unique_copy_path(cache_dir, Path::new(&format!("{base_name}.{extension}")));
    let script = format!(
        r#"
$uri = '{}'
$destination = '{}'
$headers = @{{ 'User-Agent' = 'Mozilla/5.0'; 'Referer' = 'https://music.163.com/' }}
try {{
  Invoke-WebRequest -Uri $uri -Headers $headers -OutFile $destination -UseBasicParsing -TimeoutSec 45
}} catch {{
  if (Test-Path -LiteralPath $destination) {{ Remove-Item -LiteralPath $destination -Force }}
  throw
}}
"#,
        escape_ps_single_quote(url),
        escape_ps_single_quote(&path_string(&destination))
    );
    if run_powershell(&script).is_err() || !destination.is_file() {
        return None;
    }
    let _ = allow_media_directory(app, cache_dir);
    Some(path_string(&destination))
}

#[tauri::command]
fn download_remote_song(
    app: tauri::AppHandle,
    download_dir: String,
    cache_dir: String,
    url: String,
    suggested_name: String,
    existing_hashes: Vec<String>,
    cover_url: Option<String>,
    lyrics: Option<String>,
    expected_duration: Option<u64>,
) -> AppResult<ImportedSong> {
    eprintln!("DOWNLOAD_START url_len={}", url.len());
    let download = PathBuf::from(download_dir);
    let cache = PathBuf::from(cache_dir);
    ensure_directory(&download)?;
    ensure_directory(&cache)?;
    allow_media_directory(&app, &download)?;
    allow_media_directory(&app, &cache)?;
    let (suggested_title, suggested_artist) = suggested_name
        .rsplit_once(" - ")
        .unwrap_or((suggested_name.as_str(), "未知歌手"));
    let suggested_base = cache_base_name(suggested_title, suggested_artist);
    let url_path = url.split('?').next().unwrap_or("");
    let extension = Path::new(url_path)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| is_supported_audio(Path::new(&format!("file.{value}"))))
        .unwrap_or("mp3");
    let source = PathBuf::from(format!(
        "{}.{}",
        suggested_base,
        extension
    ));
    let destination = unique_copy_path(&download, &source);
    let script = format!(
        r#"
$uri = '{}'
$destination = '{}'
try {{
  Invoke-WebRequest -Uri $uri -OutFile $destination -UseBasicParsing -TimeoutSec 90
}} catch {{
  if (Test-Path -LiteralPath $destination) {{ Remove-Item -LiteralPath $destination -Force }}
  throw
}}
"#,
        escape_ps_single_quote(&url),
        escape_ps_single_quote(&path_string(&destination))
    );
    run_powershell(&script)?;
    eprintln!("DOWNLOAD_DONE path={}", destination.display());
    let source_hash = sha256_file(&destination)?;
    if existing_hashes.iter().any(|hash| hash == &source_hash) {
        let _ = fs::remove_file(&destination);
        return Err(AppError("云端音源已在试听列表中".into()));
    }
    let parsed = SongMetadata::from_path(&destination);
    if parsed.duration_ms == 0 {
        let file_size = fs::metadata(&destination).map(|value| value.len()).unwrap_or(0);
        if file_size > 0 && expected_duration.unwrap_or(0) > 0 {
            let mut parsed = parsed;
            parsed.duration_ms = expected_duration.unwrap_or(0);
            return finish_remote_song(
                app,
                destination,
                cache,
                parsed,
                suggested_title,
                suggested_artist,
                cover_url,
                lyrics,
                source_hash,
                url,
            );
        }
        let _ = fs::remove_file(&destination);
        return Err(AppError("无法解析或播放该歌曲，未添加到试听列表；建议切换音源后重试".into()));
    }
    finish_remote_song(
        app,
        destination,
        cache,
        parsed,
        suggested_title,
        suggested_artist,
        cover_url,
        lyrics,
        source_hash,
        url,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_remote_song(
    app: tauri::AppHandle,
    mut destination: PathBuf,
    cache: PathBuf,
    parsed: SongMetadata,
    suggested_title: &str,
    suggested_artist: &str,
    cover_url: Option<String>,
    lyrics: Option<String>,
    source_hash: String,
    url: String,
) -> AppResult<ImportedSong> {
    let title = if suggested_title.trim().is_empty() || suggested_title == "未知歌曲" {
        parsed.title
    } else {
        suggested_title.to_string()
    };
    let artist = if suggested_artist.trim().is_empty() || suggested_artist == "未知歌手" {
        parsed.artist
    } else {
        suggested_artist.to_string()
    };
    let base_name = cache_base_name(&title, &artist);
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mp3")
        .to_ascii_lowercase();
    let final_file_name = format!("{base_name}.{extension}");
    if destination.file_name().and_then(|value| value.to_str()) != Some(final_file_name.as_str()) {
        let final_destination = unique_copy_path(&cache, Path::new(&final_file_name));
        fs::rename(&destination, &final_destination)?;
        destination = final_destination;
    }
    let cover_path = cover_url
        .as_deref()
        .and_then(|cover| download_remote_cover(&app, cover, &cache, &base_name))
        .or_else(|| parsed.cover.as_ref().and_then(|cover| save_cover(&cache, cover, &base_name)));
    let lyrics = lyrics.unwrap_or_default();
    let lyrics_path = if lyrics.trim().is_empty() {
        None
    } else {
        let path = unique_copy_path(&cache, Path::new(&format!("{base_name}.lrc")));
        fs::write(&path, &lyrics)?;
        Some(path_string(&path))
    };
    Ok(ImportedSong {
        title,
        artist,
        album: parsed.album,
        duration: parsed.duration_ms,
        file_path: path_string(&destination),
        original_path: url,
        cover_path,
        lyrics,
        lyrics_path,
        source_hash,
    })
}

#[tauri::command]
fn copy_song_to_download(
    app: tauri::AppHandle,
    file_path: String,
    download_dir: String,
) -> AppResult<String> {
    let source = PathBuf::from(&file_path);
    if !source.is_file() {
        return Err(AppError("歌曲文件不存在".into()));
    }
    let download = PathBuf::from(download_dir);
    ensure_directory(&download)?;
    allow_media_directory(&app, &download)?;
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .map(PathBuf::from)
        .ok_or_else(|| AppError("歌曲文件名无效".into()))?;
    let destination = unique_copy_path(&download, &file_name);
    if destination.exists() {
        let same_file = sha256_file(&destination)
            .ok()
            .and_then(|destination_hash| sha256_file(&source).ok().map(|source_hash| destination_hash == source_hash))
            .unwrap_or(false);
        if same_file {
            return Ok(path_string(&destination));
        }
    }
    fs::copy(&source, &destination)?;
    allow_media_directory(&app, &download)?;
    Ok(path_string(&destination))
}

#[tauri::command]
fn remove_cached_song_files(cache_dir: String, paths: Vec<String>) -> AppResult<()> {
    let cache = PathBuf::from(cache_dir);
    ensure_directory(&cache)?;
    let cache_root = fs::canonicalize(&cache)?;
    for path_text in paths {
        let candidate = PathBuf::from(&path_text);
        if !candidate.exists() {
            continue;
        }
        let canonical = fs::canonicalize(&candidate)?;
        if !canonical.starts_with(&cache_root) || !canonical.is_file() {
            return Err(AppError("拒绝删除缓存目录之外的文件".into()));
        }
        fs::remove_file(canonical)?;
    }
    Ok(())
}

#[tauri::command]
fn clear_unused_cache(cache_dir: String, keep_paths: Vec<String>) -> AppResult<u64> {
    let cache = PathBuf::from(&cache_dir);
    ensure_directory(&cache)?;
    let cache_root = fs::canonicalize(&cache)?;
    let keep: HashSet<PathBuf> = keep_paths
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter_map(|value| fs::canonicalize(value).ok())
        .collect();
    let Ok(entries) = fs::read_dir(&cache_root) else {
        return Ok(0);
    };
    let mut removed = 0u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else { continue };
        // 只清理缓存目录第一层的文件，跳过 music-sources 等子目录
        if !metadata.is_file() {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(entry.path()) else { continue };
        if keep.contains(&canonical) {
            continue;
        }
        if fs::remove_file(&canonical).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[derive(Serialize)]
struct MusicSource {
    id: String,
    name: String,
    version: String,
    path: String,
    hash: String,
    source_url: Option<String>,
}

fn music_source_dir(cache_dir: &Path) -> AppResult<PathBuf> {
    let directory = cache_dir.join("music-sources");
    ensure_directory(&directory)?;
    Ok(directory)
}

fn source_url_sidecar(path: &Path) -> PathBuf {
    path.with_extension("url")
}

fn read_source_url(path: &Path) -> Option<String> {
    let url = fs::read_to_string(source_url_sidecar(path)).ok()?;
    let url = url.trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

fn source_header(path: &Path, fallback: &str) -> (String, String) {
    let Ok(content) = fs::read_to_string(path) else {
        return (fallback.to_string(), "未知版本".to_string());
    };
    let header = content.chars().take(4096).collect::<String>();
    let header_value = |marker: &str| {
        header.lines().find_map(|line| {
            let index = line.find(marker)?;
            let value = line[index + marker.len()..]
                .trim()
                .trim_start_matches(':')
                .trim();
            (!value.is_empty()).then(|| value.to_string())
        })
    };
    let name = header_value("@name").unwrap_or_else(|| fallback.to_string());
    let version = header_value("@version").unwrap_or_else(|| "未知版本".to_string());
    (name, version)
}

fn music_source_from_path(path: &Path) -> AppResult<MusicSource> {
    let hash = sha256_file(path)?;
    let fallback = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("音源")
        .to_string();
    let (name, version) = source_header(path, &fallback);
    Ok(MusicSource {
        id: format!("source-{}", &hash[..12.min(hash.len())]),
        name,
        version,
        path: path_string(path),
        hash,
        source_url: read_source_url(path),
    })
}

#[tauri::command]
fn list_music_sources(cache_dir: String) -> AppResult<Vec<MusicSource>> {
    let directory = music_source_dir(Path::new(&cache_dir))?;
    let mut sources = Vec::new();
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(sources);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("js")) == Some(true) {
            if let Ok(source) = music_source_from_path(&path) {
                sources.push(source);
            }
        }
    }
    sources.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(sources)
}

#[tauri::command]
fn read_music_source(source_path: String, cache_dir: String) -> AppResult<String> {
    let directory = fs::canonicalize(music_source_dir(Path::new(&cache_dir))?)?;
    let source = fs::canonicalize(source_path)?;
    if !source.starts_with(&directory)
        || !source.is_file()
        || source.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("js")) != Some(true)
    {
        return Err(AppError("只能读取已导入的 JavaScript 音源".into()));
    }
    fs::read_to_string(source).map_err(Into::into)
}

#[tauri::command]
fn source_http_request(
    url: String,
    method: String,
    headers: Option<serde_json::Value>,
    body: Option<String>,
    timeout_secs: Option<u64>,
) -> AppResult<String> {
    let url = url.trim().to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError("音源请求地址无效".into()));
    }
    let method = method.trim().to_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS") {
        return Err(AppError("音源请求方法无效".into()));
    }
    let timeout = timeout_secs.unwrap_or(20).min(60);
    let mut header_lines = Vec::new();
    if let Some(object) = headers.as_ref().and_then(|value| value.as_object()) {
        for (key, value) in object {
            let value = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
                }
                value => value.to_string(),
            };
            header_lines.push(format!(
                "$headers['{}'] = '{}'",
                escape_ps_single_quote(key),
                escape_ps_single_quote(&value)
            ));
        }
    }
    let body_argument = body
        .as_ref()
        .filter(|value| !value.is_empty())
        .map(|value| format!("-Body '{}'", escape_ps_single_quote(value)))
        .unwrap_or_default();
    let script = format!(
        r#"
$ProgressPreference = 'SilentlyContinue'
$headers = @{{}}
{}
$response = Invoke-WebRequest -Uri '{}' -Method {} -Headers $headers -UseBasicParsing -TimeoutSec {} {}
if ($null -ne $response.RawContentStream) {{
  $stream = $response.RawContentStream
  if ($stream.CanSeek) {{ $stream.Position = 0 }}
  $buffer = New-Object byte[] ([int]$stream.Length)
  $null = $stream.Read($buffer, 0, $buffer.Length)
  [Text.Encoding]::UTF8.GetString($buffer)
}} elseif ($null -ne $response.Content) {{
  [string]$response.Content
}} else {{
  ''
}}
"#,
        header_lines.join("\n"),
        escape_ps_single_quote(&url),
        method,
        timeout,
        body_argument
    );
    let lines = run_powershell(&script)?;
    Ok(lines.join(""))
}

#[tauri::command]
fn import_music_source(source_path: String, cache_dir: String) -> AppResult<MusicSource> {
    let source = PathBuf::from(source_path);
    if !source.is_file() || source.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("js")) != Some(true) {
        return Err(AppError("请选择 JavaScript 音源文件".into()));
    }
    let directory = music_source_dir(Path::new(&cache_dir))?;
    let source_hash = sha256_file(&source)?;
    if list_music_sources(cache_dir.clone())?
        .iter()
        .any(|existing| existing.hash == source_hash)
    {
        return Err(AppError("已有相同哈希的音源，未导入".into()));
    }
    let destination = unique_copy_path(&directory, &source);
    fs::copy(&source, &destination)?;
    music_source_from_path(&destination)
}

#[tauri::command]
fn import_music_source_url(source_url: String, cache_dir: String) -> AppResult<MusicSource> {
    let source_url = source_url.trim().to_string();
    if !source_url.starts_with("http://") && !source_url.starts_with("https://") {
        return Err(AppError("请输入 http 或 https 音源地址".into()));
    }
    let directory = music_source_dir(Path::new(&cache_dir))?;
    let existing_hashes: HashSet<String> = list_music_sources(cache_dir.clone())?
        .into_iter()
        .map(|source| source.hash)
        .collect();
    let url_path = source_url.split('?').next().unwrap_or("");
    let file_name = if Path::new(url_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("js"))
        == Some(true)
    {
        Path::new(url_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("source.js")
    } else {
        "downloaded-source.js"
    };
    let destination = unique_copy_path(&directory, Path::new(file_name));
    let script = format!(
        r#"
$uri = '{}'
$destination = '{}'
try {{
  Invoke-WebRequest -Uri $uri -OutFile $destination -UseBasicParsing -TimeoutSec 60
}} catch {{
  if (Test-Path -LiteralPath $destination) {{ Remove-Item -LiteralPath $destination -Force }}
  throw
}}
"#,
        escape_ps_single_quote(&source_url),
        escape_ps_single_quote(&path_string(&destination))
    );
    run_powershell(&script)?;
    if !destination.is_file() {
        return Err(AppError("音源下载失败".into()));
    }
    let content = fs::read_to_string(&destination).unwrap_or_default();
    if content.trim().is_empty() || content.to_ascii_lowercase().contains("<html") {
        let _ = fs::remove_file(&destination);
        return Err(AppError("下载内容不是有效的 JavaScript 音源".into()));
    }
    let downloaded_hash = sha256_file(&destination)?;
    if existing_hashes.contains(&downloaded_hash) {
        let _ = fs::remove_file(&destination);
        return Err(AppError("已有相同哈希的音源，未导入".into()));
    }
    let _ = fs::write(source_url_sidecar(&destination), &source_url);
    music_source_from_path(&destination)
}

#[tauri::command]
fn delete_music_source(source_path: String, cache_dir: String) -> AppResult<()> {
    let directory = fs::canonicalize(music_source_dir(Path::new(&cache_dir))?)?;
    let source = fs::canonicalize(source_path)?;
    if !source.starts_with(&directory)
        || !source.is_file()
        || source.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("js")) != Some(true)
    {
        return Err(AppError("只能删除已导入的音源文件".into()));
    }
    let _ = fs::remove_file(source_url_sidecar(&source));
    fs::remove_file(source)?;
    Ok(())
}

#[tauri::command]
fn refresh_music_source_url(source_path: String, cache_dir: String) -> AppResult<MusicSource> {
    let directory = fs::canonicalize(music_source_dir(Path::new(&cache_dir))?)?;
    let source = fs::canonicalize(&source_path)?;
    if !source.starts_with(&directory)
        || !source.is_file()
        || source.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("js")) != Some(true)
    {
        return Err(AppError("只能刷新已导入的 JavaScript 音源".into()));
    }
    let url = read_source_url(&source)
        .ok_or_else(|| AppError("该音源不是通过 URL 导入的，无法刷新".into()))?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError("音源 URL 无效".into()));
    }
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.js");
    let temp = directory.join(format!("{file_name}.refresh"));
    let script = format!(
        r#"
$uri = '{}'
$destination = '{}'
try {{
  Invoke-WebRequest -Uri $uri -OutFile $destination -UseBasicParsing -TimeoutSec 60
}} catch {{
  if (Test-Path -LiteralPath $destination) {{ Remove-Item -LiteralPath $destination -Force }}
  throw
}}
"#,
        escape_ps_single_quote(&url),
        escape_ps_single_quote(&path_string(&temp))
    );
    if run_powershell(&script).is_err() || !temp.is_file() {
        let _ = fs::remove_file(&temp);
        return Err(AppError("刷新音源失败：下载未成功".into()));
    }
    let content = fs::read_to_string(&temp).unwrap_or_default();
    if content.trim().is_empty() || content.to_ascii_lowercase().contains("<html") {
        let _ = fs::remove_file(&temp);
        return Err(AppError("下载内容不是有效的 JavaScript 音源".into()));
    }
    fs::rename(&temp, &source).map_err(|error| AppError(format!("覆盖音源文件失败：{error}")))?;
    music_source_from_path(&source)
}

#[tauri::command]
fn pick_music_source() -> AppResult<Option<String>> {
    #[cfg(windows)]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Filter = '音源脚本|*.js|所有文件|*.*'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  Write-Output $dialog.FileName
}
"#;
        let paths = run_powershell(script)?;
        Ok(paths.into_iter().next())
    }
    #[cfg(not(windows))]
    {
        Err(AppError("当前平台暂不支持原生音源选择".into()))
    }
}

fn run_powershell(script: &str) -> AppResult<Vec<String>> {
    let script = format!(
        "$ProgressPreference = 'SilentlyContinue'\n[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n$OutputEncoding = [System.Text.Encoding]::UTF8\n{}",
        script
    );
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-STA", "-Command", script.as_str()]);
    // 不要在 GUI 程序里弹出 PowerShell 控制台窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(AppError(message.trim().to_string()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim_start_matches('\u{feff}').to_string();
    Ok(stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

#[tauri::command]
fn pick_audio_files() -> AppResult<Vec<String>> {
    #[cfg(windows)]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Multiselect = $true
$dialog.Filter = '音频文件|*.mp3;*.flac;*.wav;*.m4a;*.aac;*.ogg;*.oga|所有文件|*.*'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  foreach ($file in $dialog.FileNames) { Write-Output $file }
}
"#;
        run_powershell(script)
    }
    #[cfg(not(windows))]
    {
        Err(AppError("当前平台暂不支持原生文件选择，请使用文件输入控件".into()))
    }
}

#[tauri::command]
fn pick_directory() -> AppResult<Option<String>> {
    #[cfg(windows)]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '选择目录'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  Write-Output $dialog.SelectedPath
}
"#;
        let paths = run_powershell(script)?;
        Ok(paths.into_iter().next())
    }
    #[cfg(not(windows))]
    {
        Err(AppError("当前平台暂不支持原生目录选择".into()))
    }
}

#[tauri::command]
fn hash_audio_file(file_path: String) -> AppResult<String> {
    sha256_file(Path::new(&file_path))
}

// 快捷键 Ctrl+F12 打开调试窗口（release 构建也可用）
#[tauri::command]
fn open_devtools(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.open_devtools();
    }
}

#[tauri::command]
fn open_app_directory(path: String) -> AppResult<()> {
    if path.trim().is_empty() {
        return Err(AppError("目录路径不能为空".into()));
    }
    let directory = PathBuf::from(path);
    ensure_directory(&directory)?;
    tauri_plugin_opener::open_path(&directory, None::<&str>)
        .map_err(|error| AppError(error.to_string()))
}

enum Instance {
    Primary(TcpListener),
    Secondary,
}

fn instance_lock_path() -> PathBuf {
    std::env::temp_dir().join("tingci-music-instance.lock")
}

fn acquire_instance() -> Instance {
    let lock_path = instance_lock_path();
    if let Ok(contents) = fs::read_to_string(&lock_path) {
        if let Ok(port) = contents.trim().parse::<u16>() {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                let _ = stream.write_all(b"focus\n");
                return Instance::Secondary;
            }
        }
        let _ = fs::remove_file(&lock_path);
    }
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => {
            if let Ok(address) = listener.local_addr() {
                let _ = fs::write(&lock_path, address.port().to_string());
            }
            Instance::Primary(listener)
        }
        Err(_) => Instance::Primary(
            TcpListener::bind(("127.0.0.1", 0)).expect("无法创建实例监听端口"),
        ),
    }
}

fn spawn_instance_listener(listener: TcpListener, app: tauri::AppHandle) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 16];
            let Ok(read_count) = stream.read(&mut buffer) else { continue };
            if &buffer[..read_count] == b"focus\n" {
                show_main_window(&app);
            }
        }
    });
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        // 若窗口被保存到了屏幕之外（例如上次最小化时记下的 -32000 坐标），拉回主屏。
        if let Ok(position) = window.outer_position() {
            let visible = app
                .available_monitors()
                .map(|monitors| {
                    monitors.iter().any(|monitor| {
                        let origin = monitor.position();
                        let size = monitor.size();
                        position.x < origin.x + size.width as i32 - 40
                            && position.x + 120 > origin.x
                            && position.y < origin.y + size.height as i32 - 40
                            && position.y + 60 > origin.y
                    })
                })
                .unwrap_or(true);
            if !visible {
                let _ = window.center();
            }
        }
        let _ = window.set_focus();
    }
}

fn close_all_windows(app: &AppHandle) {
    for window in app.webview_windows().into_values() {
        let _ = window.destroy();
    }
}

fn tray_icon_image() -> Image<'static> {
    #[cfg(windows)]
    if let Ok(icon) = Image::from_app_icon_resource(32) {
        return icon;
    }

    const SIZE: u32 = 16;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - 7;
            let dy = y as i32 - 7;
            let inside = dx * dx + dy * dy <= 49;
            if inside {
                rgba.extend_from_slice(&[91, 124, 255, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text("show", "显示主界面")
        .text("quit", "退出听词")
        .build()?;
    TrayIconBuilder::with_id("main-tray")
        .icon(tray_icon_image())
        .tooltip("听词")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                close_all_windows(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let listener = match acquire_instance() {
        Instance::Secondary => return,
        Instance::Primary(listener) => listener,
    };
    let builder = tauri::Builder::default()
        .manage(MediaScope::default())
        .register_uri_scheme_protocol("media", |context, request| {
            media_response(context.app_handle(), request)
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            spawn_instance_listener(listener, app.handle().clone());
            let _ = setup_tray(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_paths,
            set_app_paths,
            allow_media_files,
            import_songs,
            hash_audio_file,
            open_devtools,
            open_app_directory,
            pick_audio_files,
            pick_directory,
            search_music,
            get_playlist,
            get_cloud_media,
            download_remote_song,
            copy_song_to_download,
            remove_cached_song_files,
            clear_unused_cache,
            list_music_sources,
            read_music_source,
            source_http_request,
            import_music_source,
            import_music_source_url,
            refresh_music_source_url,
            delete_music_source,
            pick_music_source
        ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");
    app.run(|handle, event| match event {
        // 退出前先销毁所有 WebView 窗口，否则 WebView2 在进程结束时会因为
        // 窗口类下仍有存活窗口而注销失败（Failed to unregister class
        // Chrome_WidgetWin_0. Error = 1412）。
        RunEvent::ExitRequested { .. } => close_all_windows(handle),
        RunEvent::Exit => {
            let _ = fs::remove_file(instance_lock_path());
        }
        _ => {}
    });
}
