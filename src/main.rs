#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use iced::time;
use iced::widget::{button, column, container, image, row, scrollable, slider, text, text_input, Space};
use iced::{application, Element, Length, Size, Subscription, Task, Theme};
use musicbee_iced::{self as mpd, AlbumArt, MpdStatus, Track};
use std::collections::HashMap;
use std::time::Duration;

const TRACK_RENDER_LIMIT: usize = 1_200;

pub fn main() -> iced::Result {
    application(boot, update, view)
        .title("MusicBee Iced")
        .theme(Theme::Dark)
        .subscription(subscription)
        .window_size(Size::new(1280.0, 800.0))
        .centered()
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    LibraryLoaded(Result<Vec<Track>, String>),
    StatusLoaded(MpdStatus),
    AlbumArtLoaded(String, Result<AlbumArt, String>),
    CommandFinished(Result<(), String>),
    SearchChanged(String),
    TrackDoubleClicked(usize),
    Tick,
    PlayPause,
    Stop,
    Next,
    Previous,
    Shuffle,
    Repeat,
    VolumeChanged(u8),
}

#[derive(Debug, Default)]
struct App {
    tracks: Vec<Track>,
    visible: Vec<usize>,
    search: String,
    selected: Option<usize>,
    current: Option<usize>,
    status: Option<MpdStatus>,
    status_message: String,
    error: Option<String>,
    loading: bool,
    volume: u8,
    shuffle: bool,
    repeat: bool,
    album_art: HashMap<String, Option<image::Handle>>,
    wanted_art_path: Option<String>,
}

fn boot() -> (App, Task<Message>) {
    (
        App {
            loading: true,
            volume: 70,
            status_message: "Loading MPD library...".to_string(),
            ..App::default()
        },
        Task::batch([
            Task::perform(async { mpd::get_library() }, Message::LibraryLoaded),
            Task::perform(async { mpd::get_mpd_status() }, Message::StatusLoaded),
        ]),
    )
}

fn subscription(_app: &App) -> Subscription<Message> {
    time::every(Duration::from_secs(1)).map(|_| Message::Tick)
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::LibraryLoaded(result) => {
            app.loading = false;
            match result {
                Ok(mut tracks) => {
                    tracks.sort_by(|a, b| {
                        a.album_artist
                            .cmp(&b.album_artist)
                            .then(a.album.cmp(&b.album))
                            .then(a.track_number.cmp(&b.track_number))
                            .then(a.title.cmp(&b.title))
                    });
                    app.tracks = tracks;
                    rebuild_visible(app);
                    app.status_message = format!("{} tracks loaded", app.tracks.len());
                    app.error = None;
                    app.sync_current_from_status()
                }
                Err(error) => {
                    app.error = Some(error.clone());
                    app.status_message = "MPD library unavailable".to_string();
                    Task::none()
                }
            }
        }
        Message::StatusLoaded(status) => {
            app.volume = status.volume.clamp(0, 100) as u8;
            app.shuffle = status.random;
            app.repeat = status.repeat;
            app.status_message = if status.connected {
                if status.error.is_empty() {
                    "MPD connected".to_string()
                } else {
                    format!("MPD audio error: {}", status.error)
                }
            } else {
                format!("MPD disconnected: {}", status.error)
            };
            app.status = Some(status);
            app.sync_current_from_status()
        }
        Message::AlbumArtLoaded(path, result) => {
            let handle = result.ok().map(|art| image::Handle::from_bytes(art.data));
            app.album_art.insert(path, handle);
            Task::none()
        }
        Message::CommandFinished(result) => {
            if let Err(error) = result {
                app.error = Some(error);
            }
            Task::perform(async { mpd::get_mpd_status() }, Message::StatusLoaded)
        }
        Message::SearchChanged(query) => {
            app.search = query;
            rebuild_visible(app);
            Task::none()
        }
        Message::TrackDoubleClicked(index) => {
            app.selected = Some(index);
            let paths: Vec<String> = app.tracks.iter().map(|track| track.path.clone()).collect();
            let task = Task::perform(
                async move { mpd::mpd_set_queue(paths, index) },
                Message::CommandFinished,
            );
            Task::batch([app.ensure_art_for_index(index), task])
        }
        Message::Tick => Task::perform(async { mpd::get_mpd_status() }, Message::StatusLoaded),
        Message::PlayPause => Task::perform(async { mpd::mpd_toggle_play() }, Message::CommandFinished),
        Message::Stop => Task::perform(async { mpd::mpd_stop() }, Message::CommandFinished),
        Message::Next => Task::perform(async { mpd::mpd_next() }, Message::CommandFinished),
        Message::Previous => Task::perform(async { mpd::mpd_previous() }, Message::CommandFinished),
        Message::Shuffle => {
            let enabled = !app.shuffle;
            app.shuffle = enabled;
            Task::perform(async move { mpd::mpd_set_random(enabled) }, Message::CommandFinished)
        }
        Message::Repeat => {
            let enabled = !app.repeat;
            app.repeat = enabled;
            Task::perform(async move { mpd::mpd_set_repeat(enabled) }, Message::CommandFinished)
        }
        Message::VolumeChanged(volume) => {
            app.volume = volume;
            Task::perform(
                async move { mpd::mpd_set_volume(volume as u32) },
                Message::CommandFinished,
            )
        }
    }
}

fn view(app: &App) -> Element<'_, Message> {
    let header = row![
        text("MusicBee Iced").size(22),
        Space::new().width(Length::Fill),
        text(&app.status_message).size(14)
    ]
    .spacing(16)
    .padding(12);

    let search = text_input("Search title, artist, album...", &app.search)
        .on_input(Message::SearchChanged)
        .padding(8)
        .size(14);

    let library = column![
        row![
            text("Library").size(16),
            Space::new().width(Length::Fill),
            text(format!(
                "showing {} of {}",
                app.visible.len().min(TRACK_RENDER_LIMIT),
                app.visible.len()
            ))
            .size(12)
        ]
        .spacing(8),
        search,
        track_list(app)
    ]
    .spacing(8)
    .padding(12);

    let body = row![
        container(sidebar(app))
            .width(Length::Fixed(230.0))
            .height(Length::Fill),
        container(library).width(Length::Fill).height(Length::Fill)
    ]
    .height(Length::Fill);

    let mut content = column![header, body, playerbar(app)]
        .height(Length::Fill)
        .spacing(0);

    if let Some(error) = &app.error {
        content = content.push(
            container(text(error).size(13))
                .padding(8)
                .width(Length::Fill),
        );
    }

    container(content).width(Length::Fill).height(Length::Fill).into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let current = current_track(app);
    let art = album_art_view(app, Length::Fixed(184.0), Length::Fixed(184.0));
    column![
        text("Now Playing").size(16),
        art,
        text(current.map(|t| t.title.as_str()).unwrap_or("No track selected")).size(15),
        text(current.map(|t| t.artist.as_str()).unwrap_or("")).size(13),
        text(current.map(|t| t.album.as_str()).unwrap_or("")).size(12),
        Space::new().height(Length::Fixed(12.0)),
        text("Native iced UI").size(14),
        text("No webview, no GTK/WebKit display flush path, no DOM grid churn.").size(12),
    ]
    .spacing(8)
    .padding(12)
    .into()
}

fn track_list(app: &App) -> Element<'_, Message> {
    let mut rows = column![track_header()].spacing(0);
    for &index in app.visible.iter().take(TRACK_RENDER_LIMIT) {
        rows = rows.push(track_row(app, index));
    }
    if app.visible.len() > TRACK_RENDER_LIMIT {
        rows = rows.push(
            container(text("Search to narrow results; the native list renders the first 1200 matches."))
                .padding(10)
                .width(Length::Fill),
        );
    }
    scrollable(rows).height(Length::Fill).into()
}

fn track_header() -> Element<'static, Message> {
    row![
        text("#").width(Length::Fixed(40.0)),
        text("Title").width(Length::FillPortion(3)),
        text("Artist").width(Length::FillPortion(2)),
        text("Album").width(Length::FillPortion(2)),
        text("Time").width(Length::Fixed(60.0)),
    ]
    .spacing(12)
    .padding([6, 10])
    .into()
}

fn track_row(app: &App, index: usize) -> Element<'_, Message> {
    let track = &app.tracks[index];
    let prefix = if Some(index) == app.current { "▶" } else { "" };
    let title = if prefix.is_empty() {
        track.title.clone()
    } else {
        format!("{prefix} {}", track.title)
    };
    button(
        row![
            text(track.track_number.to_string()).width(Length::Fixed(40.0)),
            text(title).width(Length::FillPortion(3)),
            text(track.artist.clone()).width(Length::FillPortion(2)),
            text(track.album.clone()).width(Length::FillPortion(2)),
            text(track.duration.clone()).width(Length::Fixed(60.0)),
        ]
        .spacing(12)
        .padding([4, 10]),
    )
    .width(Length::Fill)
    .on_press(Message::TrackDoubleClicked(index))
    .into()
}

fn playerbar(app: &App) -> Element<'_, Message> {
    let status = app.status.as_ref();
    let playing = status.is_some_and(|s| s.state == "play");
    let current = current_track(app);
    let elapsed = status.map(|s| format_seconds(s.elapsed)).unwrap_or_else(|| "0:00".to_string());
    let duration = status
        .map(|s| format_seconds(s.duration))
        .or_else(|| current.map(|t| t.duration.clone()))
        .unwrap_or_else(|| "0:00".to_string());

    let now_playing = row![
        album_art_view(app, Length::Fixed(64.0), Length::Fixed(64.0)),
        column![
            text(current.map(|t| t.title.as_str()).unwrap_or("No track selected")).size(15),
            text(current.map(|t| t.artist.as_str()).unwrap_or("")).size(13),
            text(current.map(|t| t.album.as_str()).unwrap_or("")).size(12),
        ]
        .spacing(2)
    ]
    .spacing(12)
    .width(Length::FillPortion(3));

    let controls = column![
        row![
            button("⏮").on_press(Message::Previous),
            button(if playing { "⏸" } else { "▶" }).on_press(Message::PlayPause),
            button("⏹").on_press(Message::Stop),
            button("⏭").on_press(Message::Next),
        ]
        .spacing(8),
        row![text(elapsed), text(" / "), text(duration)].spacing(2),
    ]
    .align_x(iced::Alignment::Center)
    .spacing(6)
    .width(Length::FillPortion(2));

    let toggles = row![
        button(if app.shuffle { "Shuffle ON" } else { "Shuffle" }).on_press(Message::Shuffle),
        button(if app.repeat { "Repeat ON" } else { "Repeat" }).on_press(Message::Repeat),
        text("Vol"),
        slider(0..=100, app.volume, Message::VolumeChanged).width(Length::Fixed(130.0)),
        text(app.volume.to_string()).width(Length::Fixed(34.0)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center)
    .width(Length::FillPortion(3));

    container(row![now_playing, controls, toggles].spacing(20).padding(10))
        .width(Length::Fill)
        .height(Length::Fixed(92.0))
        .into()
}

fn album_art_view(app: &App, width: Length, height: Length) -> Element<'_, Message> {
    if let Some(track) = current_track(app).or_else(|| app.selected.and_then(|i| app.tracks.get(i))) {
        if let Some(Some(handle)) = app.album_art.get(&track.path) {
            return image(handle.clone()).width(width).height(height).into();
        }
    }

    container(text("♪").size(36))
        .width(width)
        .height(height)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn current_track(app: &App) -> Option<&Track> {
    app.current
        .and_then(|i| app.tracks.get(i))
        .or_else(|| app.selected.and_then(|i| app.tracks.get(i)))
}

impl App {
    fn sync_current_from_status(&mut self) -> Task<Message> {
        let current_path = self.status.as_ref().and_then(|status| {
            if status.file.is_empty() {
                None
            } else {
                Some(status.file.as_str())
            }
        });

        if let Some(path) = current_path {
            self.current = self.tracks.iter().position(|track| track.path == path);
            if let Some(index) = self.current {
                return self.ensure_art_for_index(index);
            }
        }

        Task::none()
    }

    fn ensure_art_for_index(&mut self, index: usize) -> Task<Message> {
        let Some(track) = self.tracks.get(index) else {
            return Task::none();
        };
        if self.album_art.contains_key(&track.path) {
            return Task::none();
        }
        if self.wanted_art_path.as_deref() == Some(track.path.as_str()) {
            return Task::none();
        }

        let path = track.path.clone();
        self.wanted_art_path = Some(path.clone());
        Task::perform(
            async move {
                let result = mpd::get_album_art(path.clone());
                (path, result)
            },
            |(path, result)| Message::AlbumArtLoaded(path, result),
        )
    }
}

fn rebuild_visible(app: &mut App) {
    let needle = app.search.trim().to_lowercase();
    app.visible = app
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            if needle.is_empty()
                || t.title.to_lowercase().contains(&needle)
                || t.artist.to_lowercase().contains(&needle)
                || t.album.to_lowercase().contains(&needle)
                || t.album_artist.to_lowercase().contains(&needle)
            {
                Some(i)
            } else {
                None
            }
        })
        .collect();
}

fn format_seconds(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
