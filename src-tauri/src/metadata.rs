use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Cover {
    pub data: Vec<u8>,
    pub extension: String,
}

#[derive(Debug, Clone, Default)]
pub struct SongMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub cover: Option<Cover>,
}

impl SongMetadata {
    pub fn from_path(path: &Path) -> Self {
        let data = fs::read(path).unwrap_or_default();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let fallback_title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("未知歌曲")
            .to_string();

        let mut metadata = match extension.as_str() {
            "mp3" => parse_mp3(&data),
            "flac" => parse_flac(&data),
            "wav" => parse_wav(&data),
            "m4a" | "mp4" | "aac" => parse_mp4(&data, &extension),
            "ogg" | "oga" => parse_ogg(&data),
            _ => SongMetadata::default(),
        };
        if metadata.title.trim().is_empty() {
            metadata.title = fallback_title;
        }
        if metadata.artist.trim().is_empty() {
            metadata.artist = "未知歌手".to_string();
        }
        if metadata.album.trim().is_empty() {
            metadata.album = "未知专辑".to_string();
        }
        metadata
    }
}

fn u32be(data: &[u8]) -> u32 {
    if data.len() < 4 {
        return 0;
    }
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

fn syncsafe(data: &[u8]) -> usize {
    if data.len() < 4 {
        return 0;
    }
    ((data[0] as usize & 0x7f) << 21)
        | ((data[1] as usize & 0x7f) << 14)
        | ((data[2] as usize & 0x7f) << 7)
        | (data[3] as usize & 0x7f)
}

fn trim_bytes(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .trim_matches(|character: char| character == '\0' || character.is_whitespace())
        .to_string()
}

fn decode_text(value: &[u8], encoding: u8) -> String {
    match encoding {
        0 => trim_bytes(value),
        1 => {
            if value.len() >= 2 && value[0] == 0xff && value[1] == 0xfe {
                decode_utf16(&value[2..], false)
            } else if value.len() >= 2 && value[0] == 0xfe && value[1] == 0xff {
                decode_utf16(&value[2..], true)
            } else {
                decode_utf16(value, false)
            }
        }
        2 => decode_utf16(value, true),
        _ => trim_bytes(value),
    }
}

fn decode_utf16(value: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = value
        .chunks_exact(2)
        .map(|pair| {
            if big_endian {
                u16::from_be_bytes([pair[0], pair[1]])
            } else {
                u16::from_le_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
        .trim_matches(|character: char| character == '\0' || character.is_whitespace())
        .to_string()
}

fn extension_for_mime(mime: &str, data: &[u8]) -> String {
    if mime.starts_with("image/png") || data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if mime.starts_with("image/webp") || data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    }
    .to_string()
}

fn set_default_if_empty(value: &mut String, fallback: &str) {
    if value.trim().is_empty() {
        *value = fallback.to_string();
    }
}

fn parse_mp3(data: &[u8]) -> SongMetadata {
    let mut metadata = SongMetadata::default();
    if data.len() < 10 || &data[0..3] != b"ID3" {
        metadata.duration_ms = parse_mpeg_duration(data, 0);
        return metadata;
    }

    let version = data[3];
    let flags = data[4];
    let tag_size = syncsafe(&data[6..10]);
    let mut position = 10;
    if flags & 0x40 != 0 && position + 4 <= data.len() {
        let extended_size = if version >= 4 {
            syncsafe(&data[position..position + 4])
        } else {
            u32be(&data[position..position + 4]) as usize
        };
        position += extended_size.max(4);
    }
    let tag_end = (10 + tag_size).min(data.len());
    while position + 10 <= tag_end {
        let frame_id = &data[position..position + 4];
        if frame_id[0] == 0 {
            break;
        }
        let frame_size = if version >= 4 {
            syncsafe(&data[position + 4..position + 8])
        } else {
            u32be(&data[position + 4..position + 8]) as usize
        };
        let frame_end = position + 10 + frame_size;
        if frame_end > tag_end || frame_size == 0 {
            break;
        }
        let payload = &data[position + 10..frame_end];
        match frame_id {
            b"TIT2" if payload.len() > 1 => metadata.title = decode_text(&payload[1..], payload[0]),
            b"TPE1" if payload.len() > 1 => metadata.artist = decode_text(&payload[1..], payload[0]),
            b"TALB" if payload.len() > 1 => metadata.album = decode_text(&payload[1..], payload[0]),
            b"APIC" => {
                if payload.len() > 4 {
                    let encoding = payload[0];
                    let mime_end = payload[1..]
                        .iter()
                        .position(|byte| *byte == 0)
                        .map(|offset| offset + 1)
                        .unwrap_or(1);
                    let mime = trim_bytes(&payload[1..mime_end]);
                    let description_start = 1 + mime_end + 1;
                    if description_start < payload.len() {
                        let terminator = if encoding == 1 || encoding == 2 {
                            2
                        } else {
                            1
                        };
                        let mut description_end = description_start;
                        while description_end + terminator <= payload.len() {
                            let is_terminator = if terminator == 2 {
                                payload[description_end] == 0 && payload[description_end + 1] == 0
                            } else {
                                payload[description_end] == 0
                            };
                            if is_terminator {
                                break;
                            }
                            description_end += terminator;
                        }
                        let image_start = (description_end + terminator).min(payload.len());
                        if image_start < payload.len() {
                            let image = payload[image_start..].to_vec();
                            if !image.is_empty() {
                                metadata.cover = Some(Cover {
                                    extension: extension_for_mime(&mime, &image),
                                    data: image,
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        position = frame_end;
    }

    metadata.duration_ms = parse_mpeg_duration(data, tag_end);
    set_default_if_empty(&mut metadata.artist, "未知歌手");
    set_default_if_empty(&mut metadata.album, "未知专辑");
    metadata
}

fn parse_mpeg_duration(data: &[u8], start: usize) -> u64 {
    let mut position = start;
    while position + 4 <= data.len() {
        if data[position] == 0xff && data[position + 1] & 0xe0 == 0xe0 {
            let header = u32::from_be_bytes([
                0,
                data[position],
                data[position + 1],
                data[position + 2],
            ]);
            let version_id = (header >> 19) & 0x03;
            let layer_id = (header >> 17) & 0x03;
            let bitrate_index = ((header >> 12) & 0x0f) as usize;
            let sample_index = ((header >> 10) & 0x03) as usize;
            if version_id == 1 || layer_id == 0 || bitrate_index == 0 || sample_index == 3 {
                position += 1;
                continue;
            }
            let base_rates = [44100, 48000, 32000];
            let sample_rate = match version_id {
                3 => base_rates[sample_index],
                2 => base_rates[sample_index] / 2,
                _ => base_rates[sample_index] / 4,
            };
            let mpeg1 = version_id == 3;
            let layer3 = layer_id == 1;
            let bitrate = if layer3 {
                if mpeg1 {
                    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320][bitrate_index] * 1000
                } else {
                    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bitrate_index] * 1000
                }
            } else if layer_id == 3 {
                [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 320][bitrate_index] * 1000
            } else {
                [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 320][bitrate_index] * 1000
            };
            let samples_per_frame = if layer3 {
                if mpeg1 {
                    1152
                } else {
                    576
                }
            } else if layer_id == 3 {
                if mpeg1 {
                    384
                } else {
                    384
                }
            } else if mpeg1 {
                1152
            } else {
                576
            };
            let frame_length = if layer_id == 3 {
                if mpeg1 {
                    144 * bitrate / sample_rate
                } else {
                    72 * bitrate / sample_rate
                }
            } else if layer_id == 2 {
                144 * bitrate / sample_rate
            } else {
                12 * bitrate / sample_rate / 4
            };
            if frame_length <= 4 {
                position += 1;
                continue;
            }
            let side_info = if mpeg1 {
                if ((header >> 6) & 0x03) == 3 {
                    17
                } else {
                    32
                }
            } else if ((header >> 6) & 0x03) == 3 {
                9
            } else {
                17
            };
            let xing_position = position + 4 + side_info;
            if xing_position + 12 <= data.len() {
                let marker = &data[xing_position..xing_position + 4];
                if marker == b"Xing" || marker == b"Info" {
                    let flags = u32be(&data[xing_position + 4..xing_position + 8]);
                    let mut cursor = xing_position + 8;
                    let mut frames = 0u32;
                    if flags & 1 != 0 && cursor + 4 <= data.len() {
                        frames = u32be(&data[cursor..cursor + 4]);
                        cursor += 4;
                    }
                    if frames > 0 {
                        return (frames as u64 * samples_per_frame as u64 * 1000) / sample_rate as u64;
                    }
                    if flags & 2 != 0 && cursor + 4 <= data.len() {
                        let bytes = u32be(&data[cursor..cursor + 4]) as u64;
                        if bitrate > 0 {
                            return bytes * 8 * 1000 / bitrate as u64;
                        }
                    }
                }
            }
            let audio_bytes = data.len().saturating_sub(position);
            if bitrate > 0 {
                return audio_bytes as u64 * 8 * 1000 / bitrate as u64;
            }
            let _ = samples_per_frame;
            let _ = frame_length;
        }
        position += 1;
    }
    0
}

fn parse_flac(data: &[u8]) -> SongMetadata {
    let mut metadata = SongMetadata::default();
    if data.len() < 8 || &data[0..4] != b"fLaC" {
        return metadata;
    }
    let mut position = 4;
    while position + 4 <= data.len() {
        let block_type = data[position] & 0x7f;
        let block_length = u32be(&data[position + 1..position + 4]) as usize;
        let body_start = position + 4;
        let body_end = (body_start + block_length).min(data.len());
        if body_end < body_start {
            break;
        }
        let body = &data[body_start..body_end];
        match block_type {
            0 if body.len() >= 18 => {
                let sample_rate =
                    ((body[10] as u32) << 12) | ((body[11] as u32) << 4) | ((body[12] as u32) >> 4);
                let total_samples = ((body[14] as u64) << 28)
                    | ((body[15] as u64) << 20)
                    | ((body[16] as u64) << 12)
                    | ((body[17] as u64) << 4)
                    | ((body.get(18).copied().unwrap_or(0) as u64) >> 4);
                if sample_rate > 0 && total_samples > 0 {
                    metadata.duration_ms = total_samples * 1000 / sample_rate as u64;
                }
            }
            4 => {
                if body.len() >= 4 {
                    let mut cursor = 4;
                    let vendor_length = u32be(&body[cursor..cursor + 4]) as usize;
                    cursor += 4 + vendor_length;
                    if cursor + 4 <= body.len() {
                        let count = u32be(&body[cursor..cursor + 4]) as usize;
                        cursor += 4;
                        for _ in 0..count {
                            if cursor + 4 > body.len() {
                                break;
                            }
                            let length = u32be(&body[cursor..cursor + 4]) as usize;
                            cursor += 4;
                            if cursor + length > body.len() {
                                break;
                            }
                            let entry = trim_bytes(&body[cursor..cursor + length]);
                            cursor += length;
                            if let Some((key, value)) = entry.split_once('=') {
                                match key.to_ascii_lowercase().as_str() {
                                    "title" => metadata.title = value.to_string(),
                                    "artist" => metadata.artist = value.to_string(),
                                    "album" => metadata.album = value.to_string(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            6 if body.len() >= 32 => {
                let mime_length = u32be(&body[4..8]) as usize;
                let mime_start = 8;
                let mime_end = mime_start + mime_length;
                if mime_end + 4 <= body.len() {
                    let mime = trim_bytes(&body[mime_start..mime_end]);
                    let description_length = u32be(&body[mime_end..mime_end + 4]) as usize;
                    let data_length_start = mime_end + 4 + description_length;
                    if data_length_start + 4 <= body.len() {
                        let image_length = u32be(&body[data_length_start..data_length_start + 4]) as usize;
                        let image_start = data_length_start + 4;
                        if image_start + image_length <= body.len() {
                            let image = body[image_start..image_start + image_length].to_vec();
                            metadata.cover = Some(Cover {
                                extension: extension_for_mime(&mime, &image),
                                data: image,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        position = body_end;
        if position >= data.len() || data[position] & 0x80 != 0 {
            break;
        }
    }
    set_default_if_empty(&mut metadata.artist, "未知歌手");
    set_default_if_empty(&mut metadata.album, "未知专辑");
    metadata
}

fn parse_wav(data: &[u8]) -> SongMetadata {
    let mut metadata = SongMetadata::default();
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return metadata;
    }
    let mut position = 12;
    let mut byte_rate = 0u32;
    let mut data_size = 0u64;
    while position + 8 <= data.len() {
        let chunk_id = &data[position..position + 4];
        let chunk_size = u32be(&data[position + 4..position + 8]) as usize;
        let body_start = position + 8;
        let body_end = (body_start + chunk_size).min(data.len());
        if body_end < body_start {
            break;
        }
        let body = &data[body_start..body_end];
        if chunk_id == b"fmt " && body.len() >= 12 {
            byte_rate = u32be(&body[8..12]);
        } else if chunk_id == b"data" {
            data_size = chunk_size as u64;
        } else if chunk_id == b"LIST" && body.len() >= 4 && &body[0..4] == b"INFO" {
            let mut info_position = 4;
            while info_position + 8 <= body.len() {
                let key = &body[info_position..info_position + 4];
                let size = u32be(&body[info_position + 4..info_position + 8]) as usize;
                let value_start = info_position + 8;
                let value_end = (value_start + size).min(body.len());
                if value_end < value_start {
                    break;
                }
                let value = trim_bytes(&body[value_start..value_end]);
                match key {
                    b"INAM" => metadata.title = value,
                    b"IART" => metadata.artist = value,
                    b"IPRD" => metadata.album = value,
                    _ => {}
                }
                info_position = value_end + (size & 1);
            }
        }
        position = body_end + (chunk_size & 1);
    }
    if byte_rate > 0 && data_size > 0 {
        metadata.duration_ms = data_size * 1000 / byte_rate as u64;
    }
    set_default_if_empty(&mut metadata.artist, "未知歌手");
    set_default_if_empty(&mut metadata.album, "未知专辑");
    metadata
}

fn find_box(data: &[u8], target: &[u8; 4], start: usize, end: usize) -> Option<(usize, usize)> {
    let mut position = start;
    while position + 8 <= end {
        let mut size = u32be(&data[position..position + 4]) as usize;
        let box_type = &data[position + 4..position + 8];
        let mut header_size = 8;
        if size == 1 && position + 16 <= end {
            size = u64::from_be_bytes([
                data[position + 8],
                data[position + 9],
                data[position + 10],
                data[position + 11],
                data[position + 12],
                data[position + 13],
                data[position + 14],
                data[position + 15],
            ]) as usize;
            header_size = 16;
        } else if size == 0 {
            size = end.saturating_sub(position);
        }
        if size < header_size || position + size > end {
            break;
        }
        let payload_start = position + header_size;
        let payload_end = position + size;
        if box_type == target {
            return Some((payload_start, payload_end));
        }
        if let Some(found) = find_box(data, target, payload_start, payload_end) {
            return Some(found);
        }
        position = payload_end;
    }
    None
}

fn parse_mp4(data: &[u8], extension: &str) -> SongMetadata {
    let mut metadata = SongMetadata::default();
    if let Some((start, end)) = find_box(data, b"mvhd", 0, data.len()) {
        let payload = &data[start..end];
        if payload.len() >= 20 {
            let version = payload[0];
            if version == 0 && payload.len() >= 20 {
                let timescale = u32be(&payload[12..16]);
                let duration = u32be(&payload[16..20]);
                if timescale > 0 {
                    metadata.duration_ms = duration as u64 * 1000 / timescale as u64;
                }
            } else if payload.len() >= 32 {
                let timescale = u32be(&payload[20..24]);
                let duration = u64::from_be_bytes([
                    payload[24],
                    payload[25],
                    payload[26],
                    payload[27],
                    payload[28],
                    payload[29],
                    payload[30],
                    payload[31],
                ]);
                if timescale > 0 {
                    metadata.duration_ms = duration * 1000 / timescale as u64;
                }
            }
        }
    }
    if let Some((start, end)) = find_box(data, b"ilst", 0, data.len()) {
        let mut position = start;
        while position + 8 <= end {
            let size = u32be(&data[position..position + 4]) as usize;
            if size < 8 || position + size > end {
                break;
            }
            let item_type = &data[position + 4..position + 8];
            let item_payload_start = position + 8;
            let item_payload_end = position + size;
            if let Some((value_start, value_end)) = find_box(data, b"data", item_payload_start, item_payload_end) {
                let value_payload = &data[value_start..value_end];
                if value_payload.len() > 8 {
                    let data_type = u32be(&value_payload[0..4]);
                    let content = &value_payload[8..];
                    match item_type {
                        b"\xa9nam" => metadata.title = trim_bytes(content),
                        b"\xa9ART" => metadata.artist = trim_bytes(content),
                        b"\xa9alb" => metadata.album = trim_bytes(content),
                        b"covr" => {
                            let mime = if data_type == 14 { "image/png" } else { "image/jpeg" };
                            metadata.cover = Some(Cover {
                                extension: extension_for_mime(mime, content),
                                data: content.to_vec(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            position = item_payload_end;
        }
    }
    if metadata.duration_ms == 0 && extension == "aac" {
        metadata.duration_ms = parse_adts_duration(data);
    }
    set_default_if_empty(&mut metadata.artist, "未知歌手");
    set_default_if_empty(&mut metadata.album, "未知专辑");
    metadata
}

fn parse_adts_duration(data: &[u8]) -> u64 {
    const RATES: [u32; 12] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
    ];
    let mut position = 0;
    let mut frames = 0u64;
    let mut sample_rate = 0u32;
    while position + 7 <= data.len() {
        if data[position] != 0xff || data[position + 1] & 0xf6 != 0xf0 {
            position += 1;
            continue;
        }
        let rate_index = ((data[position + 2] >> 2) & 0x0f) as usize;
        if rate_index >= RATES.len() {
            break;
        }
        sample_rate = RATES[rate_index];
        let frame_length =
            (((data[position + 3] as usize & 0x03) << 11) | ((data[position + 4] as usize) << 3)
                | ((data[position + 5] as usize) >> 5))
            as usize;
        if frame_length < 7 {
            break;
        }
        frames += 1;
        position += frame_length;
    }
    if sample_rate > 0 {
        frames * 1024 * 1000 / sample_rate as u64
    } else {
        0
    }
}

fn parse_ogg(data: &[u8]) -> SongMetadata {
    let mut metadata = SongMetadata::default();
    if data.len() < 4 || &data[0..4] != b"OggS" {
        return metadata;
    }
    let mut sample_rate = 0u32;
    if data.len() > 30 && &data[28..34] == b"vorbis" && data[27] == 1 {
        sample_rate = u32::from_le_bytes([data[38], data[39], data[40], data[41]]);
        let nominal_bitrate = i32::from_le_bytes([data[42], data[43], data[44], data[45]]);
        if nominal_bitrate > 0 {
            metadata.duration_ms = data.len() as u64 * 8 * 1000 / nominal_bitrate as u64;
        }
    } else if data.len() > 30 && &data[28..35] == b"OpusHead" {
        sample_rate = 48000;
        metadata.duration_ms = data.len() as u64 * 8 * 1000 / 16000;
    }
    let haystack = String::from_utf8_lossy(data);
    for (key, target) in [
        ("title=", &mut metadata.title),
        ("artist=", &mut metadata.artist),
        ("album=", &mut metadata.album),
    ] {
        if let Some(position) = haystack.to_ascii_lowercase().find(key) {
            let value_start = position + key.len();
            let value_end = haystack[value_start..]
                .find('\0')
                .map(|offset| value_start + offset)
                .unwrap_or(haystack.len());
            *target = haystack[value_start..value_end].to_string();
        }
    }
    if sample_rate == 0 {
        metadata.duration_ms = 0;
    }
    set_default_if_empty(&mut metadata.artist, "未知歌手");
    set_default_if_empty(&mut metadata.album, "未知专辑");
    metadata
}
