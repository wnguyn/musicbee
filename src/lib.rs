use serde::Serialize;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tauri::Manager;
use base64::Engine;

#[derive(Serialize, Clone)]
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

#[derive(Serialize)]
struct MpdStatus {
    connected: bool,
    state: String,
    elapsed: u32,
    duration: u32,
    volume: i32,
    file: String,
    title: String,
    artist: String,
    album: String,
    error: String,
}

#[derive(Serialize, Clone)]
pub struct AlbumSummary {
    pub album: String,
    pub album_artist: String,
    pub year: u32,
}

struct MpdClient {
    stream: TcpStream,
    // Persistent buffered reader. MPD only sends data after a command, so
    // the BufReader's pre-fill is bounded by MPD's response for one
    // command; reading the same reader across calls preserves any bytes
    // MPD sent between OK and the next read.
    reader: BufReader<TcpStream>,
}

impl MpdClient {
    fn connect() -> Result<Self, String> {
        let addr = mpd_addr();
        let password = env::var("MPD_PASSWORD").ok();

        let stream = TcpStream::connect(addr).map_err(|e| format!("MPD connection failed: {e}"))?;
        // 30s is enough for listallinfo on a 5000+ track library. MPD
        // sends the whole response before the next prompt, so a single
        // read timeout applies to the whole payload.
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| format!("MPD read failed: {e}"))?;
        let greeting = line.trim_end_matches(['\r', '\n']).to_string();
        if !greeting.starts_with("OK MPD") {
            return Err(format!("MPD did not send a valid greeting: {greeting}"));
        }

        let mut client = Self { stream, reader };
        if let Some(password) = password {
            client.command(&format!("password {}", quote_arg(&password)))?;
        }
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
            let bytes = self.reader.read_line(&mut line).map_err(|e| format!("MPD read failed: {e}"))?;
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
}

impl MpdClient {
    /// Fetch album art binary data via MPD's `albumart` protocol.
    /// Response: `size: TOTAL\nbinary: CHUNK\n<data>\n...` (chunked), then `OK\n`.
    /// All reads go through the persistent `reader` so the same buffered
    /// stream is shared with `read_response` (binary data must not be
    /// pre-buffered separately on a different handle).
    fn album_art(&mut self, uri: &str) -> Result<Vec<u8>, String> {
        // MPD requires both URI and offset
        let cmd = format!("albumart {} 0\n", quote_arg(uri));
        self.stream
            .write_all(cmd.as_bytes())
            .map_err(|e| format!("MPD write failed: {e}"))?;

        // Read a single text line (up to \n) byte-by-byte.
        // Returns the line content without the trailing \n.
        fn read_line<R: Read>(r: &mut R) -> Result<String, String> {
            let mut out = Vec::with_capacity(64);
            loop {
                let mut buf = [0u8; 1];
                match r.read_exact(&mut buf) {
                    Ok(()) => {
                        if buf[0] == b'\n' {
                            return Ok(String::from_utf8_lossy(&out).into_owned());
                        }
                        out.push(buf[0]);
                    }
                    Err(e) => return Err(format!("MPD read failed: {e}")),
                }
            }
        }

        // Read "size: N" line (total size of all binary data)
        let size_line = read_line(&mut self.reader)?;
        if size_line.starts_with("ACK") {
            return Err(size_line);
        }
        let total: usize = size_line
            .strip_prefix("size: ")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("Expected size: header, got: {size_line}"))?;

        let mut data = Vec::with_capacity(total);
        // Read chunks until we have `total` bytes
        while data.len() < total {
            // Read "binary: CHUNK_SIZE" line
            let chunk_header = read_line(&mut self.reader)?;
            if chunk_header.starts_with("ACK") {
                return Err(chunk_header);
            }
            let chunk_size: usize = chunk_header
                .strip_prefix("binary: ")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("Expected binary: header, got: {chunk_header}"))?;

            // Read exactly `chunk_size` raw bytes
            let mut chunk = vec![0u8; chunk_size];
            self.reader
                .read_exact(&mut chunk)
                .map_err(|e| format!("MPD album art read error: {e}"))?;
            data.extend_from_slice(&chunk);
        }

        // Read the trailing "OK" line
        let ok_line = read_line(&mut self.reader)?;
        if ok_line != "OK" {
            return Err(format!("Expected OK after album art, got: {ok_line}"));
        }

        Ok(data)
    }
}

#[tauri::command]
fn get_library() -> Result<Vec<Track>, String> {
    load_mpd_library()
}

#[tauri::command]
fn get_mpd_status() -> MpdStatus {
    match read_mpd_status() {
        Ok(status) => status,
        Err(error) => MpdStatus {
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
        },
    }
}

#[tauri::command]
fn mpd_play_path(path: String) -> Result<(), String> {
    let mut mpd = MpdClient::connect()?;
    play_paths(&mut mpd, &[path], 0)
}

#[tauri::command]
fn mpd_play_paths(paths: Vec<String>, index: usize) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No tracks were sent to MPD".to_string());
    }
    let mut mpd = MpdClient::connect()?;
    play_paths(&mut mpd, &paths, index)
}

/// Replace MPD's queue with these paths and start playing at `index`.
/// MPD handles queues in the thousands, so we don't cap the path count
/// (capping to 500 silently clamped `index` to a different track when
/// the user picked a song past the cap).
#[tauri::command]
fn mpd_set_queue(paths: Vec<String>, index: usize) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No tracks were sent to MPD".to_string());
    }
    let mut mpd = MpdClient::connect()?;
    let index = index.min(paths.len() - 1);
    play_paths(&mut mpd, &paths, index)
}

#[tauri::command]
fn mpd_clear_queue() -> Result<(), String> {
    MpdClient::connect()?.command("clear")?;
    Ok(())
}

#[tauri::command]
fn mpd_play_idx(index: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("play {index}"))?;
    Ok(())
}

#[tauri::command]
fn mpd_delete_from_queue(index: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("delete {index}"))?;
    Ok(())
}

#[derive(Serialize)]
struct AlbumArt {
    mime: String,
    data: String,
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
        "image/jpeg" // best guess
    }
}

#[tauri::command]
fn get_album_art(path: String) -> Result<AlbumArt, String> {
    let mut mpd = MpdClient::connect()?;
    let data = mpd.album_art(&path)?;
    let mime = detect_mime(&data).to_string();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    Ok(AlbumArt { mime, data: b64 })
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

#[tauri::command]
fn mpd_toggle_play() -> Result<(), String> {
    let mut mpd = MpdClient::connect()?;
    let status = parse_pairs(&mpd.command("status")?);
    if status.get("state").is_some_and(|s| s == "play") {
        mpd.command("pause 1")?;
    } else {
        mpd.command("play")?;
    }
    Ok(())
}

#[tauri::command]
fn mpd_stop() -> Result<(), String> {
    MpdClient::connect()?.command("stop")?;
    Ok(())
}

#[tauri::command]
fn mpd_next() -> Result<(), String> {
    MpdClient::connect()?.command("next")?;
    Ok(())
}

#[tauri::command]
fn mpd_previous() -> Result<(), String> {
    MpdClient::connect()?.command("previous")?;
    Ok(())
}

#[tauri::command]
fn mpd_set_volume(volume: u32) -> Result<(), String> {
    let volume = volume.min(100);
    MpdClient::connect()?.command(&format!("setvol {volume}"))?;
    Ok(())
}

#[tauri::command]
fn mpd_seek_current(seconds: u32) -> Result<(), String> {
    MpdClient::connect()?.command(&format!("seekcur {seconds}"))?;
    Ok(())
}

#[tauri::command]
fn window_minimize(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?
        .minimize()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn window_toggle_maximize(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn window_close(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?
        .close()
        .map_err(|e| e.to_string())
}

fn load_mpd_library() -> Result<Vec<Track>, String> {
    let mut mpd = MpdClient::connect()?;
    let lines = mpd.command("listallinfo")?;
    Ok(parse_tracks(&lines))
}

// Album summary list. MPD 0.24 rejects multi-tag `list` queries (e.g.
// `list albumartist album date` returns ACK), so we derive this from
// `listallinfo` like the UI's `deriveCollections` does. The IPC contract
// is preserved: the caller gets a deduplicated list of (album, artist).
#[tauri::command]
fn get_albums() -> Result<Vec<AlbumSummary>, String> {
    let mut mpd = MpdClient::connect()?;
    let lines = mpd.command("listallinfo")?;
    Ok(summarize_albums(&parse_tracks(&lines)))
}

// Tracks for a specific album. `find albumartist "X" album "Y"` is O(album)
// and returns a small response.
#[tauri::command]
fn get_album_tracks(album_artist: String, album: String) -> Result<Vec<Track>, String> {
    let mut mpd = MpdClient::connect()?;
    let cmd = format!(
        "find albumartist {} album {}",
        quote_arg(&album_artist),
        quote_arg(&album)
    );
    let lines = mpd.command(&cmd)?;
    Ok(parse_tracks(&lines))
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
        state: status.get("state").cloned().unwrap_or_else(|| "stop".to_string()),
        elapsed,
        duration,
        volume: status.get("volume").and_then(|v| v.parse().ok()).unwrap_or(-1),
        file: song.get("file").cloned().unwrap_or_default(),
        title: song.get("Title").cloned().unwrap_or_default(),
        artist: song.get("Artist").cloned().unwrap_or_default(),
        album: song.get("Album").cloned().unwrap_or_default(),
        // MPD's `error` field is sticky (e.g. "Failed to enable output
        // ..."); surface it so the UI can show why audio isn't playing.
        error: status.get("error").cloned().unwrap_or_default(),
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

    Some(Track { title, artist, album, album_artist, genre, year, track_number, duration, path })
}

// Deduplicate (album, album_artist) pairs from a track list. Multi-disc
// releases share a single album name and collapse to one entry.
fn summarize_albums(tracks: &[Track]) -> Vec<AlbumSummary> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<AlbumSummary> = Vec::new();
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

fn parse_pairs(lines: &[String]) -> std::collections::HashMap<String, String> {
    lines
        .iter()
        .filter_map(|line| line.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn mpd_addr() -> String {
    // Default to IPv4 loopback. On some systems `localhost` resolves to
    // `::1` (IPv6) first, and MPD often only binds to 0.0.0.0 (IPv4),
    // so we'd silently fail to connect. 127.0.0.1 is unambiguous.
    let host = env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string());

    // [::1]:6600 or host:port — already complete
    if host.contains("]:") || (!host.contains("::") && host.contains(':')) {
        return host;
    }
    // Bare IPv6 address — wrap in brackets, append port
    if host.contains("::") && !host.starts_with('[') {
        return format!("[{}]:{}", host, port);
    }
    // Bracketed IPv6 without port: [::1] → [::1]:6600
    if host.starts_with('[') && !host.contains("]:") {
        return format!("{}:{}", host, port);
    }
    // Plain IPv4 or hostname — append default port
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


pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_library,
            get_mpd_status,
            mpd_play_path,
            mpd_play_paths,
            mpd_set_queue,
            mpd_clear_queue,
            mpd_play_idx,
            mpd_delete_from_queue,
            get_albums,
            get_album_tracks,
            mpd_toggle_play,
            get_album_art,
            mpd_stop,
            mpd_next,
            mpd_previous,
            mpd_set_volume,
            mpd_seek_current,
            window_minimize,
            window_toggle_maximize,
            window_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
