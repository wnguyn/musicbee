use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: u32,
    pub track_number: u32,
    pub duration: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MpdStatus {
    pub connected: bool,
    pub state: String,
    pub elapsed: u32,
    pub duration: u32,
    pub volume: i32,
    pub file: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub error: String,
    pub song: i32,
    pub playlist_version: u32,
    pub playlist_length: u32,
    pub repeat: bool,
    pub random: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumSummary {
    pub album: String,
    pub album_artist: String,
    pub year: u32,
}

#[derive(Debug, Clone)]
pub struct AlbumArt {
    pub mime: String,
    pub data: Vec<u8>,
}

struct MpdClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl MpdClient {
    fn connect() -> Result<Self, String> {
        let addr = mpd_addr();
        let password = env::var("MPD_PASSWORD").ok();

        let stream = TcpStream::connect(addr).map_err(|e| format!("MPD connection failed: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("MPD read failed: {e}"))?;
        let greeting = line.trim_end_matches(['\r', '\n']).to_string();
        if !greeting.starts_with("OK MPD") {
            return Err(format!("MPD did not send a valid greeting: {greeting}"));
        }

        let mut client = Self { stream, reader };
        if let Some(password) = password {
            client.command(&format!("password {}", quote_arg(&password)))?;
        }
        // Most cover images can now arrive in one chunk. Older MPD versions
        // reject this command, so we intentionally ignore the result.
        let _ = client.command("binarylimit 1048576");
        Ok(client)
    }

    fn command(&mut self, command: &str) -> Result<Vec<String>, String> {
        self.stream
            .write_all(format!("{command}\n").as_bytes())
            .map_err(|e| format!("MPD write failed: {e}"))?;
        self.read_response()
    }

    fn read_response(&mut self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        loop {
            let mut line = String::new();
            let bytes = self
                .reader
                .read_line(&mut line)
                .map_err(|e| format!("MPD read failed: {e}"))?;
            if bytes == 0 {
                return Err("MPD closed the connection".to_string());
            }
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            if line == "OK" || line.starts_with("OK MPD") {
                out.push(line);
                return Ok(out);
            }
            if line.starts_with("ACK") {
                return Err(line);
            }
            out.push(line);
        }
    }

    fn album_art(&mut self, uri: &str) -> Result<Vec<u8>, String> {
        match self.fetch_binary("albumart", uri) {
            Ok(data) => Ok(data),
            Err(_) => self.fetch_binary("readpicture", uri),
        }
    }

    fn read_text_line(&mut self) -> Result<String, String> {
        let mut line = Vec::with_capacity(64);
        self.reader
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("MPD read failed: {e}"))?;
        Ok(String::from_utf8_lossy(&line)
            .trim_end_matches(['\r', '\n'])
            .to_string())
    }

    fn fetch_binary(&mut self, command: &str, uri: &str) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        let mut total = None;

        loop {
            let offset = data.len();
            let cmd = format!("{command} {} {offset}\n", quote_arg(uri));
            self.stream
                .write_all(cmd.as_bytes())
                .map_err(|e| format!("MPD write failed: {e}"))?;

            let size_line = self.read_text_line()?;
            if size_line.starts_with("ACK") {
                return Err(size_line);
            }
            let size: usize = size_line
                .strip_prefix("size: ")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("Expected 'size:' header, got: {size_line}"))?;
            if total.is_none() {
                data.reserve(size);
                total = Some(size);
            }

            let binary_line = self.read_text_line()?;
            if binary_line.starts_with("ACK") {
                return Err(binary_line);
            }
            let chunk_size: usize = binary_line
                .strip_prefix("binary: ")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("Expected 'binary:' header, got: {binary_line}"))?;
            if chunk_size == 0 {
                return Err("MPD returned an empty binary chunk".to_string());
            }

            let prev = data.len();
            data.resize(prev + chunk_size, 0);
            self.reader
                .read_exact(&mut data[prev..])
                .map_err(|e| format!("MPD binary read error: {e}"))?;

            let trailing = self.read_text_line()?;
            if trailing == "OK" {
                break;
            }
            if !trailing.is_empty() {
                return Err(format!("Unexpected data after binary chunk: {trailing}"));
            }

            let ok = self.read_text_line()?;
            if ok != "OK" {
                return Err(format!("Expected OK after binary chunk, got: {ok}"));
            }

            if data.len() >= total.unwrap_or(0) {
                break;
            }
        }

        Ok(data)
    }
}

pub fn get_library() -> Result<Vec<Track>, String> {
    let mut mpd = MpdClient::connect()?;
    let lines = mpd.command("listallinfo")?;
    Ok(parse_tracks(&lines))
}

pub fn get_mpd_status() -> MpdStatus {
    read_mpd_status().unwrap_or_else(disconnected_status)
}

pub fn mpd_play_path(path: String) -> Result<(), String> {
    let mut mpd = MpdClient::connect()?;
    play_paths(&mut mpd, &[path], 0)
}

pub fn mpd_set_queue(paths: Vec<String>, index: usize) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No tracks were sent to MPD".to_string());
    }
    let mut mpd = MpdClient::connect()?;
    let index = index.min(paths.len() - 1);
    play_paths(&mut mpd, &paths, index)
}

pub fn mpd_enqueue(paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No tracks were sent to MPD".to_string());
    }
    let mut mpd = MpdClient::connect()?;
    let mut command = String::from("command_list_begin\n");
    for path in &paths {
        command.push_str("add ");
        command.push_str(&quote_arg(path));
        command.push('\n');
    }
    command.push_str("command_list_end");
    mpd.command(&command)?;
    Ok(())
}

pub fn get_queue() -> Result<Vec<Track>, String> {
    let mut mpd = MpdClient::connect()?;
    let lines = mpd.command("playlistinfo")?;
    Ok(parse_tracks(&lines))
}

pub fn mpd_play_idx(index: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("play {index}"))?;
    Ok(())
}

pub fn mpd_clear_queue() -> Result<(), String> {
    MpdClient::connect()?.command("clear")?;
    Ok(())
}

pub fn mpd_delete_from_queue(index: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("delete {index}"))?;
    Ok(())
}

pub fn mpd_set_repeat(enabled: bool) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("repeat {}", u8::from(enabled)))?;
    Ok(())
}

pub fn mpd_set_random(enabled: bool) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("random {}", u8::from(enabled)))?;
    Ok(())
}

pub fn mpd_toggle_play() -> Result<(), String> {
    let mut mpd = MpdClient::connect()?;
    let status = parse_pairs(&mpd.command("status")?);
    if status.get("state").is_some_and(|s| s == "play") {
        mpd.command("pause 1")?;
    } else {
        mpd.command("play")?;
    }
    Ok(())
}

pub fn mpd_stop() -> Result<(), String> {
    MpdClient::connect()?.command("stop")?;
    Ok(())
}

pub fn mpd_next() -> Result<(), String> {
    MpdClient::connect()?.command("next")?;
    Ok(())
}

pub fn mpd_previous() -> Result<(), String> {
    MpdClient::connect()?.command("previous")?;
    Ok(())
}

pub fn mpd_set_volume(volume: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("setvol {}", volume.min(100)))?;
    Ok(())
}

pub fn mpd_seek_current(seconds: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("seekcur {seconds}"))?;
    Ok(())
}

pub fn get_album_art(path: String) -> Result<AlbumArt, String> {
    let mut mpd = MpdClient::connect()?;
    let data = mpd.album_art(&path)?;
    let mime = detect_mime(&data).to_string();
    Ok(AlbumArt { mime, data })
}

/// Album cover with a Last.fm fallback: try MPD's embedded/sidecar art first,
/// then `album.getInfo` from Last.fm when MPD has nothing. Last.fm is only
/// consulted when `LASTFM_API_KEY` is set in the environment.
pub fn get_cover(path: String, album_artist: String, album: String) -> Result<AlbumArt, String> {
    if let Ok(mut mpd) = MpdClient::connect() {
        if let Ok(data) = mpd.album_art(&path) {
            if !data.is_empty() {
                let mime = detect_mime(&data).to_string();
                return Ok(AlbumArt { mime, data });
            }
        }
    }
    lastfm_album_art(&album_artist, &album)
}

/// Artist image from Last.fm `artist.getInfo`. Last.fm only.
pub fn get_artist_image(artist: String) -> Result<AlbumArt, String> {
    lastfm_artist_image(&artist)
}

fn lastfm_key() -> Option<String> {
    env::var("LASTFM_API_KEY").ok().filter(|s| !s.trim().is_empty())
}

fn lastfm_album_art(artist: &str, album: &str) -> Result<AlbumArt, String> {
    let key = lastfm_key().ok_or("LASTFM_API_KEY not set")?;
    let url = format!(
        "https://ws.audioscrobbler.com/2.0/?method=album.getinfo&api_key={}&artist={}&album={}&format=json",
        key,
        url_encode(artist),
        url_encode(album)
    );
    let body = http_get_string(&url)?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let url = pick_image(value.get("album").and_then(|a| a.get("image")))?;
    download_image(&url)
}

fn lastfm_artist_image(artist: &str) -> Result<AlbumArt, String> {
    let key = lastfm_key().ok_or("LASTFM_API_KEY not set")?;
    let url = format!(
        "https://ws.audioscrobbler.com/2.0/?method=artist.getinfo&api_key={}&artist={}&format=json",
        key,
        url_encode(artist)
    );
    let body = http_get_string(&url)?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let url = pick_image(value.get("artist").and_then(|a| a.get("image")))?;
    download_image(&url)
}

// Choose the largest non-empty image URL from a Last.fm `image` array.
fn pick_image(images: Option<&serde_json::Value>) -> Result<String, String> {
    let arr = images
        .and_then(|v| v.as_array())
        .ok_or("Last.fm response had no image array")?;
    let rank = |s: &str| match s {
        "mega" => 5,
        "extralarge" => 4,
        "large" => 3,
        "medium" => 2,
        "small" => 1,
        _ => 0,
    };
    let mut best: Option<String> = None;
    let mut best_rank = -1i32;
    for item in arr {
        let url = item.get("#text").and_then(|t| t.as_str()).unwrap_or("");
        if url.is_empty() {
            continue;
        }
        // Last.fm serves the same placeholder star for artists without a real
        // image; treat it as "no image" so callers can fall back.
        if url.contains("2a96cbd8b46e442fc41c2b86b821562f") {
            continue;
        }
        let r = rank(item.get("size").and_then(|s| s.as_str()).unwrap_or(""));
        if r > best_rank {
            best_rank = r;
            best = Some(url.to_string());
        }
    }
    best.ok_or_else(|| "Last.fm had no usable image".to_string())
}

fn http_get_string(url: &str) -> Result<String, String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    resp.into_string().map_err(|e| e.to_string())
}

fn download_image(url: &str) -> Result<AlbumArt, String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    resp.into_reader()
        .take(12_000_000)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.is_empty() {
        return Err("Downloaded image was empty".to_string());
    }
    let mime = detect_mime(&data).to_string();
    Ok(AlbumArt { mime, data })
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn get_albums() -> Result<Vec<AlbumSummary>, String> {
    Ok(summarize_albums(&get_library()?))
}

pub fn get_album_tracks(album_artist: String, album: String) -> Result<Vec<Track>, String> {
    let mut mpd = MpdClient::connect()?;
    let cmd = format!(
        "find albumartist {} album {}",
        quote_arg(&album_artist),
        quote_arg(&album)
    );
    let lines = mpd.command(&cmd)?;
    Ok(parse_tracks(&lines))
}

fn disconnected_status(error: String) -> MpdStatus {
    MpdStatus {
        connected: false,
        state: "stop".to_string(),
        elapsed: 0,
        duration: 0,
        volume: -1,
        file: String::new(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        error,
        song: -1,
        playlist_version: 0,
        playlist_length: 0,
        repeat: false,
        random: false,
    }
}

fn play_paths(mpd: &mut MpdClient, paths: &[String], index: usize) -> Result<(), String> {
    let index = index.min(paths.len().saturating_sub(1));
    let mut command = String::from("command_list_begin\nclear\n");
    for path in paths {
        command.push_str("add ");
        command.push_str(&quote_arg(path));
        command.push('\n');
    }
    command.push_str(&format!("play {index}\ncommand_list_end"));
    mpd.command(&command)?;
    Ok(())
}

fn read_mpd_status() -> Result<MpdStatus, String> {
    let mut mpd = MpdClient::connect()?;
    let status = parse_pairs(&mpd.command("status")?);
    let song = parse_pairs(&mpd.command("currentsong")?);
    let elapsed = status
        .get("elapsed")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0) as u32;
    let duration = status
        .get("duration")
        .or_else(|| song.get("duration"))
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0) as u32;

    Ok(MpdStatus {
        connected: true,
        state: status
            .get("state")
            .cloned()
            .unwrap_or_else(|| "stop".to_string()),
        elapsed,
        duration,
        volume: status.get("volume").and_then(|v| v.parse().ok()).unwrap_or(-1),
        file: song.get("file").cloned().unwrap_or_default(),
        title: song.get("Title").cloned().unwrap_or_default(),
        artist: song.get("Artist").cloned().unwrap_or_default(),
        album: song.get("Album").cloned().unwrap_or_default(),
        error: status.get("error").cloned().unwrap_or_default(),
        song: status.get("song").and_then(|v| v.parse().ok()).unwrap_or(-1),
        playlist_version: status
            .get("playlist")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        playlist_length: status
            .get("playlistlength")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        repeat: status.get("repeat").map(|v| v == "1").unwrap_or(false),
        random: status.get("random").map(|v| v == "1").unwrap_or(false),
    })
}

fn parse_tracks(lines: &[String]) -> Vec<Track> {
    let mut tracks = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line == "OK" || line.starts_with("OK MPD") {
            continue;
        }
        if let Some(file) = line.strip_prefix("file: ") {
            if !current.is_empty() {
                if let Some(track) = track_from_pairs(&current) {
                    tracks.push(track);
                }
                current.clear();
            }
            current.push(("file".to_string(), file.to_string()));
        } else if let Some((key, value)) = line.split_once(": ") {
            current.push((key.to_string(), value.to_string()));
        }
    }
    if let Some(track) = track_from_pairs(&current) {
        tracks.push(track);
    }
    tracks
}

fn track_from_pairs(pairs: &[(String, String)]) -> Option<Track> {
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let path = get("file")?;
    let title = get("Title").unwrap_or_else(|| title_from_path(&path));
    let artist = get("Artist").unwrap_or_else(|| "Unknown Artist".to_string());
    let album = get("Album").unwrap_or_else(|| "Unknown Album".to_string());
    let album_artist = get("AlbumArtist")
        .or_else(|| get("Album Artist"))
        .unwrap_or_else(|| artist.clone());
    let genre = get("Genre").unwrap_or_default();
    let year = get("Date")
        .or_else(|| get("OriginalDate"))
        .and_then(|v| v.chars().take(4).collect::<String>().parse().ok())
        .unwrap_or(0);
    let track_number = get("Track")
        .and_then(|v| v.split('/').next().unwrap_or(&v).parse().ok())
        .unwrap_or(0);
    let duration = get("duration")
        .or_else(|| get("Time"))
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| format_duration(v as u32))
        .unwrap_or_else(|| "0:00".to_string());

    Some(Track {
        title,
        artist,
        album,
        album_artist,
        genre,
        year,
        track_number,
        duration,
        path,
    })
}

fn summarize_albums(tracks: &[Track]) -> Vec<AlbumSummary> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in tracks {
        let key = format!("{}\u{0001}{}", t.album, t.album_artist);
        if seen.insert(key) {
            out.push(AlbumSummary {
                album: t.album.clone(),
                album_artist: t.album_artist.clone(),
                year: t.year,
            });
        }
    }
    out
}

fn parse_pairs(lines: &[String]) -> HashMap<String, String> {
    lines
        .iter()
        .filter_map(|line| line.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn mpd_addr() -> String {
    let host = env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string());

    if host.contains("]:") || (!host.contains("::") && host.contains(':')) {
        return host;
    }
    if host.contains("::") && !host.starts_with('[') {
        return format!("[{}]:{}", host, port);
    }
    if host.starts_with('[') && !host.contains("]:") {
        return format!("{}:{}", host, port);
    }
    format!("{}:{}", host, port)
}

fn quote_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn format_duration(seconds: u32) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn title_from_path(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(title, _)| title)
        .unwrap_or(path)
        .to_string()
}

fn detect_mime(data: &[u8]) -> &'static str {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        "image/jpeg"
    } else if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        "image/png"
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        "image/webp"
    } else if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}
