#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! MusicBee-skinned native iced front-end for the MPD client. The layout and
//! palette mirror the original Tauri HTML/CSS skin (titlebar with coloured
//! navigator tabs, context toolbar, library tree + track grid + Now Playing
//! panel, and the MusicBee-style player bar).

mod style;

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, slider, text, text_input,
    Column, Row, Space,
};
use iced::widget::text::Wrapping;
use iced::{application, window, Alignment, Color, Element, Length, Padding, Subscription, Task, Theme};
use musicbee_iced::{self as mpd, AlbumArt, MpdStatus, Track};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

// Cap how many track rows are ever built at once. The album-centric views
// keep lists short (one album at a time); this only bounds the flat "Music"
// list so the widget tree never explodes and the UI stays responsive.
const TRACK_RENDER_LIMIT: usize = 350;
// Bounded concurrency for album-art fetches. MPD opens a fresh connection per
// request, so we stream covers in a few at a time instead of all at once.
const MAX_ART_INFLIGHT: usize = 6;

// ---- Column layout (matches the original grid template) -----------------
const W_NUM: Length = Length::Fixed(40.0);
const W_TITLE: Length = Length::FillPortion(5);
const W_ARTIST: Length = Length::FillPortion(3);
const W_ALBUM: Length = Length::FillPortion(3);
const W_ALBUMARTIST: Length = Length::FillPortion(3);
const W_GENRE: Length = Length::Fixed(96.0);
const W_YEAR: Length = Length::Fixed(48.0);
const W_RATING: Length = Length::Fixed(78.0);
const W_PLAYS: Length = Length::Fixed(48.0);
const W_TIME: Length = Length::Fixed(58.0);

// Top-level navigator. Browsing (albums/artists/genres/songs) lives inside
// the Library home, driven by the sidebar, so the navigator stays small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Music,
    NowPlaying,
    Queue,
}

const NAV_TABS: [(View, &str, &str, u32); 3] = [
    (View::Music, "\u{266B}", "Library", 0x2A6BC4),
    (View::NowPlaying, "\u{25BA}", "Now Playing", 0xC8402E),
    (View::Queue, "\u{2263}", "Queue", 0x6B7AB8),
];

// Library home sections, selected from the left sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Albums,
    Artists,
    Genres,
    Songs,
    Folders,
    Playlists,
}

const SECTIONS: [(Section, &str, &str); 6] = [
    (Section::Albums, "\u{25A4}", "Albums"),
    (Section::Artists, "\u{25CF}", "Artists"),
    (Section::Genres, "\u{266F}", "Genres"),
    (Section::Songs, "\u{266B}", "Songs"),
    (Section::Folders, "\u{25A3}", "Folders"),
    (Section::Playlists, "\u{2261}", "Playlists"),
];

fn view_title(app: &App) -> String {
    match app.view.0 {
        View::NowPlaying => "Now Playing".to_string(),
        View::Queue => "Queue".to_string(),
        View::Music => match app.section.0 {
            Section::Albums => "Albums".to_string(),
            Section::Artists => app
                .open_artist
                .clone()
                .unwrap_or_else(|| "Artists".to_string()),
            Section::Genres => "Genres".to_string(),
            Section::Songs => "Songs".to_string(),
            Section::Folders => "Folders".to_string(),
            Section::Playlists => "Playlists".to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Details,
    Grouped,
}

#[derive(Debug, Clone, Copy)]
struct TrackMeta {
    rating: u8,
    plays: u32,
}

#[derive(Debug, Clone)]
struct Album {
    key: String,
    album: String,
    album_artist: String,
    year: u32,
    genre: String,
    tracks: Vec<usize>,
}

#[derive(Debug, Clone)]
struct ArtistRow {
    name: String,
    albums: usize,
}

#[derive(Debug, Default)]
struct App {
    tracks: Vec<Track>,
    meta: Vec<TrackMeta>,
    view: ViewState,
    section: SectionState,
    search: String,
    quick_title: String,
    quick_artist: String,
    quick_album: String,
    selected: Option<usize>,
    current: Option<usize>,
    status: Option<MpdStatus>,
    status_message: String,
    error: Option<String>,
    loading: bool,
    volume: u8,
    shuffle: bool,
    repeat: bool,
    layout: LayoutState,
    album_art: HashMap<String, Option<image::Handle>>,
    artist_art: HashMap<String, Option<image::Handle>>,
    requested_art: HashSet<String>,
    art_pending: VecDeque<ArtJob>,
    art_inflight: usize,
    albums: Vec<Album>,
    artists: Vec<ArtistRow>,
    genres: Vec<(String, usize)>,
    folders: Vec<String>,
    open_artist: Option<String>,
    sel_genre: Option<String>,
    sel_folder: Option<String>,
    sel_album: Option<usize>,
    queue: Vec<Track>,
    queue_idx: i32,
    playlist_version: i32,
    seek_preview: Option<u32>,
}

// A pending artwork fetch. Albums carry the metadata needed for the Last.fm
// fallback; artists are fetched from Last.fm by name.
#[derive(Debug, Clone)]
enum ArtJob {
    Album {
        path: String,
        album_artist: String,
        album: String,
    },
    Artist {
        name: String,
    },
}

// Newtype wrappers so we can derive Default on App.
#[derive(Debug)]
struct ViewState(View);
impl Default for ViewState {
    fn default() -> Self {
        ViewState(View::Music)
    }
}
#[derive(Debug)]
struct SectionState(Section);
impl Default for SectionState {
    fn default() -> Self {
        SectionState(Section::Albums)
    }
}
#[derive(Debug)]
struct LayoutState(Layout);
impl Default for LayoutState {
    fn default() -> Self {
        LayoutState(Layout::Grouped)
    }
}
#[derive(Debug, Clone)]
enum Message {
    LibraryLoaded(Result<Vec<Track>, String>),
    StatusLoaded(MpdStatus),
    QueueLoaded(Result<Vec<Track>, String>),
    AlbumArtLoaded(String, Result<AlbumArt, String>),
    ArtistArtLoaded(String, Result<AlbumArt, String>),
    CommandFinished(Result<(), String>),
    SearchChanged(String),
    QuickTitle(String),
    QuickArtist(String),
    QuickAlbum(String),
    Tick,
    SelectView(View),
    SetSection(Section),
    OpenArtist(String),
    CloseArtist,
    SelectTrack(usize),
    PlayTrack(usize),
    SelectGenre(String),
    SelectFolder(String),
    SelectAlbum(usize),
    PlayAlbum(usize),
    SetLayout(Layout),
    PlayQueueIndex(usize),
    RemoveFromQueue(usize),
    ClearQueue,
    PlayPause,
    Stop,
    Next,
    Previous,
    Shuffle,
    Repeat,
    VolumeChanged(u8),
    SeekChanged(u32),
    SeekCommit,
    WinMinimize,
    WinToggleMax,
    WinClose,
    WinDrag,
}

pub fn main() -> iced::Result {
    application(boot, update, view)
        .title("MusicBee")
        .theme(Theme::Dark)
        .subscription(subscription)
        .window(window::Settings {
            decorations: false,
            size: iced::Size::new(1280.0, 800.0),
            min_size: Some(iced::Size::new(900.0, 600.0)),
            ..Default::default()
        })
        .centered()
        .run()
}

fn boot() -> (App, Task<Message>) {
    (
        App {
            loading: true,
            volume: 70,
            queue_idx: -1,
            playlist_version: -1,
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
    iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
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
                    app.rebuild_collections();
                    app.status_message = format!("{} tracks loaded", app.tracks.len());
                    app.error = None;
                    if app.sel_album.is_none() && !app.albums.is_empty() {
                        app.sel_album = Some(0);
                    }
                    // Begin streaming covers for every album.
                    for i in 0..app.albums.len() {
                        app.queue_album_cover(i);
                    }
                    Task::batch([app.sync_current_from_status(), app.pump_art()])
                }
                Err(error) => {
                    app.error = Some(error);
                    app.status_message = "MPD library unavailable".to_string();
                    Task::none()
                }
            }
        }
        Message::StatusLoaded(status) => {
            if app.seek_preview.is_none() {
                app.volume = status.volume.clamp(0, 100) as u8;
            }
            app.shuffle = status.random;
            app.repeat = status.repeat;
            app.queue_idx = status.song;
            app.status_message = if status.connected {
                if status.error.is_empty() {
                    "MPD connected".to_string()
                } else {
                    format!("MPD audio error: {}", status.error)
                }
            } else {
                format!("MPD disconnected: {}", status.error)
            };
            let version = status.playlist_version as i32;
            let connected = status.connected;
            app.status = Some(status);
            let mut tasks = vec![app.sync_current_from_status()];
            if connected && version != app.playlist_version {
                app.playlist_version = version;
                tasks.push(Task::perform(async { mpd::get_queue() }, Message::QueueLoaded));
            }
            Task::batch(tasks)
        }
        Message::QueueLoaded(result) => {
            if let Ok(tracks) = result {
                app.queue = tracks;
            }
            Task::none()
        }
        Message::AlbumArtLoaded(path, result) => {
            let handle = result.ok().map(|art| image::Handle::from_bytes(art.data));
            app.album_art.insert(path, handle);
            app.art_inflight = app.art_inflight.saturating_sub(1);
            app.pump_art()
        }
        Message::ArtistArtLoaded(name, result) => {
            let handle = result.ok().map(|art| image::Handle::from_bytes(art.data));
            app.artist_art.insert(name, handle);
            app.art_inflight = app.art_inflight.saturating_sub(1);
            app.pump_art()
        }
        Message::CommandFinished(result) => {
            if let Err(error) = result {
                app.error = Some(error);
            }
            Task::perform(async { mpd::get_mpd_status() }, Message::StatusLoaded)
        }
        Message::SearchChanged(query) => {
            app.search = query;
            Task::none()
        }
        Message::QuickTitle(v) => {
            app.quick_title = v;
            Task::none()
        }
        Message::QuickArtist(v) => {
            app.quick_artist = v;
            Task::none()
        }
        Message::QuickAlbum(v) => {
            app.quick_album = v;
            Task::none()
        }
        Message::Tick => Task::perform(async { mpd::get_mpd_status() }, Message::StatusLoaded),
        Message::SelectView(v) => {
            app.view = ViewState(v);
            Task::none()
        }
        Message::SetSection(section) => {
            app.section = SectionState(section);
            app.view = ViewState(View::Music);
            app.open_artist = None;
            // Lazily request artist images the first time Artists is opened.
            if section == Section::Artists {
                let names: Vec<String> = app.artists.iter().map(|a| a.name.clone()).collect();
                for name in names {
                    app.queue_artist_image(name);
                }
                return app.pump_art();
            }
            Task::none()
        }
        Message::OpenArtist(name) => {
            app.open_artist = Some(name.clone());
            // Select the artist's first album and ensure those covers load.
            if let Some(idx) = app.albums.iter().position(|a| a.album_artist == name) {
                app.sel_album = Some(idx);
            }
            let album_indices: Vec<usize> = app
                .albums
                .iter()
                .enumerate()
                .filter(|(_, a)| a.album_artist == name)
                .map(|(i, _)| i)
                .collect();
            for i in album_indices {
                app.queue_album_cover(i);
            }
            app.pump_art()
        }
        Message::CloseArtist => {
            app.open_artist = None;
            Task::none()
        }
        Message::SelectTrack(index) => {
            app.selected = Some(index);
            app.ensure_art_for_index(index)
        }
        Message::PlayTrack(index) => {
            app.selected = Some(index);
            app.current = Some(index);
            let context = app.play_context_indices(index);
            let position = context.iter().position(|&i| i == index).unwrap_or(0);
            let paths: Vec<String> = context.iter().map(|&i| app.tracks[i].path.clone()).collect();
            let task = Task::perform(
                async move { mpd::mpd_set_queue(paths, position) },
                Message::CommandFinished,
            );
            Task::batch([app.ensure_art_for_index(index), task])
        }
        Message::SelectGenre(name) => {
            app.sel_genre = Some(name);
            Task::none()
        }
        Message::SelectFolder(path) => {
            app.sel_folder = Some(path);
            Task::none()
        }
        Message::SelectAlbum(index) => {
            app.sel_album = Some(index);
            app.queue_album_cover(index);
            app.pump_art()
        }
        Message::PlayAlbum(index) => {
            if let Some(album) = app.albums.get(index) {
                let paths: Vec<String> = album
                    .tracks
                    .iter()
                    .map(|&i| app.tracks[i].path.clone())
                    .collect();
                if let Some(&first) = album.tracks.first() {
                    app.current = Some(first);
                    app.selected = Some(first);
                }
                return Task::perform(
                    async move { mpd::mpd_set_queue(paths, 0) },
                    Message::CommandFinished,
                );
            }
            Task::none()
        }
        Message::SetLayout(layout) => {
            app.layout = LayoutState(layout);
            Task::none()
        }
        Message::PlayQueueIndex(index) => Task::perform(
            async move { mpd::mpd_play_idx(index as u32) },
            Message::CommandFinished,
        ),
        Message::RemoveFromQueue(index) => {
            if index < app.queue.len() {
                app.queue.remove(index);
            }
            Task::perform(
                async move { mpd::mpd_delete_from_queue(index as u32) },
                Message::CommandFinished,
            )
        }
        Message::ClearQueue => {
            app.queue.clear();
            app.queue_idx = -1;
            Task::perform(async { mpd::mpd_clear_queue() }, Message::CommandFinished)
        }
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
        Message::SeekChanged(seconds) => {
            app.seek_preview = Some(seconds);
            Task::none()
        }
        Message::SeekCommit => {
            let seconds = app.seek_preview.take().unwrap_or(0);
            Task::perform(
                async move { mpd::mpd_seek_current(seconds) },
                Message::CommandFinished,
            )
        }
        Message::WinMinimize => window::latest().and_then(|id| window::minimize(id, true)),
        Message::WinToggleMax => window::latest().and_then(window::toggle_maximize),
        Message::WinClose => window::latest().and_then(window::close),
        Message::WinDrag => window::latest().and_then(window::drag),
    }
}

// ===================================================================
//  View
// ===================================================================
fn view(app: &App) -> Element<'_, Message> {
    let body: Element<Message> = match app.view.0 {
        View::Music => music_view(app),
        View::NowPlaying => now_playing_full(app),
        View::Queue => queue_view(app),
    };

    let content = column![
        titlebar(app),
        contextbar(app),
        container(body).width(Length::Fill).height(Length::Fill),
        playerbar(app),
    ]
    .spacing(0);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::root)
        .into()
}

// ---- Titlebar -----------------------------------------------------------
fn titlebar(app: &App) -> Element<'_, Message> {
    let brand = mouse_area(
        container(text("MusicBee MPD").size(14).color(style::rgb(0xF0F0F0)))
            .padding(Padding::from([0, 10]))
            .center_y(Length::Fill),
    )
    .on_press(Message::WinDrag);

    let mut tabs = Row::new().spacing(1).align_y(Alignment::End).height(Length::Fill);
    for (v, icon, label, accent) in NAV_TABS.iter() {
        let active = app.view.0 == *v;
        let accent_color = style::rgb(*accent);
        let label_row = row![
            text(*icon).size(13).color(accent_color),
            text(*label).size(11),
        ]
        .spacing(5)
        .align_y(Alignment::Center);
        tabs = tabs.push(
            button(label_row)
                .padding(Padding::from([5, 8]))
                .height(Length::Fixed(31.0))
                .on_press(Message::SelectView(*v))
                .style(move |_t, s| style::nav_tab(s, active, accent_color)),
        );
    }

    let (connected, has_error) = app
        .status
        .as_ref()
        .map(|s| (s.connected, !s.error.is_empty()))
        .unwrap_or((false, false));
    let pill_label = if connected {
        if has_error {
            "MPD: audio error"
        } else {
            "MPD 127.0.0.1:6600"
        }
    } else {
        "MPD disconnected"
    };
    let pill = container(text(pill_label).size(11))
        .padding(Padding::from([3, 9]))
        .center_y(Length::Fixed(22.0))
        .style(style::status_pill(connected, has_error));

    let controls = row![
        window_button("\u{2014}", false, Message::WinMinimize),
        window_button("\u{25A1}", false, Message::WinToggleMax),
        window_button("\u{2715}", true, Message::WinClose),
    ];

    container(
        row![
            brand,
            container(tabs).width(Length::Fill).height(Length::Fill),
            container(pill).center_y(Length::Fill).padding(Padding::from([0, 8])),
            controls,
        ]
        .align_y(Alignment::Center)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(38.0))
    .style(style::titlebar)
    .into()
}

fn window_button(glyph: &str, close: bool, msg: Message) -> Element<'_, Message> {
    button(
        container(text(glyph.to_string()).size(13))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(44.0))
    .height(Length::Fixed(38.0))
    .on_press(msg)
    .style(move |_t, s| style::window_btn(s, close))
    .into()
}

// ---- Context toolbar ----------------------------------------------------
fn contextbar(app: &App) -> Element<'_, Message> {
    let layout = app.layout.0;
    let seg = row![
        seg_button("\u{2630}", layout == Layout::Details, Message::SetLayout(Layout::Details)),
        seg_button("\u{229E}", layout == Layout::Grouped, Message::SetLayout(Layout::Grouped)),
    ]
    .spacing(1);

    let left = row![
        seg,
        container(text("").size(1)).width(Length::Fixed(2.0)),
        text(view_title(app)).size(13).color(style::rgb(0xEAEAEA)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let search = container(
        text_input("Search library", &app.search)
            .on_input(Message::SearchChanged)
            .padding(Padding::from([4, 8]))
            .size(13)
            .width(Length::Fixed(240.0))
            .style(style::search_input),
    );

    container(
        row![left, Space::new().width(Length::Fill), search]
            .align_y(Alignment::Center)
            .padding(Padding::from([0, 8])),
    )
    .width(Length::Fill)
    .height(Length::Fixed(32.0))
    .style(style::contextbar)
    .into()
}

fn seg_button(glyph: &str, active: bool, msg: Message) -> Element<'_, Message> {
    button(
        container(text(glyph.to_string()).size(13))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(32.0))
    .height(Length::Fixed(24.0))
    .on_press(msg)
    .style(move |_t, s| style::toolbar_btn(s, active))
    .into()
}

// ---- Library home (sidebar + section content) ---------------------------
fn music_view(app: &App) -> Element<'_, Message> {
    let sidebar = container(library_sidebar(app))
        .width(Length::Fixed(190.0))
        .height(Length::Fill)
        .style(style::sidebar);

    let content: Element<Message> = match app.section.0 {
        Section::Albums => albums_section(app),
        Section::Artists => artists_section(app),
        Section::Genres => genres_section(app),
        Section::Songs => songs_section(app),
        Section::Folders => folders_section(app),
        Section::Playlists => playlists_view(app),
    };

    row![
        sidebar,
        container(content).width(Length::Fill).height(Length::Fill).style(style::content),
    ]
    .height(Length::Fill)
    .into()
}

fn library_sidebar(app: &App) -> Element<'_, Message> {
    let header = container(text("Library").size(12).color(style::rgb(style::TEXT)))
        .padding(Padding::from([6, 10]))
        .width(Length::Fill)
        .style(style::grid_header);

    let mut items = Column::new().spacing(2).padding(Padding::from([6, 6]));
    for (section, icon, label) in SECTIONS.iter() {
        let selected = app.section.0 == *section;
        items = items.push(library_button(icon, label, selected, Message::SetSection(*section)));
    }

    column![header, scrollable(items).height(Length::Fill).style(style::scroller)]
        .height(Length::Fill)
        .into()
}

fn library_button(icon: &str, label: &str, selected: bool, msg: Message) -> Element<'static, Message> {
    button(
        row![
            text(icon.to_string()).size(13).width(Length::Fixed(18.0)),
            text(label.to_string()).size(13),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([6, 10]))
    .on_press(msg)
    .style(move |_t, s| style::list_item(s, selected))
    .into()
}

// Albums section: cover grid on the left, selected-album detail on the right.
fn albums_section(app: &App) -> Element<'_, Message> {
    let needle = app.search.trim().to_lowercase();
    let shown: Vec<usize> = app
        .albums
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            needle.is_empty()
                || a.album.to_lowercase().contains(&needle)
                || a.album_artist.to_lowercase().contains(&needle)
        })
        .map(|(i, _)| i)
        .collect();

    let header = section_header("Albums", &format!("{} albums", shown.len()));
    let left = container(column![
        header,
        scrollable(cover_grid(app, &shown, 4)).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ])
    .width(Length::Fill)
    .height(Length::Fill);

    let right = container(album_detail(app))
        .width(Length::Fixed(360.0))
        .height(Length::Fill)
        .style(style::now_playing_panel);

    row![left, right].height(Length::Fill).into()
}

// Artists section: a grid of artist images, or one artist's albums when opened.
fn artists_section(app: &App) -> Element<'_, Message> {
    if let Some(name) = app.open_artist.clone() {
        return artist_detail(app, &name);
    }

    let needle = app.search.trim().to_lowercase();
    let shown: Vec<&ArtistRow> = app
        .artists
        .iter()
        .filter(|a| needle.is_empty() || a.name.to_lowercase().contains(&needle))
        .collect();

    let header = section_header("Artists", &format!("{} artists", shown.len()));

    const PER_ROW: usize = 5;
    let mut grid = Column::new().spacing(16).padding(14);
    let mut current_row = Row::new().spacing(14);
    let mut in_row = 0;
    for a in &shown {
        current_row = current_row.push(artist_card(app, a));
        in_row += 1;
        if in_row == PER_ROW {
            grid = grid.push(current_row);
            current_row = Row::new().spacing(14);
            in_row = 0;
        }
    }
    if in_row > 0 {
        for _ in in_row..PER_ROW {
            current_row = current_row.push(Space::new().width(Length::FillPortion(1)));
        }
        grid = grid.push(current_row);
    }

    column![
        header,
        scrollable(grid).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ]
    .height(Length::Fill)
    .into()
}

fn artist_detail<'a>(app: &'a App, name: &str) -> Element<'a, Message> {
    let album_indices: Vec<usize> = app
        .albums
        .iter()
        .enumerate()
        .filter(|(_, a)| a.album_artist == name)
        .map(|(i, _)| i)
        .collect();

    let header = container(
        row![
            button(text("\u{2190} Artists").size(12))
                .padding(Padding::from([4, 10]))
                .on_press(Message::CloseArtist)
                .style(|_t, s| style::toolbar_btn(s, false)),
            artist_avatar(app, name, 64.0),
            column![
                text(name.to_string()).size(18).color(style::rgb(style::TEXT)).wrapping(Wrapping::None),
                text(format!("{} albums", album_indices.len())).size(12).color(style::rgb(style::TEXT_DIM)),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::from([8, 12])),
    )
    .width(Length::Fill)
    .style(style::grid_header);

    let left = container(column![
        header,
        scrollable(cover_grid(app, &album_indices, 4)).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ])
    .width(Length::Fill)
    .height(Length::Fill);

    let right = container(album_detail(app))
        .width(Length::Fixed(360.0))
        .height(Length::Fill)
        .style(style::now_playing_panel);

    row![left, right].height(Length::Fill).into()
}

// Genres section: a row of genre chips that filters an album grid.
fn genres_section(app: &App) -> Element<'_, Message> {
    let mut chips = Row::new().spacing(6);
    chips = chips.push(genre_chip("All", app.sel_genre.is_none(), Message::SelectGenre("*".to_string())));
    for (name, _count) in &app.genres {
        if name.is_empty() {
            continue;
        }
        let selected = app.sel_genre.as_deref() == Some(name.as_str());
        chips = chips.push(genre_chip(name, selected, Message::SelectGenre(name.clone())));
    }

    let active = app.sel_genre.clone().filter(|g| g != "*");
    let shown: Vec<usize> = app
        .albums
        .iter()
        .enumerate()
        .filter(|(_, a)| active.as_deref().map(|g| a.genre == g).unwrap_or(true))
        .map(|(i, _)| i)
        .collect();

    let header = container(
        column![
            row![
                text("Genres").size(13).color(style::rgb(style::TEXT)),
                text(format!("   {} albums", shown.len())).size(11).color(style::rgb(style::TEXT_DIM)),
            ]
            .align_y(Alignment::Center),
            scrollable(chips).width(Length::Fill),
        ]
        .spacing(6)
        .padding(Padding::from([6, 10])),
    )
    .width(Length::Fill)
    .style(style::grid_header);

    let left = container(column![
        header,
        scrollable(cover_grid(app, &shown, 4)).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ])
    .width(Length::Fill)
    .height(Length::Fill);

    let right = container(album_detail(app))
        .width(Length::Fixed(360.0))
        .height(Length::Fill)
        .style(style::now_playing_panel);

    row![left, right].height(Length::Fill).into()
}

fn genre_chip(label: &str, selected: bool, msg: Message) -> Element<'static, Message> {
    button(text(label.to_string()).size(11))
        .padding(Padding::from([4, 10]))
        .on_press(msg)
        .style(move |_t, s| style::toggle_btn(s, selected))
        .into()
}

// Songs section: the flat track list (capped) with quick filters.
fn songs_section(app: &App) -> Element<'_, Message> {
    let indices = app.visible_indices();
    column![
        quickfilter(app),
        container(track_grid(app, &indices, app.layout.0 == Layout::Grouped))
            .width(Length::Fill)
            .height(Length::Fill),
        statusbar(app, indices.len()),
    ]
    .height(Length::Fill)
    .into()
}

// Folders section: directory list on the left, its tracks on the right.
fn folders_section(app: &App) -> Element<'_, Message> {
    let mut list = Column::new().spacing(1).padding(Padding::from([4, 4]));
    list = list.push(list_item_btn(
        "All Folders".to_string(),
        app.sel_folder.is_none(),
        Message::SelectFolder("*".to_string()),
    ));
    for folder in &app.folders {
        let selected = app.sel_folder.as_deref() == Some(folder.as_str());
        let label = folder.rsplit('/').next().unwrap_or(folder).to_string();
        list = list.push(list_item_btn(label, selected, Message::SelectFolder(folder.clone())));
    }
    let left = container(column![
        section_header("Folders", &format!("{} folders", app.folders.len())),
        scrollable(list).height(Length::Fill).style(style::scroller),
    ])
    .width(Length::Fixed(240.0))
    .height(Length::Fill)
    .style(style::sidebar);

    let indices = app.visible_indices();
    let center = column![
        container(track_grid(app, &indices, false)).width(Length::Fill).height(Length::Fill),
        statusbar(app, indices.len()),
    ];

    row![left, container(center).width(Length::Fill).height(Length::Fill)]
        .height(Length::Fill)
        .into()
}

fn section_header<'a>(title: &str, sub: &str) -> Element<'a, Message> {
    container(
        row![
            text(title.to_string()).size(13).color(style::rgb(style::TEXT)),
            text(format!("   {sub}")).size(11).color(style::rgb(style::TEXT_DIM)),
        ]
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 10]))
    .width(Length::Fill)
    .style(style::grid_header)
    .into()
}

// Shared album cover grid used by Albums / Artists detail / Genres.
fn cover_grid<'a>(app: &'a App, indices: &[usize], per_row: usize) -> Element<'a, Message> {
    let mut grid = Column::new().spacing(16).padding(14);
    let mut current_row = Row::new().spacing(14);
    let mut in_row = 0;
    for &i in indices {
        current_row = current_row.push(album_card(app, i, &app.albums[i]));
        in_row += 1;
        if in_row == per_row {
            grid = grid.push(current_row);
            current_row = Row::new().spacing(14);
            in_row = 0;
        }
    }
    if in_row > 0 {
        for _ in in_row..per_row {
            current_row = current_row.push(Space::new().width(Length::FillPortion(1)));
        }
        grid = grid.push(current_row);
    }
    grid.into()
}

// An artist card: image (Last.fm / first album cover / tile) + name.
fn artist_card<'a>(app: &'a App, artist: &'a ArtistRow) -> Element<'a, Message> {
    let card = column![
        artist_avatar(app, &artist.name, 150.0),
        text(artist.name.clone()).size(12).color(style::rgb(style::TEXT)).wrapping(Wrapping::None),
        text(format!("{} albums", artist.albums)).size(10).color(style::rgb(style::TEXT_DIM)),
    ]
    .spacing(4)
    .align_x(Alignment::Center)
    .width(Length::FillPortion(1));

    mouse_area(container(card).padding(2))
        .on_press(Message::OpenArtist(artist.name.clone()))
        .into()
}

// Artist image: Last.fm artist art, falling back to their first album cover,
// then a coloured initial tile.
fn artist_avatar<'a>(app: &'a App, name: &str, size: f32) -> Element<'a, Message> {
    if let Some(Some(handle)) = app.artist_art.get(name) {
        return container(
            image(handle.clone())
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .content_fit(iced::ContentFit::Cover),
        )
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .clip(true)
        .style(style::art_tile(style::rgb(style::BORDER_DK)))
        .into();
    }
    // Fall back to the artist's first album cover if we have it.
    if let Some(album) = app.albums.iter().find(|a| a.album_artist == name) {
        if let Some(t) = album.tracks.first().and_then(|&i| app.tracks.get(i)) {
            if let Some(Some(handle)) = app.album_art.get(&t.path) {
                return container(
                    image(handle.clone())
                        .width(Length::Fixed(size))
                        .height(Length::Fixed(size))
                        .content_fit(iced::ContentFit::Cover),
                )
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .clip(true)
                .style(style::art_tile(style::rgb(style::BORDER_DK)))
                .into();
            }
        }
    }
    let (color, initial) = tile_for(name, name);
    container(text(initial).size(size * 0.4).color(Color::from_rgba(1.0, 1.0, 1.0, 0.92)))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(style::art_tile(color))
        .into()
}

fn quickfilter(app: &App) -> Element<'_, Message> {
    let mk = |placeholder: &str, value: &str, on: fn(String) -> Message| {
        text_input(placeholder, value)
            .on_input(on)
            .padding(Padding::from([3, 6]))
            .size(12)
            .width(Length::Fixed(130.0))
            .style(style::search_input)
    };
    container(
        row![
            text("Filter:").size(12).color(style::rgb(style::TEXT)),
            mk("Title", &app.quick_title, Message::QuickTitle),
            mk("Artist", &app.quick_artist, Message::QuickArtist),
            mk("Album", &app.quick_album, Message::QuickAlbum),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .padding(Padding::from([6, 10])),
    )
    .width(Length::Fill)
    .style(style::sidebar)
    .into()
}

fn statusbar(app: &App, count: usize) -> Element<'_, Message> {
    let playing = app
        .current
        .and_then(|i| app.tracks.get(i))
        .map(|t| format!("Playing: {} \u{2014} {}", t.title, t.artist))
        .unwrap_or_else(|| "Ready".to_string());
    container(
        row![
            text(playing).size(11).color(style::rgb(0xDADADA)),
            Space::new().width(Length::Fill),
            text(format!("{count} tracks")).size(11).color(style::rgb(style::TEXT_DIM)),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::from([0, 10])),
    )
    .width(Length::Fill)
    .height(Length::Fixed(22.0))
    .style(style::statusbar)
    .into()
}

// ---- Track grid ---------------------------------------------------------
fn track_grid<'a>(app: &'a App, indices: &[usize], grouped: bool) -> Element<'a, Message> {
    let header = container(grid_header_row()).width(Length::Fill).style(style::grid_header);

    let mut rows = Column::new().spacing(0);
    let mut last_album: Option<String> = None;
    let mut parity = 0u8;
    for (n, &index) in indices.iter().take(TRACK_RENDER_LIMIT).enumerate() {
        let track = &app.tracks[index];
        if grouped {
            let key = album_key(track);
            if last_album.as_deref() != Some(key.as_str()) {
                rows = rows.push(group_header_row(app, &key, track));
                last_album = Some(key);
                parity = 0;
            }
        }
        rows = rows.push(track_row(app, index, n + 1, parity));
        parity ^= 1;
    }
    if indices.len() > TRACK_RENDER_LIMIT {
        rows = rows.push(
            container(
                text(format!(
                    "Showing first {TRACK_RENDER_LIMIT} of {} matches \u{2014} refine your search.",
                    indices.len()
                ))
                .size(11)
                .color(style::rgb(style::TEXT_DIM)),
            )
            .padding(10),
        );
    }

    column![
        header,
        scrollable(rows).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ]
    .height(Length::Fill)
    .into()
}

fn grid_header_row() -> Element<'static, Message> {
    let h = |label: &str, w: Length| -> Element<'static, Message> {
        container(text(label.to_string()).size(12).color(style::rgb(style::ACCENT_2)).wrapping(Wrapping::None))
            .width(w)
            .padding(Padding::from([5, 7]))
            .clip(true)
            .into()
    };
    Row::with_children(vec![
        h("#", W_NUM),
        h("Title", W_TITLE),
        h("Artist", W_ARTIST),
        h("Album", W_ALBUM),
        h("Album Artist", W_ALBUMARTIST),
        h("Genre", W_GENRE),
        h("Year", W_YEAR),
        h("Rating", W_RATING),
        h("Plays", W_PLAYS),
        h("Time", W_TIME),
    ])
    .into()
}

fn track_row<'a>(app: &'a App, index: usize, display_num: usize, parity: u8) -> Element<'a, Message> {
    let track = &app.tracks[index];
    let meta = app.meta.get(index).copied().unwrap_or(TrackMeta { rating: 0, plays: 0 });
    let is_selected = app.selected == Some(index);
    let is_playing = app.current == Some(index);
    let kind = if is_selected {
        2
    } else if is_playing {
        3
    } else {
        parity
    };
    let dim = !(is_selected);

    let num_label = if is_playing {
        format!("\u{25B6} {display_num}")
    } else {
        display_num.to_string()
    };

    let cells = Row::with_children(vec![
        cell(num_label, W_NUM, true, true),
        cell(track.title.clone(), W_TITLE, false, dim),
        cell(track.artist.clone(), W_ARTIST, false, dim),
        cell(track.album.clone(), W_ALBUM, false, dim),
        cell(track.album_artist.clone(), W_ALBUMARTIST, false, dim),
        cell(track.genre.clone(), W_GENRE, false, true),
        cell(if track.year > 0 { track.year.to_string() } else { String::new() }, W_YEAR, true, true),
        cell(stars(meta.rating), W_RATING, false, false),
        cell(meta.plays.to_string(), W_PLAYS, true, true),
        cell(track.duration.clone(), W_TIME, true, dim),
    ]);

    let row_container = container(cells)
        .width(Length::Fill)
        .style(style::row(kind));

    mouse_area(row_container)
        .on_press(Message::SelectTrack(index))
        .on_double_click(Message::PlayTrack(index))
        .into()
}

fn cell<'a>(value: String, w: Length, right: bool, dim: bool) -> Element<'a, Message> {
    let color = if dim { style::rgb(style::TEXT_DIM) } else { style::rgb(style::TEXT) };
    let mut t = text(value).size(12).color(color).wrapping(Wrapping::None);
    if right {
        t = t.align_x(iced::alignment::Horizontal::Right).width(Length::Fill);
    }
    container(t)
        .width(w)
        .padding(Padding::from([4, 7]))
        .clip(true)
        .into()
}

fn group_header_row<'a>(app: &'a App, key: &str, track: &'a Track) -> Element<'a, Message> {
    let count = app
        .albums
        .iter()
        .find(|a| a.key == key)
        .map(|a| a.tracks.len())
        .unwrap_or(1);
    let tile = small_art(app, Some(track), 46.0);
    let year = if track.year > 0 { format!("  ({})", track.year) } else { String::new() };
    let info = column![
        text(format!("{}{}", track.album, year)).size(13).color(style::rgb(style::ACCENT_2)).wrapping(Wrapping::None),
        text(track.album_artist.clone()).size(11).color(style::rgb(style::TEXT_DIM)).wrapping(Wrapping::None),
    ]
    .spacing(1);
    container(
        row![
            tile,
            info,
            Space::new().width(Length::Fill),
            text(format!("{count} tracks")).size(11).color(style::rgb(style::TEXT_DIM)),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .padding(Padding::from([6, 10])),
    )
    .width(Length::Fill)
    .style(style::group_header)
    .into()
}

fn list_item_btn(label: String, selected: bool, msg: Message) -> Element<'static, Message> {
    button(text(label).size(12).wrapping(Wrapping::None))
        .width(Length::Fill)
        .padding(Padding::from([4, 10]))
        .on_press(msg)
        .style(move |_t, s| style::list_item(s, selected))
        .into()
}

fn album_card<'a>(app: &'a App, index: usize, album: &'a Album) -> Element<'a, Message> {
    let selected = app.sel_album == Some(index);
    let card = column![
        album_cover(app, album, Length::Fill, 160.0),
        text(album.album.clone()).size(12).color(style::rgb(style::TEXT)).wrapping(Wrapping::None),
        text(album.album_artist.clone()).size(11).color(style::rgb(style::TEXT_DIM)).wrapping(Wrapping::None),
        text(format!("{}  \u{00b7}  {} trk", album.year, album.tracks.len()))
            .size(10)
            .color(style::rgb(style::TEXT_DIM)),
    ]
    .spacing(3)
    .width(Length::FillPortion(1));

    let wrapped = if selected {
        container(card).padding(2).style(|_t: &Theme| container::Style {
            border: iced::Border {
                color: style::rgb(style::ACCENT),
                width: 2.0,
                radius: iced::border::Radius::from(3.0),
            },
            ..Default::default()
        })
    } else {
        container(card).padding(2)
    };

    mouse_area(wrapped)
        .on_press(Message::SelectAlbum(index))
        .on_double_click(Message::PlayAlbum(index))
        .into()
}

// Album artwork that uses the real cover when loaded, else a coloured tile.
fn album_cover<'a>(app: &'a App, album: &'a Album, width: Length, height: f32) -> Element<'a, Message> {
    if let Some(t) = album.tracks.first().and_then(|&i| app.tracks.get(i)) {
        if let Some(Some(handle)) = app.album_art.get(&t.path) {
            return container(
                image(handle.clone())
                    .width(width)
                    .height(Length::Fixed(height))
                    .content_fit(iced::ContentFit::Cover),
            )
            .width(width)
            .height(Length::Fixed(height))
            .clip(true)
            .style(style::art_tile(style::rgb(style::BORDER_DK)))
            .into();
        }
    }
    let (color, initial) = tile_for(&album.key, &album.album);
    let label = if album.genre.is_empty() { String::new() } else { album.genre.to_uppercase() };
    container(
        column![
            text(initial).size(44).color(Color::from_rgba(1.0, 1.0, 1.0, 0.92)),
            text(label).size(9).color(Color::from_rgba(1.0, 1.0, 1.0, 0.75)),
        ]
        .align_x(Alignment::Center)
        .spacing(2),
    )
    .width(width)
    .height(Length::Fixed(height))
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(style::art_tile(color))
    .into()
}

// Right-hand detail pane: the selected album's cover, info, and track list.
fn album_detail(app: &App) -> Element<'_, Message> {
    let Some(index) = app.sel_album else {
        return container(text("Select an album").size(13).color(style::rgb(style::TEXT_DIM)))
            .padding(16)
            .into();
    };
    let Some(album) = app.albums.get(index) else {
        return container(text("Select an album").size(13).color(style::rgb(style::TEXT_DIM)))
            .padding(16)
            .into();
    };

    let head = column![
        album_cover(app, album, Length::Fill, 280.0),
        text(album.album.clone()).size(16).color(style::rgb(style::TEXT)).wrapping(Wrapping::None),
        text(album.album_artist.clone()).size(13).color(style::rgb(0xD0D0D0)).wrapping(Wrapping::None),
        text(format!(
            "{}  \u{00b7}  {} tracks",
            if album.year > 0 { album.year.to_string() } else { "\u{2014}".into() },
            album.tracks.len()
        ))
        .size(11)
        .color(style::rgb(style::TEXT_DIM)),
        button(text("\u{25B6} Play album").size(12))
            .padding(Padding::from([5, 10]))
            .on_press(Message::PlayAlbum(index))
            .style(|_t, s| style::transport_btn(s, true)),
    ]
    .spacing(6)
    .padding(12);

    let mut list = Column::new().spacing(0);
    for (n, &ti) in album.tracks.iter().enumerate() {
        list = list.push(album_track_row(app, ti, n + 1));
    }

    column![
        head,
        scrollable(list).height(Length::Fill).style(style::scroller),
    ]
    .height(Length::Fill)
    .into()
}

fn album_track_row(app: &App, index: usize, num: usize) -> Element<'_, Message> {
    let track = &app.tracks[index];
    let is_selected = app.selected == Some(index);
    let is_playing = app.current == Some(index);
    let kind = if is_selected { 2 } else if is_playing { 3 } else { 0 };
    let num_label = if is_playing { format!("\u{25B6}{num}") } else { num.to_string() };
    let cells = row![
        text(num_label).size(12).color(style::rgb(style::TEXT_DIM)).width(Length::Fixed(30.0)),
        text(track.title.clone()).size(12).color(style::rgb(style::TEXT)).wrapping(Wrapping::None).width(Length::Fill),
        text(track.duration.clone()).size(12).color(style::rgb(style::TEXT_DIM)).width(Length::Fixed(46.0)).align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(6)
    .padding(Padding::from([4, 10]));
    mouse_area(container(cells).width(Length::Fill).style(style::row(kind)))
        .on_press(Message::SelectTrack(index))
        .on_double_click(Message::PlayTrack(index))
        .into()
}

fn playlists_view(app: &App) -> Element<'_, Message> {
    let _ = app;
    container(
        column![
            text("Playlists").size(18).color(style::rgb(style::TEXT)),
            text("Playlist management is not wired to MPD stored playlists yet.")
                .size(13)
                .color(style::rgb(style::TEXT_DIM)),
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(style::content)
    .into()
}

// ---- Now Playing full view ----------------------------------------------
fn now_playing_full(app: &App) -> Element<'_, Message> {
    let current = app.current.or(app.selected).and_then(|i| app.tracks.get(i));
    let art = big_art(app, current, 280.0);
    let info: Element<Message> = if let Some(t) = current {
        column![
            text(t.title.clone()).size(22).color(style::rgb(style::TEXT)),
            text(t.artist.clone()).size(15).color(style::rgb(0xD0D0D0)),
            text(format!("{}  ({})", t.album, t.year)).size(13).color(style::rgb(style::TEXT_DIM)),
            Space::new().height(Length::Fixed(16.0)),
            text("Lyrics").size(13).color(style::rgb(style::TEXT)),
            text("No lyrics available.").size(12).color(style::rgb(style::TEXT_DIM)),
        ]
        .spacing(4)
        .into()
    } else {
        text("No track is playing.").size(15).color(style::rgb(style::TEXT_DIM)).into()
    };

    container(row![art, info].spacing(24).padding(24))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::content)
        .into()
}

// ---- Queue view ---------------------------------------------------------
fn queue_view(app: &App) -> Element<'_, Message> {
    let header = container(
        row![
            text("Queue").size(12).color(style::rgb(style::TEXT)),
            text(format!("  {} tracks", app.queue.len())).size(11).color(style::rgb(style::TEXT_DIM)),
            Space::new().width(Length::Fill),
            button(text("\u{2716} Clear").size(11))
                .padding(Padding::from([3, 8]))
                .on_press(Message::ClearQueue)
                .style(|_t, s| style::toolbar_btn(s, false)),
        ]
        .align_y(Alignment::Center)
        .padding(Padding::from([4, 10])),
    )
    .width(Length::Fill)
    .style(style::grid_header);

    let mut rows = Column::new().spacing(0);
    if app.queue.is_empty() {
        rows = rows.push(
            container(
                text("Queue is empty. Double-click a track in any view to start playback.")
                    .size(12)
                    .color(style::rgb(style::TEXT_DIM)),
            )
            .padding(20),
        );
    } else {
        for (i, t) in app.queue.iter().enumerate() {
            let kind = if i as i32 == app.queue_idx { 3 } else { (i % 2) as u8 };
            let playing = i as i32 == app.queue_idx;
            let num = if playing { format!("\u{25B6} {}", i + 1) } else { (i + 1).to_string() };
            let cells = Row::with_children(vec![
                cell(num, Length::Fixed(54.0), true, true),
                cell(t.title.clone(), W_TITLE, false, !playing),
                cell(t.artist.clone(), W_ARTIST, false, true),
                cell(t.album.clone(), W_ALBUM, false, true),
                cell(t.duration.clone(), Length::Fixed(60.0), true, true),
                remove_cell(i),
            ]);
            let rc = container(cells).width(Length::Fill).style(style::row(kind));
            rows = rows.push(
                mouse_area(rc)
                    .on_press(Message::PlayQueueIndex(i))
                    .on_double_click(Message::PlayQueueIndex(i)),
            );
        }
    }

    let body = column![
        header,
        scrollable(rows).width(Length::Fill).height(Length::Fill).style(style::scroller),
    ];
    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::content)
        .into()
}

fn remove_cell(index: usize) -> Element<'static, Message> {
    container(
        button(text("\u{2715}").size(11).color(style::rgb(0xE08A7A)))
            .padding(0)
            .on_press(Message::RemoveFromQueue(index))
            .style(style::bare_btn),
    )
    .width(Length::Fixed(40.0))
    .center_x(Length::Fill)
    .padding(Padding::from([4, 7]))
    .into()
}

// ---- Player bar ---------------------------------------------------------
fn playerbar(app: &App) -> Element<'_, Message> {
    let status = app.status.as_ref();
    let playing = status.is_some_and(|s| s.state == "play");
    let current = app.current.or(app.selected).and_then(|i| app.tracks.get(i));

    let elapsed = app
        .seek_preview
        .or_else(|| status.map(|s| s.elapsed))
        .unwrap_or(0);
    let duration = status.map(|s| s.duration).filter(|d| *d > 0).unwrap_or(0);

    let now_playing = row![
        small_art(app, current, 54.0),
        column![
            text(current.map(|t| t.title.clone()).unwrap_or_else(|| "No track selected".into()))
                .size(12)
                .color(style::rgb(style::TEXT))
                .wrapping(Wrapping::None),
            text(current.map(|t| t.artist.clone()).unwrap_or_default())
                .size(11)
                .color(style::rgb(style::TEXT_DIM))
                .wrapping(Wrapping::None),
        ]
        .spacing(2),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .width(Length::Fixed(230.0));

    let buttons = row![
        transport("\u{25C0}\u{25C0}", false, Message::Previous),
        transport(if playing { "\u{25AE}\u{25AE}" } else { "\u{25B6}" }, true, Message::PlayPause),
        transport("\u{25A0}", false, Message::Stop),
        transport("\u{25B6}\u{25B6}", false, Message::Next),
    ]
    .spacing(3);

    let seek = slider(0..=duration.max(1), elapsed.min(duration.max(1)), Message::SeekChanged)
        .on_release(Message::SeekCommit)
        .width(Length::Fill)
        .style(style::slider_style);

    let seek_row = row![
        text(fmt_secs(elapsed)).size(11).color(style::rgb(style::TEXT)).width(Length::Fixed(40.0)).align_x(iced::alignment::Horizontal::Center),
        seek,
        text(fmt_secs(duration)).size(11).color(style::rgb(style::TEXT)).width(Length::Fixed(40.0)).align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let center = column![buttons, seek_row]
        .spacing(5)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .padding(Padding::from([0, 12]));

    let right = row![
        toggle("SHUF", app.shuffle, Message::Shuffle),
        toggle("REP", app.repeat, Message::Repeat),
        text("Vol").size(11).color(style::rgb(style::TEXT_DIM)),
        slider(0..=100, app.volume, Message::VolumeChanged)
            .width(Length::Fixed(90.0))
            .style(style::slider_style),
        text(app.volume.to_string()).size(11).color(style::rgb(style::TEXT)).width(Length::Fixed(26.0)).align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    container(
        row![now_playing, center, right]
            .spacing(12)
            .align_y(Alignment::Center)
            .padding(Padding::from([0, 12])),
    )
    .width(Length::Fill)
    .height(Length::Fixed(70.0))
    .style(style::playerbar)
    .into()
}

fn transport(glyph: &str, primary: bool, msg: Message) -> Element<'_, Message> {
    button(
        container(text(glyph.to_string()).size(12))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(if primary { 40.0 } else { 34.0 }))
    .height(Length::Fixed(30.0))
    .on_press(msg)
    .style(move |_t, s| style::transport_btn(s, primary))
    .into()
}

fn toggle(label: &str, active: bool, msg: Message) -> Element<'_, Message> {
    button(text(label.to_string()).size(10))
        .padding(Padding::from([5, 7]))
        .on_press(msg)
        .style(move |_t, s| style::toggle_btn(s, active))
        .into()
}

// ---- Album art elements -------------------------------------------------
fn small_art<'a>(app: &'a App, track: Option<&'a Track>, size: f32) -> Element<'a, Message> {
    art_element(app, track, size, 18.0)
}

fn big_art<'a>(app: &'a App, track: Option<&'a Track>, size: f32) -> Element<'a, Message> {
    art_element(app, track, size, 40.0)
}

fn art_element<'a>(app: &'a App, track: Option<&'a Track>, size: f32, font: f32) -> Element<'a, Message> {
    if let Some(t) = track {
        if let Some(Some(handle)) = app.album_art.get(&t.path) {
            return container(image(handle.clone()).width(Length::Fixed(size)).height(Length::Fixed(size)))
                .style(style::art_tile(style::rgb(style::BORDER_DK)))
                .into();
        }
        let (color, initial) = tile_for(&album_key(t), &t.album);
        return container(text(initial).size(font).color(Color::from_rgba(1.0, 1.0, 1.0, 0.92)))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(style::art_tile(color))
            .into();
    }
    container(text("\u{266A}").size(font).color(style::rgb(style::TEXT_DIM)))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(style::art_tile(style::rgb(0x202020)))
        .into()
}

// ===================================================================
//  App helpers
// ===================================================================
impl App {
    fn rebuild_collections(&mut self) {
        // Per-track deterministic metadata (rating / play count) mirrors the
        // original UI's path-hash derivation so the grid looks populated.
        self.meta = self
            .tracks
            .iter()
            .map(|t| {
                let h = path_hash(&t.path);
                TrackMeta {
                    rating: (h % 6) as u8,
                    plays: (h >> 3) % 43,
                }
            })
            .collect();

        // Albums grouped by (album, album_artist), preserving sort order.
        let mut albums: Vec<Album> = Vec::new();
        let mut index_of: HashMap<String, usize> = HashMap::new();
        for (i, t) in self.tracks.iter().enumerate() {
            let key = album_key(t);
            let pos = *index_of.entry(key.clone()).or_insert_with(|| {
                albums.push(Album {
                    key: key.clone(),
                    album: t.album.clone(),
                    album_artist: t.album_artist.clone(),
                    year: t.year,
                    genre: t.genre.clone(),
                    tracks: Vec::new(),
                });
                albums.len() - 1
            });
            albums[pos].tracks.push(i);
        }

        // Artists from album list.
        let mut artist_albums: HashMap<String, usize> = HashMap::new();
        let mut artist_order: Vec<String> = Vec::new();
        for a in &albums {
            let entry = artist_albums.entry(a.album_artist.clone()).or_insert_with(|| {
                artist_order.push(a.album_artist.clone());
                0
            });
            *entry += 1;
        }
        artist_order.sort();
        self.artists = artist_order
            .iter()
            .map(|name| ArtistRow {
                name: name.clone(),
                albums: artist_albums.get(name).copied().unwrap_or(0),
            })
            .collect();

        // Genres with counts.
        let mut genre_counts: HashMap<String, usize> = HashMap::new();
        for t in &self.tracks {
            *genre_counts.entry(t.genre.clone()).or_insert(0) += 1;
        }
        let mut genres: Vec<(String, usize)> = genre_counts.into_iter().collect();
        genres.sort_by(|a, b| a.0.cmp(&b.0));
        self.genres = genres;

        // Folders (distinct parent directories).
        let mut folder_set: HashSet<String> = HashSet::new();
        for t in &self.tracks {
            if let Some(pos) = t.path.rfind('/') {
                folder_set.insert(t.path[..pos].to_string());
            }
        }
        let mut folders: Vec<String> = folder_set.into_iter().collect();
        folders.sort();
        self.folders = folders;

        self.albums = albums;
    }

    fn sync_current_from_status(&mut self) -> Task<Message> {
        let path = self
            .status
            .as_ref()
            .map(|s| s.file.clone())
            .filter(|f| !f.is_empty());
        if let Some(path) = path {
            self.current = self.tracks.iter().position(|t| t.path == path);
            if let Some(index) = self.current {
                return self.ensure_art_for_index(index);
            }
        }
        Task::none()
    }

    fn ensure_art_for_index(&mut self, index: usize) -> Task<Message> {
        if let Some(track) = self.tracks.get(index) {
            let path = track.path.clone();
            if !self.album_art.contains_key(&path) && self.requested_art.insert(path.clone()) {
                // Prioritise the current/selected track by pushing to the front.
                self.art_pending.push_front(ArtJob::Album {
                    path,
                    album_artist: track.album_artist.clone(),
                    album: track.album.clone(),
                });
            }
        }
        self.pump_art()
    }

    // Queue an album's cover for fetching (deduped by its first track's path).
    fn queue_album_cover(&mut self, album_index: usize) {
        let Some(album) = self.albums.get(album_index) else {
            return;
        };
        let Some(&first) = album.tracks.first() else {
            return;
        };
        let path = self.tracks[first].path.clone();
        if self.album_art.contains_key(&path) || !self.requested_art.insert(path.clone()) {
            return;
        }
        self.art_pending.push_back(ArtJob::Album {
            path,
            album_artist: album.album_artist.clone(),
            album: album.album.clone(),
        });
    }

    // Queue an artist image for fetching (deduped by name).
    fn queue_artist_image(&mut self, name: String) {
        let key = format!("@artist@{name}");
        if self.artist_art.contains_key(&name) || !self.requested_art.insert(key) {
            return;
        }
        self.art_pending.push_back(ArtJob::Artist { name });
    }

    // Start fetches up to the concurrency cap. Returns a batch of art-fetch
    // tasks; each completion calls back into `pump_art` to keep the pipe full.
    fn pump_art(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.art_inflight < MAX_ART_INFLIGHT {
            let Some(job) = self.art_pending.pop_front() else {
                break;
            };
            self.art_inflight += 1;
            tasks.push(match job {
                ArtJob::Album { path, album_artist, album } => Task::perform(
                    async move {
                        let result = mpd::get_cover(path.clone(), album_artist, album);
                        (path, result)
                    },
                    |(path, result)| Message::AlbumArtLoaded(path, result),
                ),
                ArtJob::Artist { name } => Task::perform(
                    async move {
                        let result = mpd::get_artist_image(name.clone());
                        (name, result)
                    },
                    |(name, result)| Message::ArtistArtLoaded(name, result),
                ),
            });
        }
        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    // The list of track indices to render for the active view, after search
    // and (for Music) quick filters and (for selector views) the selection.
    fn visible_indices(&self) -> Vec<usize> {
        let search = self.search.trim().to_lowercase();
        let qt = self.quick_title.trim().to_lowercase();
        let qa = self.quick_artist.trim().to_lowercase();
        let ql = self.quick_album.trim().to_lowercase();

        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                let scope = match self.section.0 {
                    Section::Folders => self
                        .sel_folder
                        .as_deref()
                        .map(|f| f == "*" || t.path.starts_with(f))
                        .unwrap_or(true),
                    _ => true,
                };
                if !scope {
                    return false;
                }
                if !search.is_empty() {
                    let hay = format!("{} {} {} {}", t.title, t.artist, t.album, t.genre).to_lowercase();
                    if !hay.contains(&search) {
                        return false;
                    }
                }
                if !qt.is_empty() && !t.title.to_lowercase().contains(&qt) {
                    return false;
                }
                if !qa.is_empty() && !t.artist.to_lowercase().contains(&qa) {
                    return false;
                }
                if !ql.is_empty() && !t.album.to_lowercase().contains(&ql) {
                    return false;
                }
                true
            })
            .map(|(i, _)| i)
            .collect()
    }

    // Tracks to enqueue when the user plays `index`: the album it belongs to
    // if it's part of one, else the currently visible list.
    fn play_context_indices(&self, index: usize) -> Vec<usize> {
        let track = &self.tracks[index];
        let key = album_key(track);
        if let Some(album) = self.albums.iter().find(|a| a.key == key) {
            if album.tracks.len() > 1 {
                return album.tracks.clone();
            }
        }
        let visible = self.visible_indices();
        if visible.contains(&index) {
            visible
        } else {
            vec![index]
        }
    }
}

// ===================================================================
//  Pure helpers
// ===================================================================
fn album_key(t: &Track) -> String {
    format!("{}\u{0001}{}", t.album, t.album_artist)
}

fn path_hash(path: &str) -> u32 {
    let mut h: u32 = 0;
    for b in path.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    h
}

fn tile_for(key: &str, label: &str) -> (Color, String) {
    let h = path_hash(key);
    let hue = (h % 360) as f32;
    let sat = 0.45 + ((h >> 4) % 20) as f32 / 100.0;
    let color = hsl_to_rgb(hue, sat, 0.30);
    let initial = label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    (color, initial)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::from_rgb(r + m, g + m, b + m)
}

fn stars(rating: u8) -> String {
    let r = rating.min(5) as usize;
    let mut s = String::new();
    for _ in 0..r {
        s.push('\u{2605}');
    }
    for _ in r..5 {
        s.push('\u{2606}');
    }
    s
}

fn fmt_secs(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
