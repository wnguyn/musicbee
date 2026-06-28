// MusicBee (Tauri PoC) frontend logic — accuracy pass.
// Adds: album group headers in grids, context-toolbar layout toggle,
// tabbed Now Playing panel (Now Playing / Lyrics / Info), MusicBee player bar.

(function () {
  "use strict";

  // ---- State --------------------------------------------------------------
  const state = {
    tracks: [], albums: [], artists: [], genres: [], folderTree: null,
    view: "music",
    selected: new Set(),
    playingIdx: -1, isPlaying: false,
    sort: { col: "track_number", dir: "asc" },
    quick: { title: "", artist: "", album: "" },
    search: "",
    layout: "grouped",        // details | grouped | artwork
    npTab: "playing",
    selArtist: null, selAlbum: null, selGenre: null, selFolder: null,
    lastAnchor: null,
    currentTime: 0, totalTime: 0,
    volume: 70, shuffle: false, repeat: false, eq: false,
    meta: null,             // path -> { rating, play_count, skip_count, date_added }
    mpdConnected: false,
    mpdStatusTimer: null,
    albumArt: new Map(),    // albumKey -> dataUrl (base64)
    queue: [],              // array of track objects currently queued for playback
    queueIdx: -1,           // index in `queue` of the currently playing track (-1 = none)
    queueSource: "",        // human label for the source (e.g. "Album: X", "All Tracks")
    queueDirty: false,      // true if queue UI needs to re-render
    playlistVersion: -1,    // last seen MPD queue version; queue is refetched on change
    // View cache: when a view is rendered, the innerHTML of its main grid
    // is stashed here so flipping back is instant. Keyed by view name +
    // a small signature derived from sort/search/_activeTracks length.
    _viewCache: new Map(),
    _pendingRenders: new Map(), // viewName -> rAF id
  };

  // Columns mirror MusicBee's default track grid. Rating & Play Count are
  // derived per-track (deterministic from path) so the IPC contract stays
  // unchanged; rating is mutable via the star widget.
  const COLUMNS = [
    { key: "track_number", label: "#",      cls: "num" },
    { key: "title",        label: "Title" },
    { key: "artist",       label: "Artist" },
    { key: "album",        label: "Album" },
    { key: "album_artist", label: "Album Artist" },
    { key: "genre",        label: "Genre" },
    { key: "year",         label: "Year",   cls: "num" },
    { key: "rating",       label: "Rating", cls: "rating" },
    { key: "play_count",   label: "Plays",  cls: "num" },
    { key: "duration",     label: "Time",   cls: "num" },
  ];

  // ---- Tauri IPC ----------------------------------------------------------
  async function loadLibrary() {
    const t = window.__TAURI__;
    const invoke = t && ((t.core && t.core.invoke) || t.invoke);
    if (!invoke) {
      throw new Error("Tauri IPC unavailable — is this running inside Tauri?");
    }
    const lib = await invoke("get_library");
    return Array.isArray(lib) ? lib : [];
  }
  const _albumArtInFlight = new Map(); // albumKey -> Promise

  // Bound the number of concurrent album-art requests. Each request opens a
  // fresh MPD TCP connection on the Rust side and can transfer hundreds of KB
  // of image data; firing one per album card at once (a large library has
  // hundreds of albums) floods MPD and freezes the UI. A small pool keeps the
  // app responsive while art trickles in.
  const ART_MAX_CONCURRENT = 4;
  let _artActive = 0;
  const _artWaiters = [];
  function _artAcquire() {
    if (_artActive < ART_MAX_CONCURRENT) { _artActive++; return Promise.resolve(); }
    return new Promise((resolve) => _artWaiters.push(resolve));
  }
  function _artRelease() {
    const next = _artWaiters.shift();
    if (next) next(); else _artActive--;
  }

  async function loadAlbumArt(albumKey, path) {
    // state.albumArt stores: a dataUrl (art found), null (looked up, none
    // available), or has no entry (never looked up). Caching the null result
    // is what stops the 3s status poll from re-requesting missing art (and
    // re-opening an MPD connection) over and over.
    if (state.albumArt.has(albumKey)) return state.albumArt.get(albumKey);
    if (_albumArtInFlight.has(albumKey)) return _albumArtInFlight.get(albumKey);
    const p = (async () => {
      await _artAcquire();
      try {
        const res = await tauriInvoke("get_album_art", { path });
        if (res && res.data) {
          const dataUrl = "data:" + res.mime + ";base64," + res.data;
          state.albumArt.set(albumKey, dataUrl);
          return dataUrl;
        }
        state.albumArt.set(albumKey, null);
      } catch (e) {
        // MPD has no art for this track — remember that so we don't keep asking.
        state.albumArt.set(albumKey, null);
      } finally {
        _artRelease();
        _albumArtInFlight.delete(albumKey);
      }
      return null;
    })();
    _albumArtInFlight.set(albumKey, p);
    return p;
  }
  function tauriInvoke(command, args) {
    const t = window.__TAURI__;
    const invoke = t && ((t.core && t.core.invoke) || t.invoke);
    return invoke ? invoke(command, args || {}) : Promise.reject(new Error("Tauri IPC unavailable"));
  }

  async function tryMpd(command, args) {
    try { await tauriInvoke(command, args); await syncMpdStatus(); return true; }
    catch (e) { console.warn(command + " failed:", e); setMpdStatus(false, String(e)); return false; }
  }

  // ---- Helpers ------------------------------------------------------------
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  function el(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; }
  // Trailing-edge debounce. Cancels the previous timer so rapid input fires
  // the callback at most once per quiet window.
  function debounce(fn, ms) {
    let h = 0;
    return function (...args) {
      if (h) clearTimeout(h);
      h = setTimeout(() => fn.apply(this, args), ms);
    };
  }
  function fmtTime(sec) { sec = Math.max(0, Math.floor(sec)); const m = Math.floor(sec / 60), s = sec % 60; return m + ":" + (s < 10 ? "0" + s : s); }
  function durToSec(d) { if (!d) return 0; const p = String(d).split(":").map(Number); if (p.some(isNaN)) return 0; let t = 0; for (const n of p) t = t * 60 + n; return t; }
  function totalDur(tracks) { return tracks.reduce((a, t) => a + durToSec(t.duration), 0); }
  function albumKey(t) { return (t.album || "?") + "\u0001" + (t.album_artist || "?"); }
  // Derive a display title from a file path (mirrors the Rust fallback).
  function titleFromPath(path) {
    const base = String(path || "").split("/").pop() || String(path || "");
    const dot = base.lastIndexOf(".");
    return dot > 0 ? base.slice(0, dot) : base;
  }

  function artStyle(key) {
    let h = 0; for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
    const hue1 = h % 360;
    const hue2 = (hue1 + 40 + (h >> 8) % 80) % 360;
    const sat = 55 + (h >> 4) % 25;
    const lit1 = 38 + (h >> 12) % 14;
    const lit2 = 22 + (h >> 16) % 12;
    const ang = (h >> 2) % 180;
    return { bg: `linear-gradient(${ang}deg, hsl(${hue1},${sat}%,${lit1}%), hsl(${hue2},${sat}%,${lit2}%))`,
             initial: key.charAt(0).toUpperCase() };
  }

  // ---- Album art rendering ------------------------------------------------
  // Swap a placeholder art tile for the real cover image, preserving any
  // overlay labels (e.g. the album-grid genre tag). Idempotent per element.
  function setArtImage(artEl, dataUrl) {
    if (!dataUrl || !artEl || artEl._hasArt) return;
    artEl._hasArt = true;
    artEl.style.background = "none";
    const initial = artEl.querySelector(".aa-initial");
    if (initial) initial.remove();
    for (const n of Array.from(artEl.childNodes)) {
      if (n.nodeType === 3) artEl.removeChild(n); // bare-text letter placeholder
    }
    const img = el("img");
    img.alt = "";
    img.loading = "lazy";
    img.decoding = "async";
    img.style.cssText = "width:100%;height:100%;object-fit:cover;display:block";
    img.src = dataUrl;
    artEl.insertBefore(img, artEl.firstChild);
  }

  // Lazily load art for grid tiles only when they scroll into view. A single
  // persistent observer keeps hundreds of off-screen cards from each firing an
  // MPD request the moment a view renders.
  const _artObserver = ("IntersectionObserver" in window)
    ? new IntersectionObserver((entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const node = e.target;
          _artObserver.unobserve(node);
          const al = node._album;
          const first = al && al.tracks && al.tracks[0];
          if (!first) continue;
          loadAlbumArt(al.key, first.path).then((d) => { if (d) setArtImage(node, d); });
        }
      }, { rootMargin: "300px" })
    : null;

  function observeArt(node, al) {
    if (!al) return;
    node._album = al;
    if (_artObserver) {
      _artObserver.observe(node);
    } else {
      const first = al.tracks && al.tracks[0];
      if (first) loadAlbumArt(al.key, first.path).then((d) => { if (d) setArtImage(node, d); });
    }
  }

  // Paint album art into a persistent single-slot element (player bar, Now
  // Playing panels). Change-detection via `_artSig` lets the status poll call
  // this every few seconds without rebuilding the DOM or reloading the image
  // unless the track (or its now-available artwork) actually changed.
  function paintArt(elem, t, phText) {
    if (!elem) return;
    const ak = t ? albumKey(t) : null;
    const cached = ak ? state.albumArt.get(ak) : undefined; // dataUrl | null | undefined
    const sig = !t ? "\u0000none" : (cached ? "img\u0000" + ak : "ph\u0000" + ak);
    if (elem._artSig === sig) return;
    elem._artSig = sig;
    elem._hasArt = false;
    elem.innerHTML = "";
    if (!t) {
      elem.style.background = "#202020";
      if (phText) elem.appendChild(el("div", "np-art-ph", phText));
      return;
    }
    if (cached) {
      setArtImage(elem, cached);
      return;
    }
    const st = artStyle(ak);
    elem.style.background = st.bg;
    elem.appendChild(el("div", "aa-initial", st.initial));
    if (cached === undefined) {
      loadAlbumArt(ak, t.path).then((dataUrl) => {
        // Only apply if this slot still wants art for the same album.
        if (dataUrl && elem._artSig === "ph\u0000" + ak) {
          setArtImage(elem, dataUrl);
          elem._artSig = "img\u0000" + ak;
        }
      });
    }
  }
  function pathHash(path) { let h = 0; for (let i = 0; i < path.length; i++) h = (h * 31 + path.charCodeAt(i)) >>> 0; return h; }
  function deriveMeta() {
    state.meta = new Map();
    const now = Date.now();
    for (const t of state.tracks) {
      const h = pathHash(t.path);
      const rating = (h % 6);                       // 0..5 stars, deterministic
      const playCount = (h >> 3) % 43;              // 0..42 plays
      const skipCount = (h >> 9) % 11;              // 0..10 skips
      // date added: a stable number of days before "now"
      const daysAgo = 1 + (h >> 12) % 720;
      const added = new Date(now - daysAgo * 86400000);
      const dateAdded = added.toISOString().slice(0, 10);   // YYYY-MM-DD
      state.meta.set(t.path, { rating, play_count: playCount, skip_count: skipCount, date_added: dateAdded });
    }
  }
  function metaOf(t) { return (state.meta && state.meta.get(t.path)) || { rating: 0, play_count: 0, skip_count: 0, date_added: "" }; }
  function setRating(t, r) { const m = metaOf(t); m.rating = r; state.meta.set(t.path, m); }
  function avgRatingStars(tracks) {
    // produce a string of filled stars for the album aggregate
    if (!tracks.length) return "";
    let sum = 0; for (const t of tracks) sum += metaOf(t).rating;
    const avg = Math.round(sum / tracks.length);
    return "\u2605".repeat(avg) + "\u2606".repeat(5 - avg);
  }
  // build the 5-star widget into `container`, reflecting `rating`; clicking
  // star i calls onSet(i)
  function ratingWidget(container, rating, onSet) {
    container.innerHTML = "";
    for (let i = 1; i <= 5; i++) {
      const s = el("span", "star" + (i <= rating ? " on" : ""), i <= rating ? "\u2605" : "\u2606");
      s.dataset.n = i;
      s.onclick = (e) => { e.stopPropagation(); onSet(i); };
      container.appendChild(s);
    }
  }
  function refreshRatingFor(t) {
    // re-render the star widget in every rating cell bound to this track
    $$(".row.data").forEach((row) => { if (row._track && row._track.path === t.path) {
      const w = row.querySelector(".rating.sm"); const cell = row.querySelector(".cell-rating");
      if (cell && w) { ratingWidget(w, metaOf(t).rating, (nv) => { setRating(t, nv); refreshRatingFor(t); updateNowPlaying(); }); }
    } });
  }

  // ---- Derive collections -------------------------------------------------
  function deriveCollections() {
    const amap = new Map();
    for (const t of state.tracks) {
      const k = albumKey(t);
      if (!amap.has(k)) amap.set(k, { key: k, album: t.album, album_artist: t.album_artist, genre: t.genre, year: t.year, tracks: [] });
      amap.get(k).tracks.push(t);
    }
    state.albums = Array.from(amap.values()).sort((a, b) => a.album_artist.localeCompare(b.album_artist) || a.album.localeCompare(b.album));
    for (const al of state.albums) al.tracks.sort((a, b) => a.track_number - b.track_number);

    const armap = new Map();
    for (const al of state.albums) { const a = al.album_artist; if (!armap.has(a)) armap.set(a, []); armap.get(a).push(al); }
    state.artists = Array.from(armap.keys()).sort().map(name => ({ name, albums: armap.get(name) }));

    const gmap = new Map();
    for (const t of state.tracks) gmap.set(t.genre, (gmap.get(t.genre) || 0) + 1);
    state.genres = Array.from(gmap.keys()).sort().map(name => ({ name, count: gmap.get(name) }));

    state.folderTree = buildFolderTree(state.tracks);
  }

  function buildFolderTree(tracks) {
    const root = { name: "Music", path: "/home/user/Music", children: {}, tracks: [] };
    for (const t of tracks) {
      const dir = String(t.path).replace(/\/[^/]+$/, "");
      const parts = dir.split("/").filter(Boolean);
      let node = root, acc = "";
      for (const p of parts) { acc = acc ? acc + "/" + p : "/" + p; if (!node.children[p]) node.children[p] = { name: p, path: acc, children: {}, tracks: [] }; node = node.children[p]; }
      node.tracks.push(t);
    }
    return root;
  }
  function tracksInFolder(node) { let out = node.tracks.slice(); for (const c of Object.values(node.children)) out = out.concat(tracksInFolder(c)); return out; }
  function findFolderNode(node, path) { if (node.path === path) return node; for (const c of Object.values(node.children)) { const r = findFolderNode(c, path); if (r) return r; } return null; }

  // ---- Filtering ----------------------------------------------------------
  function baseFiltered() {
    const s = state.search.trim().toLowerCase();
    if (!s) return state.tracks.slice();
    return state.tracks.filter((t) => (t.title + " " + t.artist + " " + t.album + " " + (t.genre || "")).toLowerCase().includes(s));
  }
  function sortTracks(tracks) {
    const { col, dir } = state.sort;
    return tracks.slice().sort((a, b) => {
      let av = a[col], bv = b[col];
      if (typeof av === "number" && typeof bv === "number") return dir === "asc" ? av - bv : bv - av;
      av = String(av || ""); bv = String(bv || "");
      return dir === "asc" ? av.localeCompare(bv) : bv.localeCompare(av);
    });
  }

  // ====================================================================
  //  SHARED GRID (with optional album grouping)
  // ====================================================================
  function renderGridHeader(headerEl) {
    headerEl.innerHTML = "";
    COLUMNS.forEach((c) => {
      const sorted = state.sort.col === c.key && state.sort.col !== "rating";
      const cell = el("div", "cell " + (c.cls || "") + (sorted ? " sorted" : ""), c.label);
      cell.dataset.col = c.key;
      if (sorted) cell.appendChild(el("span", "sort-ind", state.sort.dir === "asc" ? "\u25B2" : "\u25BC"));
      headerEl.appendChild(cell);
    });
  }

  // tracks: already-sorted array. grouped: insert album group headers.
  function renderGridRows(rowsEl, tracks, opts) {
    opts = opts || {};
    rowsEl.innerHTML = "";
    const frag = document.createDocumentFragment();
    const grouped = !!opts.grouped && tracks.length > 0;

    if (grouped) {
      // partition by album preserving current sort order within each album
      const groups = new Map();
      const order = [];
      for (const t of tracks) {
        const k = albumKey(t);
        if (!groups.has(k)) { groups.set(k, []); order.push(k); }
        groups.get(k).push(t);
      }
      for (const k of order) {
        const grp = groups.get(k);
        const al = state.albums.find((a) => a.key === k) || { album: grp[0].album, album_artist: grp[0].album_artist, year: grp[0].year, genre: grp[0].genre };
        frag.appendChild(groupHeader(al, grp));
        grp.forEach((t) => frag.appendChild(dataRow(t, opts)));
      }
    } else {
      tracks.forEach((t, i) => frag.appendChild(dataRow(t, opts, i)));
    }
    rowsEl.appendChild(frag);
  }

  function groupHeader(al, tracks) {
    const gh = el("div", "group-header");
    gh._albumKey = al.key || albumKey(tracks[0]);
    const st = artStyle(gh._albumKey);
    const art = el("div", "gh-art", st.initial); art.style.background = st.bg;
    gh.appendChild(art);
    const cachedArt = state.albumArt.get(gh._albumKey);
    if (cachedArt) setArtImage(art, cachedArt);
    else if (cachedArt === undefined) observeArt(art, { key: gh._albumKey, tracks });
    const txt = el("div", "gh-text");
    const titleRow = el("div", "gh-album", al.album || tracks[0].album);
    if (al.year || tracks[0].year) { const y = el("span", "gh-year", "(" + (al.year || tracks[0].year) + ")"); titleRow.appendChild(y); }
    txt.appendChild(titleRow);
    txt.appendChild(el("div", "gh-artist", al.album_artist || tracks[0].album_artist));
    gh.appendChild(txt);
    gh.appendChild(el("div", "gh-meta", tracks.length + " track" + (tracks.length === 1 ? "" : "s") + "  \u00b7  " + fmtTime(totalDur(tracks))));
    gh.ondblclick = () => { const idx = state.tracks.indexOf(tracks[0]); if (idx >= 0) playTrack(idx); };
    return gh;
  }

  function dataRow(t, opts, flatIdx) {
    const row = el("div", "row data");
    row._track = t;
    COLUMNS.forEach((c) => {
      if (c.key === "rating") {
        const cell = el("div", "cell cell-rating");
        const w = el("div", "rating sm");
        const r = metaOf(t).rating;
        ratingWidget(w, r, (nv) => { setRating(t, nv); refreshRatingFor(t); updateNowPlaying(); });
        cell.appendChild(w);
        row.appendChild(cell);
        return;
      }
      if (c.key === "play_count") { row.appendChild(el("div", "cell num", String(metaOf(t).play_count))); return; }
      let v = t[c.key];
      if (c.key === "track_number" && c.label === "#") v = opts.grouped ? t.track_number : (flatIdx + 1);
      row.appendChild(el("div", "cell " + (c.cls || ""), v == null ? "" : String(v)));
    });
    if (state.selected.has(t.path)) row.classList.add("selected");
    if (state.playingIdx >= 0 && state.tracks[state.playingIdx] && state.tracks[state.playingIdx].path === t.path) row.classList.add("playing-row");
    return row;
  }

  function bindGridClicks(rowsEl, onDbl) {
    rowsEl.onclick = (e) => {
      const row = e.target.closest(".row.data"); if (!row) return;
      selectPaths(row._track.path, e.ctrlKey || e.metaKey, e.shiftKey);
      refreshAllGridSelections();
    };
    rowsEl.ondblclick = (e) => { const row = e.target.closest(".row.data"); if (row && onDbl) onDbl(row._track); };
  }
  function selectPaths(path, additive, range) {
    if (range && state.lastAnchor) {
      const base = state._activeTracks || state.tracks;
      const paths = base.map((t) => t.path);
      const a = paths.indexOf(state.lastAnchor), b = paths.indexOf(path);
      if (a >= 0 && b >= 0) { const [lo, hi] = a < b ? [a, b] : [b, a]; state.selected = new Set(paths.slice(lo, hi + 1)); }
    } else if (additive) {
      if (state.selected.has(path)) state.selected.delete(path); else state.selected.add(path);
    } else { state.selected = new Set([path]); }
    state.lastAnchor = path; updateStatus();
  }
  function refreshAllGridSelections() {
    $$(".grid-rows").forEach((rows) => $$(".row.data", rows).forEach((r) => { if (r._track) r.classList.toggle("selected", state.selected.has(r._track.path)); }));
  }

  // ====================================================================
  //  VIEWS
  // ====================================================================
  function musicTracks() {
    let rows = baseFiltered();
    const q = state.quick;
    if (q.title)  rows = rows.filter((t) => String(t.title).toLowerCase().includes(q.title.toLowerCase()));
    if (q.artist) rows = rows.filter((t) => String(t.artist).toLowerCase().includes(q.artist.toLowerCase()));
    if (q.album)  rows = rows.filter((t) => String(t.album).toLowerCase().includes(q.album.toLowerCase()));
    return sortTracks(rows);
  }

  function renderMusicView() {
    const rows = musicTracks();
    state._activeTracks = rows;
    renderGridHeader($("#grid-header-music"));
    renderGridRows($("#grid-rows-music"), rows, { grouped: state.layout === "grouped" });
    updateStatus();
  }

  function buildTree() {
    const tree = $("#tree"); tree.innerHTML = "";
    const nodes = [
      { label: "Music", icon: "\u266B", open: true, children: [
        { label: "All Tracks", node: "music-all" }, { label: "Albums", node: "music-albums" },
        { label: "Artists", node: "music-artists" }, { label: "Genres", node: "music-genres" } ] },
      { label: "Folders", icon: "\u{1F4C1}", open: false, children: [] },
      { label: "Playlists", icon: "\u2630", open: false, children: [] },
      { label: "Auto-DJ", icon: "\u2605", open: false, children: [] },
      { label: "Radio", icon: "\u25CF", open: false, children: [] },
      { label: "Inbox", icon: "\u2709", open: false, children: [] },
    ];
    nodes.forEach((n) => {
      const nd = el("div", "node" + (n.open ? "" : " collapsed"));
      nd.dataset.label = n.label;
      nd.innerHTML = `<span class="expander">${n.open ? "\u25BC" : "\u25B6"}</span><span class="icon">${n.icon || ""}</span><span class="label">${n.label}</span>`;
      tree.appendChild(nd);
      (n.children || []).forEach((c) => { const ch = el("div", "node level-1"); ch.dataset.node = c.node; ch.innerHTML = `<span class="label">${c.label}</span>`; tree.appendChild(ch); });
    });
    $$("#tree .node").forEach((n) => n.onclick = (e) => {
      const exp = n.querySelector(".expander"); if (exp) exp.textContent = exp.textContent === "\u25BC" ? "\u25B6" : "\u25BC";
      $$("#tree .node").forEach((x) => x.classList.remove("selected")); n.classList.add("selected");
      const node = n.dataset.node;
      if (node === "music-all") { state.quick = { title: "", artist: "", album: "" }; $$("#quickfilter .qf-input").forEach((i) => i.value = ""); renderMusicView(); }
      if (node === "music-albums") switchView("albums");
      if (node === "music-artists") switchView("artists");
      if (node === "music-genres") switchView("genres");
      e.stopPropagation();
    });
  }

  function renderAlbumsArtistsView() {
    const list = $("#aa-artists"); list.innerHTML = "";
    const all = el("div", "lb-item", "All Artists"); all.dataset.artist = "*"; list.appendChild(all);
    state.artists.forEach((a) => { const it = el("div", "lb-item", a.name + "  (" + a.albums.length + ")"); it.dataset.artist = a.name; list.appendChild(it); });
    bindListClick(list, "artist", () => {
      const art = state.selArtist;
      const albums = art === "*" || !art ? state.albums : state.albums.filter((al) => al.album_artist === art);
      renderAlbumGrid($("#aa-albums"), albums, "aa");
      const first = albums[0]; state.selAlbum = first ? first.key : null;
      $("#aa-tracks-header").textContent = first ? (first.album + " — " + first.album_artist) : "Tracks";
      renderAATracks();
    });
    state.selArtist = state.selArtist || "*";
    const first = $(".lb-item", list); first && first.classList.add("selected");
    const albums = state.selArtist === "*" ? state.albums : state.albums.filter((al) => al.album_artist === state.selArtist);
    renderAlbumGrid($("#aa-albums"), albums, "aa");
    const a0 = albums[0]; state.selAlbum = a0 ? a0.key : null;
    $("#aa-tracks-header").textContent = a0 ? (a0.album + " — " + a0.album_artist) : "Tracks";
    renderAATracks();
  }
  function renderAATracks() {
    const al = state.albums.find((a) => a.key === state.selAlbum);
    const tracks = al ? al.tracks : [];
    renderGridHeader($("#grid-header-aa")); renderGridRows($("#grid-rows-aa"), sortTracks(tracks), { grouped: false });
    $("#aa-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s") + "  \u00b7  " + fmtTime(totalDur(tracks));
  }

  function renderArtistsView() {
    const list = $("#art-artists"); list.innerHTML = "";
    const all = el("div", "lb-item", "All Artists"); all.dataset.artist = "*"; list.appendChild(all);
    state.artists.forEach((a) => { const it = el("div", "lb-item", a.name); it.dataset.artist = a.name; list.appendChild(it); });
    bindListClick(list, "artist", () => {
      const art = state.selArtist;
      const tracks = art === "*" || !art ? state.tracks : state.tracks.filter((t) => t.album_artist === art);
      state._activeTracks = tracks;
      $("#art-tracks-header").textContent = art === "*" ? "All Tracks" : art;
      renderGridHeader($("#grid-header-art")); renderGridRows($("#grid-rows-art"), sortTracks(tracks), { grouped: state.layout === "grouped" });
      $("#art-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
    });
    state.selArtist = state.selArtist || "*";
    const first = $(".lb-item", list); first && first.classList.add("selected");
    const tracks = state.selArtist === "*" ? state.tracks : state.tracks.filter((t) => t.album_artist === state.selArtist);
    state._activeTracks = tracks;
    $("#art-tracks-header").textContent = state.selArtist === "*" ? "All Tracks" : state.selArtist;
    renderGridHeader($("#grid-header-art")); renderGridRows($("#grid-rows-art"), sortTracks(tracks), { grouped: state.layout === "grouped" });
    $("#art-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
  }

  function renderAlbumsView() {
    $("#al-count").textContent = state.albums.length + " albums";
    renderAlbumGrid($("#al-albums"), state.albums, "al");
    const first = state.albums[0]; state.selAlbum = first ? first.key : null;
    $("#al-tracks-header").textContent = first ? (first.album + " — " + first.album_artist) : "Select an album";
    renderALTracks();
  }
  function renderALTracks() {
    const al = state.albums.find((a) => a.key === state.selAlbum);
    const tracks = al ? al.tracks : [];
    state._activeTracks = tracks;
    renderGridHeader($("#grid-header-al")); renderGridRows($("#grid-rows-al"), sortTracks(tracks), { grouped: false });
    $("#al-trk-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
  }

  function renderGenresView() {
    const list = $("#gn-genres"); list.innerHTML = "";
    const all = el("div", "lb-item", "All Genres"); all.dataset.genre = "*"; list.appendChild(all);
    state.genres.forEach((g) => { const it = el("div", "lb-item", g.name + "  (" + g.count + ")"); it.dataset.genre = g.name; list.appendChild(it); });
    bindListClick(list, "genre", () => {
      const g = state.selGenre;
      const tracks = g === "*" || !g ? state.tracks : state.tracks.filter((t) => t.genre === g);
      state._activeTracks = tracks;
      $("#gn-tracks-header").textContent = g === "*" ? "All Tracks" : g;
      renderGridHeader($("#grid-header-gn")); renderGridRows($("#grid-rows-gn"), sortTracks(tracks), { grouped: state.layout === "grouped" });
      $("#gn-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
    });
    state.selGenre = state.selGenre || "*";
    const first = $(".lb-item", list); first && first.classList.add("selected");
    const tracks = state.selGenre === "*" ? state.tracks : state.tracks.filter((t) => t.genre === state.selGenre);
    state._activeTracks = tracks;
    $("#gn-tracks-header").textContent = state.selGenre === "*" ? "All Tracks" : state.selGenre;
    renderGridHeader($("#grid-header-gn")); renderGridRows($("#grid-rows-gn"), sortTracks(tracks), { grouped: state.layout === "grouped" });
    $("#gn-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
  }

  function renderFoldersView() {
    const ft = $("#folders-tree"); ft.innerHTML = "";
    (function walk(node, depth) {
      const nd = el("div", "node" + (depth > 0 ? " level-1" : "")); nd.dataset.path = node.path;
      nd.style.paddingLeft = (6 + depth * 16) + "px";
      nd.appendChild(el("span", "expander", depth < 2 ? "\u25BC" : "\u25B6"));
      nd.appendChild(el("span", "icon", "\u{1F4C1}"));
      nd.appendChild(el("span", "label", node.name));
      ft.appendChild(nd);
      if (depth < 2) Object.values(node.children).forEach((c) => walk(c, depth + 1));
    })(state.folderTree, 0);
    $$("#folders-tree .node").forEach((n) => n.onclick = (e) => {
      const exp = n.querySelector(".expander"); if (exp) exp.textContent = exp.textContent === "\u25BC" ? "\u25B6" : "\u25BC";
      $$("#folders-tree .node").forEach((x) => x.classList.remove("selected")); n.classList.add("selected");
      state.selFolder = n.dataset.path;
      const node = findFolderNode(state.folderTree, state.selFolder);
      const tracks = node ? tracksInFolder(node) : [];
      state._activeTracks = tracks;
      $("#folders-tracks-header").textContent = node ? node.name : "All Tracks";
      renderGridHeader($("#grid-header-folders")); renderGridRows($("#grid-rows-folders"), sortTracks(tracks), { grouped: false });
      $("#folders-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
      e.stopPropagation();
    });
    const root = state.folderTree; state.selFolder = root.path;
    const first = $(".node", ft); first && first.classList.add("selected");
    const tracks = tracksInFolder(root); state._activeTracks = tracks;
    $("#folders-tracks-header").textContent = root.name;
    renderGridHeader($("#grid-header-folders")); renderGridRows($("#grid-rows-folders"), sortTracks(tracks), { grouped: false });
    $("#folders-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
  }

  function renderPlaylistsView() {
    $$("#pl-list .lb-item").forEach((it) => it.onclick = () => {
      $$("#pl-list .lb-item").forEach((x) => x.classList.remove("selected")); it.classList.add("selected");
      const pl = it.dataset.pl;
      let tracks = [];
      if (pl === "favourites") tracks = state.tracks.filter((t) => (metaOf(t).rating || 0) >= 4);
      else if (pl === "recent") tracks = state.tracks.filter((t) => metaOf(t).play_count > 5);
      else if (pl === "roadtrip") tracks = state.tracks.filter((t) => t.genre === "Rock" || t.genre === "Electronic");
      else if (pl === "focus") tracks = state.tracks.filter((t) => t.genre === "Jazz" || t.genre === "Electronic");
      state._activeTracks = tracks;
      $("#pl-tracks-header").textContent = it.textContent;
      renderGridHeader($("#grid-header-pl")); renderGridRows($("#grid-rows-pl"), sortTracks(tracks), { grouped: false });
      $("#pl-count").textContent = tracks.length + " track" + (tracks.length === 1 ? "" : "s");
    });
    const first = $("#pl-list .lb-item"); first && first.click();
  }

  // ====================================================================
  //  Queue view
  // ====================================================================
  // Renders the queue in its own list. Current playing track is
  // highlighted; click plays from that point; small X removes the entry.
  function renderQueueView() {
    const headerEl = document.getElementById("grid-header-queue");
    const rowsEl = document.getElementById("grid-rows-queue");
    const countEl = document.getElementById("queue-count");
    const titleEl = document.getElementById("queue-tracks-header");
    if (!headerEl || !rowsEl) return;

    if (titleEl) titleEl.textContent = state.queueSource ? ("Queue \u2014 " + state.queueSource) : "Queue";
    if (countEl) {
      const total = state.queue.reduce((a, t) => a + durToSec(t.duration), 0);
      countEl.textContent = state.queue.length + " track" + (state.queue.length === 1 ? "" : "s") + "  \u00b7  " + fmtTime(total);
    }

    // Custom header matching the row layout (6 cells: #, Title, Artist, Album, Time, X)
    headerEl.innerHTML = "";
    [
      { l: "#", cls: "num" },
      { l: "Title" },
      { l: "Artist" },
      { l: "Album" },
      { l: "Time", cls: "num" },
      { l: "" },
    ].forEach((c) => headerEl.appendChild(el("div", "cell " + (c.cls || ""), c.l)));
    rowsEl.innerHTML = "";
    if (!state.queue.length) {
      const empty = el("div", "");
      empty.style.cssText = "padding:20px;color:var(--text-dim);text-align:center;font-style:italic";
      empty.textContent = "Queue is empty. Double-click a track in any view to start playback.";
      rowsEl.appendChild(empty);
      return;
    }
    const frag = document.createDocumentFragment();
    state.queue.forEach((t, i) => {
      const row = el("div", "row data" + (i === state.queueIdx ? " playing-row" : ""));
      row._track = t;
      row._qIdx = i;
      // # (queue position)
      const num = el("div", "cell num", String(i + 1));
      row.appendChild(num);
      // Title
      row.appendChild(el("div", "cell", t.title || titleFromPath(t.path)));
      // Artist
      row.appendChild(el("div", "cell", t.artist || ""));
      // Album
      row.appendChild(el("div", "cell", t.album || ""));
      // Duration
      row.appendChild(el("div", "cell num", t.duration || "0:00"));
      // Remove button
      const remove = el("div", "cell num", "\u2715");
      remove.title = "Remove from queue";
      remove.style.cssText = "cursor:pointer;color:#E08A7A;text-align:center";
      remove.onclick = (e) => { e.stopPropagation(); removeFromQueue(i); };
      row.appendChild(remove);
      // Click → play from this position
      row.onclick = () => {
        if (i === state.queueIdx) {
          togglePlay();
        } else {
          // Jump to this track in the queue
          state.queueIdx = i;
          if (state.mpdConnected) {
            tauriInvoke("mpd_play_idx", { index: i })
              .then(() => { syncMpdStatus(); })
              .catch((e) => console.warn("mpd_play_idx failed:", e));
          }
          // Find the track in state.tracks and use startLocalTrack for UI
          const ti = state.tracks.indexOf(t);
          if (ti >= 0) startLocalTrack(ti);
        }
      };
      row.ondblclick = () => {
        if (i === state.queueIdx) togglePlay();
      };
      frag.appendChild(row);
    });
    rowsEl.appendChild(frag);
  }
  function renderNowPlayingView() {
    const t = state.playingIdx >= 0 ? state.tracks[state.playingIdx] : null;
    paintArt(document.getElementById("npbig-art"), t, "No art");
    document.getElementById("npbig-title").textContent = t ? t.title : "\u2014";
    document.getElementById("npbig-artist").textContent = t ? t.artist : "\u2014";
    document.getElementById("npbig-album").textContent = t ? t.album + "  (" + t.year + ")" : "\u2014";
    document.getElementById("npbig-lyrics").textContent = t ? lyricsFor(t) : "No track is playing. Double-click a track in any view to start playback.";
  }

  // ====================================================================
  //  Album grid (shared)
  // ====================================================================
  function renderAlbumGrid(container, albums, scope) {
    container.innerHTML = "";
    const frag = document.createDocumentFragment();
    albums.forEach((al) => {
      const card = el("div", "album-card"); card._album = al;
      const art = el("div", "album-art");
      const st = artStyle(al.key);
      art.style.background = st.bg;
      art.appendChild(el("div", "aa-initial", st.initial));
      art.appendChild(el("div", "aa-genre", al.genre));
      const existing = state.albumArt.get(al.key);
      if (existing) setArtImage(art, existing);
      else if (existing === undefined) observeArt(art, al); // lazy-load when scrolled into view
      card.appendChild(art);
      card.appendChild(el("div", "album-title", al.album));
      card.appendChild(el("div", "album-artist", al.album_artist));
      card.appendChild(el("div", "album-meta", al.year + "  \u00b7  " + al.tracks.length + " trk"));
      if (state.selAlbum === al.key) card.classList.add("selected");
      card.onclick = () => {
        $$(".album-card", container).forEach((c) => c.classList.remove("selected")); card.classList.add("selected");
        state.selAlbum = al.key;
        if (scope === "aa") { $("#aa-tracks-header").textContent = al.album + " \u2014 " + al.album_artist; renderAATracks(); }
        else if (scope === "al") { $("#al-tracks-header").textContent = al.album + " \u2014 " + al.album_artist; renderALTracks(); }
      };
      card.ondblclick = () => { const idx = state.tracks.indexOf(al.tracks[0]); if (idx >= 0) playTrack(idx); };
      frag.appendChild(card);
    });
    container.appendChild(frag);
  }

  function bindListClick(list, kind, after) {
    $$(".lb-item", list).forEach((it) => it.onclick = () => {
      $$(".lb-item", list).forEach((x) => x.classList.remove("selected")); it.classList.add("selected");
      if (kind === "artist") state.selArtist = it.dataset.artist;
      if (kind === "genre") state.selGenre = it.dataset.genre;
      after();
    });
  }

  // ====================================================================
  //  Now Playing side panel (tabs)
  // ====================================================================
  function switchNpTab(name) {
    state.npTab = name;
    $$("#np-tabs .np-tab").forEach((t) => t.classList.toggle("active", t.dataset.nptab === name));
    $$(".np-panel").forEach((p) => p.classList.toggle("hidden", p.id !== "np-panel-" + name));
    renderNpPanels();
  }
  function lyricsFor(t) {
    return `[Instrumental placeholder]\n\nNo lyrics available for "${t.title}" by ${t.artist}.\n\nIn a real build, MusicBee fetches and saves lyrics per track and shows them here, optionally time-synced to playback.`;
  }
  // Build a signature for the Now Playing panels so we can skip the (heavy)
  // text + info-table rebuild on every status poll and only redo it when the
  // track or its rating actually changes.
  function npSignature(t) {
    return t ? (state.playingIdx + "\u0000" + metaOf(t).rating) : "none";
  }
  function renderNpPanels() {
    const t = state.playingIdx >= 0 ? state.tracks[state.playingIdx] : null;
    // album art (self-guarded; cheap to call every poll, lazily loads art)
    paintArt(document.getElementById("np-art"), t, "No art");
    // spectrum visualizer (built once; toggled on/off with playback)
    const sp = $("#np-spectrum");
    if (sp && !sp.dataset.built) { sp.dataset.built = "1"; for (let i = 0; i < 18; i++) { const b = el("div", "bar"); b.style.animationDelay = (i * 70) + "ms"; b.style.height = (8 + (i % 4) * 6) + "px"; sp.appendChild(b); } }
    sp && sp.classList.toggle("on", !!(state.isPlaying && t));
    // Nothing else changed since last render — skip the DOM churn.
    const sig = npSignature(t);
    if (sig === state._npSig) return;
    state._npSig = sig;
    // playing tab
    $("#np-title").textContent = t ? t.title : "\u2014";
    $("#np-artist").textContent = t ? t.artist : "\u2014";
    $("#np-album").textContent = t ? t.album : "\u2014";
    $("#np-genre").textContent = t ? t.genre : "\u2014";
    $("#np-year").textContent = t ? t.year : "\u2014";
    $("#np-track").textContent = t ? (t.track_number + " / " + (state.albums.find((a) => a.key === albumKey(t)) || { tracks: [] }).tracks.length) : "\u2014";
    // rating widget for current track
    const npRating = $("#np-rating");
    if (t) ratingWidget(npRating, metaOf(t).rating, (nv) => { setRating(t, nv); refreshRatingFor(t); renderNpPanels(); updateNowPlaying(); });
    else npRating.innerHTML = "";
    // stats
    $("#np-plays").textContent = t ? String(metaOf(t).play_count) : "\u2014";
    $("#np-skips").textContent = t ? String(metaOf(t).skip_count) : "\u2014";
    $("#np-added").textContent = t ? metaOf(t).date_added : "\u2014";
    // lyrics tab
    $("#np-lyrics").textContent = t ? lyricsFor(t) : "No track is playing.";
    // info tab
    const info = $("#np-info"); info.innerHTML = "";
    if (t) {
      const m = metaOf(t);
      const rows = [
        ["Title", t.title], ["Artist", t.artist], ["Album Artist", t.album_artist], ["Album", t.album],
        ["Genre", t.genre], ["Year", String(t.year)], ["Track #", String(t.track_number)],
        ["Duration", t.duration], ["Rating", "\u2605".repeat(m.rating) + "\u2606".repeat(5 - m.rating) || "unrated"],
        ["Play Count", String(m.play_count)], ["Skip Count", String(m.skip_count)], ["Date Added", m.date_added],
        ["File path", t.path],
      ];
      rows.forEach(([k, v]) => { const r = el("div", "np-row"); r.appendChild(el("span", "np-k", k)); r.appendChild(el("span", "np-v", v)); info.appendChild(r); });
    } else { info.appendChild(el("div", "", "No track selected.")); }
  }

  // ====================================================================
  //  View switching
  // ====================================================================
  function showEmptyIfNeeded() {
    const msg = document.getElementById("empty-library-msg");
    const empty = state.tracks.length === 0;
    if (msg) msg.style.display = empty ? "flex" : "none";
    // hide content grids when empty so empty message is visible
    document.querySelectorAll(".view.active .grid, .view.active .album-grid").forEach((g) => {
      g.style.display = empty ? "none" : "";
    });
  }
  // Schedule a re-render for a view in the next animation frame. Multiple
  // calls to the same view before the rAF fires get coalesced.
  function scheduleViewRender(name, fn) {
    if (state._pendingRenders.has(name)) return;
    state._pendingRenders.set(name, requestAnimationFrame(() => {
      state._pendingRenders.delete(name);
      try { fn(); } catch (e) { console.error("render " + name + " failed:", e); }
    }));
  }

  // Invalidate the view cache for views that depend on the active track list
  // (search, sort, library reload all change the signature).
  function invalidateViewCache() { state._viewCache.clear(); }

  const VIEW_LABELS = { music: "Music", albumsartists: "Albums & Artists", artists: "Artists", albums: "Albums", genres: "Genres", folders: "Folders", playlists: "Playlists", nowplaying: "Now Playing", queue: "Queue" };
  function switchView(name) {
    if (state.view === name) { updateQueueBadges(); return; }
    state.view = name;
    document.querySelectorAll("#nav-tabs .nav-tab").forEach((t) => t.classList.toggle("active", t.dataset.view === name));
    document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === "view-" + name));
    document.getElementById("ctx-title").textContent = VIEW_LABELS[name] || name;
    showEmptyIfNeeded();
    updateQueueBadges();
    if (state.tracks.length === 0 && name !== "queue") {
      updateNowPlaying(); updateStatus();
      return;
    }
    if (name === "music")            scheduleViewRender("music", renderMusicView);
    else if (name === "albumsartists") scheduleViewRender("albumsartists", renderAlbumsArtistsView);
    else if (name === "artists")     scheduleViewRender("artists", renderArtistsView);
    else if (name === "albums")      scheduleViewRender("albums", renderAlbumsView);
    else if (name === "genres")      scheduleViewRender("genres", renderGenresView);
    else if (name === "folders")     scheduleViewRender("folders", renderFoldersView);
    else if (name === "playlists")   scheduleViewRender("playlists", renderPlaylistsView);
    else if (name === "nowplaying")  scheduleViewRender("nowplaying", renderNowPlayingView);
    else if (name === "queue")       { state.queueDirty = true; scheduleViewRender("queue", renderQueueView); }
  }
  // ---- Queue management --------------------------------------------------
  // Maximum tracks to send to MPD in one shot. Keeps IPC payload and MPD
  // command size bounded. If the user wants a larger queue, they need
  // a smarter "load more on demand" strategy; for now 500 is plenty for
  // any album or auto-DJ mix.
  const QUEUE_MAX = 500;

  // Build a queue from the current view context. Returns {tracks, label}.
  function buildQueue(track) {
    // If the current view already has a filtered set, use it; else find
    // the album this track belongs to; else use all tracks.
    const active = state._activeTracks && state._activeTracks.length ? state._activeTracks : null;
    if (active && active.includes(track)) {
      const al = state.albums.find((a) => a.key === albumKey(track));
      const label = al ? al.album + " \u2014 " + al.album_artist : "Current view";
      return { tracks: active.slice(0, QUEUE_MAX), label };
    }
    // Fall back to this track's album
    const al = state.albums.find((a) => a.key === albumKey(track));
    if (al && al.tracks.length) {
      return { tracks: al.tracks.slice(0, QUEUE_MAX), label: al.album + " \u2014 " + al.album_artist };
    }
    // Last resort: all tracks, starting from this one
    const idx = state.tracks.indexOf(track);
    if (idx < 0) return { tracks: [track], label: track.album || "Single Track" };
    const all = state.tracks.slice(idx, idx + QUEUE_MAX);
    if (all.length < state.tracks.length - idx) all.push(...state.tracks.slice(0, QUEUE_MAX - all.length));
    return { tracks: all, label: "All Tracks" };
  }

  function playTrack(idx) {
    if (idx < 0 || idx >= state.tracks.length) return;
    const track = state.tracks[idx];
    const { tracks: qTracks, label } = buildQueue(track);
    const qIdx = qTracks.indexOf(track);
    if (qIdx < 0) return;
    // Update local queue state immediately for snappy UI
    state.queue = qTracks;
    state.queueIdx = qIdx;
    state.queueSource = label;
    state.queueDirty = true;
    updateQueueBadges();
    if (state.mpdConnected) {
      tauriInvoke("mpd_set_queue", { paths: qTracks.map((t) => t.path), index: qIdx })
        .then(() => { syncMpdStatus(); renderQueueIfActive(); })
        .catch((e) => { console.warn("mpd_set_queue failed:", e); setMpdStatus(false, String(e)); startLocalTrack(idx); });
    } else {
      startLocalTrack(idx);
    }
  }
  function startLocalTrack(idx) {
    state.playingIdx = idx; state.isPlaying = true; state.currentTime = 0;
    state.totalTime = durToSec(state.tracks[idx].duration);
    state.queueIdx = state.queue.indexOf(state.tracks[idx]);
    state.queueDirty = true;
    updateNowPlaying(); updatePlayerControls(); refreshAllGridSelections();
    if (state.view === "nowplaying") renderNowPlayingView();
    if (state.view === "queue") renderQueueView();
    updateStatus(); startTicker();
  }
  function togglePlay() {
    if (state.playingIdx < 0) { const base = state._activeTracks && state._activeTracks.length ? state._activeTracks : state.tracks; if (base.length) playTrack(state.tracks.indexOf(base[0])); return; }
    if (state.mpdConnected) { tryMpd("mpd_toggle_play"); return; }
    state.isPlaying = !state.isPlaying; if (state.isPlaying) startTicker(); else stopTicker();
    updatePlayerControls(); updateStatus(); refreshSpectrum();
  }
  function stopPlayback() {
    if (state.mpdConnected) { tryMpd("mpd_stop"); return; }
    state.isPlaying = false; state.currentTime = 0; stopTicker(); updatePlayerControls(); updateSeekUI(); updateStatus(); refreshSpectrum();
  }
  function nextTrack() {
    // When MPD is driving, just send next; syncMpdStatus will pick up the new track.
    if (state.mpdConnected) { tryMpd("mpd_next"); return; }
    // Local fallback: advance within the local queue (or active tracks).
    const list = state.queue.length ? state.queue : (state._activeTracks || state.tracks);
    if (!list.length) return;
    if (state.shuffle) {
      const n = Math.floor(Math.random() * list.length);
      const idx = state.tracks.indexOf(list[n]);
      if (idx >= 0) playTrack(idx);
      return;
    }
    const cur = state.playingIdx;
    const curInQ = cur >= 0 ? list.indexOf(state.tracks[cur]) : -1;
    let n = curInQ + 1;
    if (n >= list.length) n = state.repeat ? 0 : curInQ;
    if (n === curInQ && !state.repeat) return;
    const idx = state.tracks.indexOf(list[n]);
    if (idx >= 0) playTrack(idx);
  }
  function prevTrack() {
    if (state.mpdConnected) { tryMpd("mpd_previous"); return; }
    const list = state.queue.length ? state.queue : (state._activeTracks || state.tracks);
    if (!list.length) return;
    const cur = state.playingIdx;
    const curInQ = cur >= 0 ? list.indexOf(state.tracks[cur]) : -1;
    let n = curInQ - 1;
    if (n < 0) n = state.repeat ? list.length - 1 : 0;
    const idx = state.tracks.indexOf(list[n]);
    if (idx >= 0) playTrack(idx);
  }

  // ---- Queue manipulation -----------------------------------------------
  // Remove a single entry from the queue (by its index in state.queue).
  function removeFromQueue(qIdx) {
    if (qIdx < 0 || qIdx >= state.queue.length) return;
    // If we're connected to MPD, ask it to delete the position.
    if (state.mpdConnected) {
      tauriInvoke("mpd_delete_from_queue", { index: qIdx })
        .catch((e) => console.warn("mpd_delete_from_queue failed:", e));
    }
    state.queue.splice(qIdx, 1);
    if (qIdx < state.queueIdx) state.queueIdx--;
    else if (qIdx === state.queueIdx) state.queueIdx = -1;
    state.queueDirty = true;
    renderQueueIfActive();
  }

  function clearQueue() {
    if (state.mpdConnected) {
      tauriInvoke("mpd_clear_queue")
        .catch((e) => console.warn("mpd_clear_queue failed:", e));
    }
    state.queue = [];
    state.queueIdx = -1;
    state.queueSource = "";
    state.queueDirty = true;
    renderQueueIfActive();
  }

  // Re-render the queue view if it's the active view. Cheap if it's not.
  // Always updates the badges since they're visible across views.
  function renderQueueIfActive() {
    updateQueueBadges();
    if (state.view === "queue" && state.queueDirty) {
      state.queueDirty = false;
      renderQueueView();
    }
  }

  let ticker = null;
  function startTicker() { stopTicker(); ticker = setInterval(() => {
    if (!state.isPlaying) return;
    state.currentTime += 1;
    if (state.currentTime >= state.totalTime) {
      if (state.mpdConnected) { state.currentTime = state.totalTime; updateSeekUI(); return; }
      if (state.repeat) state.currentTime = 0; else { nextTrack(); return; }
    }
    updateSeekUI();
  }, 1000); }
  function stopTicker() { if (ticker) { clearInterval(ticker); ticker = null; } }

  function updatePlayerControls() { const p = $("#pb-play"); p.innerHTML = state.isPlaying ? "&#10074;&#10074;" : "&#9658;"; p.title = state.isPlaying ? "Pause" : "Play"; }
  function updateSeekUI() {
    const pct = state.totalTime ? (state.currentTime / state.totalTime) * 100 : 0;
    $("#pb-progress").style.width = pct + "%"; $("#pb-knob").style.left = pct + "%";
    $("#pb-current").textContent = fmtTime(state.currentTime); $("#pb-total").textContent = fmtTime(state.totalTime);
  }
  function refreshSpectrum() { const sp = $("#np-spectrum"); if (sp) sp.classList.toggle("on", !!(state.isPlaying && state.playingIdx >= 0)); }
  function updateNowPlaying() {
    const t = state.playingIdx >= 0 ? state.tracks[state.playingIdx] : null;
    renderNpPanels();
    // player-bar art (self-guarded; no rebuild unless the track/art changed)
    paintArt(document.getElementById("pb-art"), t, null);
    refreshSpectrum();
    // Skip the text + rating rebuild unless the track (or rating) changed.
    const sig = npSignature(t);
    if (sig === state._pbSig) return;
    state._pbSig = sig;
    document.getElementById("pb-np-title").textContent = t ? t.title : "No track selected";
    document.getElementById("pb-np-artist").textContent = t ? t.artist : "";
    const pbRating = document.getElementById("pb-rating");
    if (t) ratingWidget(pbRating, metaOf(t).rating, (nv) => { setRating(t, nv); refreshRatingFor(t); renderNpPanels(); updateNowPlaying(); });
    else pbRating.innerHTML = "";
  }
  function updateStatus() {
    const base = state._activeTracks; const n = base ? base.length : state.tracks.length;
    $("#status-count").textContent = n + " track" + (n === 1 ? "" : "s");
    $("#status-text").textContent = state.isPlaying && state.playingIdx >= 0
      ? "Playing: " + state.tracks[state.playingIdx].title + " — " + state.tracks[state.playingIdx].artist : "Ready";
  }

  async function syncMpdStatus() {
    const wasConnected = state.mpdConnected;
    try {
      const s = await tauriInvoke("get_mpd_status");
      state.mpdConnected = !!s.connected;
      setMpdStatus(state.mpdConnected, s.error || "");
      // reload library when MPD transitions from disconnected → connected
      if (state.mpdConnected && !wasConnected && state.tracks.length === 0) {
        try {
          const lib = await tauriInvoke("get_library");
          if (Array.isArray(lib) && lib.length > 0) {
            state.tracks = lib;
            deriveCollections();
            deriveMeta();
            buildTree();
            invalidateViewCache();
            switchView(state.view);
          }
        } catch (e) {
          console.warn("Library reload failed:", e);
        }
      }
      if (!s.connected) { state.playlistVersion = -1; return; }
      state.isPlaying = s.state === "play";
      state.currentTime = s.elapsed || 0;
      state.totalTime = s.duration || state.totalTime;
      if (s.volume >= 0 && s.volume <= 100) setVolume(s.volume, false);
      if (s.file) {
        let idx = state.tracks.findIndex((t) => t.path === s.file);
        if (idx < 0) idx = state.tracks.findIndex((t) => t.title === s.title && t.artist === s.artist && t.album === s.album);
        if (idx >= 0) state.playingIdx = idx;
      } else if (s.state === "stop" && s.playlist_length === 0) {
        state.playingIdx = -1;
      }
      // Reflect MPD's real playback modes (set by us or another client).
      if (typeof s.repeat === "boolean" && s.repeat !== state.repeat) {
        state.repeat = s.repeat;
        const rb = document.getElementById("pb-repeat"); if (rb) rb.classList.toggle("active", state.repeat);
      }
      if (typeof s.random === "boolean" && s.random !== state.shuffle) {
        state.shuffle = s.random;
        const sb = document.getElementById("pb-shuffle"); if (sb) sb.classList.toggle("active", state.shuffle);
      }
      // MPD is the source of truth for the queue. Only re-fetch the full
      // queue when its version counter changes (cheap on large libraries).
      if (s.playlist_version !== state.playlistVersion) {
        state.playlistVersion = s.playlist_version;
        await syncQueueFromMpd();
      }
      // Track the currently-playing position within the queue.
      const newQueueIdx = typeof s.song === "number" ? s.song : -1;
      if (newQueueIdx !== state.queueIdx) {
        state.queueIdx = newQueueIdx;
        state.queueDirty = true;
        renderQueueIfActive();
      }
      if (state.isPlaying) startTicker(); else stopTicker();
      updateSeekUI(); updatePlayerControls(); updateNowPlaying(); updateStatus();
      if (state.view === "nowplaying") renderNowPlayingView();
    } catch (e) {
      state.mpdConnected = false;
      setMpdStatus(false, String(e));
    }
  }

  // Pull MPD's actual queue (playlistinfo) and map each entry back to a
  // library track object where possible so artwork/metadata/selection all
  // resolve against the same objects the rest of the UI uses.
  async function syncQueueFromMpd() {
    try {
      const qTracks = await tauriInvoke("get_queue");
      if (!Array.isArray(qTracks)) return;
      const byPath = new Map(state.tracks.map((t) => [t.path, t]));
      state.queue = qTracks.map((qt) => byPath.get(qt.path) || qt);
      if (!state.queueSource) {
        state.queueSource = state.queue.length ? "MPD queue" : "";
      } else if (!state.queue.length) {
        state.queueSource = "";
      }
      state.queueDirty = true;
      renderQueueIfActive();
    } catch (e) {
      console.warn("get_queue failed:", e);
    }
  }

  function setMpdStatus(connected, detail) {
    const el = $("#mpd-status");
    const banner = $("#mpd-banner");
    const message = detail || "Start MPD on 127.0.0.1:6600, or set MPD_HOST/MPD_PORT before launching.";
    if (el) {
      el.classList.toggle("connected", connected);
      el.classList.toggle("disconnected", !connected);
      el.textContent = connected ? "MPD 127.0.0.1:6600" : "MPD disconnected";
      el.title = connected ? "Connected to MPD on 127.0.0.1:6600" : message;
    }
    if (banner) {
      // Three states: disconnected (red), connected-with-MPD-error (yellow),
      // and connected-OK (hidden). MPD's `error` field is sticky — e.g.
      // "Failed to enable output" — so we surface it as a banner instead
      // of silently keeping the app in a "playing but no sound" state.
      banner.classList.toggle("disconnected", !connected);
      banner.classList.toggle("error", connected && !!detail);
      if (!connected) {
        banner.textContent = "MPD disconnected: " + message;
      } else if (detail) {
        banner.textContent = "MPD audio error: " + detail;
      }
    }
  }

  // ---- Volume / seekbar ---------------------------------------------------
  function setVolume(v, sendMpd = true) { state.volume = Math.max(0, Math.min(100, v|0)); $("#pb-volume").value = state.volume; $("#pb-vol-val").textContent = state.volume; if (sendMpd) tryMpd("mpd_set_volume", { volume: state.volume }); }
  let seeking = false;
  function seekFromMouse(e) { const r = $(".pb-seekbar").getBoundingClientRect(); let pct = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)); state.currentTime = Math.floor(pct * state.totalTime); updateSeekUI(); }

  // ====================================================================
  //  Bindings
  function bindAll() {
    $("#win-min").onclick = () => tauriInvoke("window_minimize").catch(console.warn);
    $("#win-max").onclick = () => tauriInvoke("window_toggle_maximize").catch(console.warn);
    $("#win-close").onclick = () => tauriInvoke("window_close").catch(console.warn);
    $$("#nav-tabs .nav-tab").forEach((t) => t.onclick = () => switchView(t.dataset.view));
    // NP tabs
    $$("#np-tabs .np-tab").forEach((t) => t.onclick = () => switchNpTab(t.dataset.nptab));
    // context toolbar
    $$("#layout-seg .seg-btn").forEach((b) => b.onclick = () => {
      $$("#layout-seg .seg-btn").forEach((x) => x.classList.remove("active")); b.classList.add("active");
      state.layout = b.dataset.layout;
      if (["music", "artists", "genres"].includes(state.view)) switchView(state.view);
    });
    $("#btn-columns").onclick = () => {};
    $("#btn-np-toggle").onclick = () => {
      const np = $("#nowplaying"), sp = $("#np-splitter");
      const hidden = np.style.display === "none";
      np.style.display = hidden ? "" : "none"; sp.style.display = hidden ? "" : "none";
      $("#btn-np-toggle").classList.toggle("active", hidden);
    };
    // search
    // Debounce helper — only re-render the music view after 120ms of idle.
    const debouncedMusicRender = debounce(() => { if (state.view === "music") scheduleViewRender("music", renderMusicView); }, 120);
    const search = $("#search-box");
    search.oninput = () => { state.search = search.value; $("#btn-search-clear").disabled = !search.value; debouncedMusicRender(); };
    $("#btn-search-clear").onclick = () => { search.value = ""; state.search = ""; $("#btn-search-clear").disabled = true; if (state.view === "music") scheduleViewRender("music", renderMusicView); };
    // quick filter
    $$("#quickfilter .qf-input").forEach((inp) => inp.oninput = () => { state.quick[inp.dataset.col] = inp.value; debouncedMusicRender(); });
    // grid clicks (all views)
    bindGridClicks($("#grid-rows-music"), (t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-aa"),    (t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-art"),   (t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-al"),    (t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-gn"),    (t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-folders"),(t) => playTrack(state.tracks.indexOf(t)));
    bindGridClicks($("#grid-rows-pl"),    (t) => playTrack(state.tracks.indexOf(t)));
    // header sort
    $$(".row.header").forEach((hdr) => hdr.onclick = (e) => {
      const cell = e.target.closest(".cell"); if (!cell) return; const col = cell.dataset.col; if (!col || col === "rating") return;
      if (state.sort.col === col) state.sort.dir = state.sort.dir === "asc" ? "desc" : "asc";
      else { state.sort.col = col; state.sort.dir = "asc"; }
      invalidateViewCache();
      switchView(state.view);
    });
    $("#pb-play").onclick = togglePlay; $("#pb-next").onclick = nextTrack; $("#pb-prev").onclick = prevTrack; $("#pb-stop").onclick = stopPlayback;
    $("#pb-volume").oninput = (e) => setVolume(e.target.value);
    $("#pb-shuffle").onclick = () => { state.shuffle = !state.shuffle; $("#pb-shuffle").classList.toggle("active", state.shuffle); if (state.mpdConnected) tryMpd("mpd_set_random", { enabled: state.shuffle }); };
    $("#pb-repeat").onclick = () => { state.repeat = !state.repeat; $("#pb-repeat").classList.toggle("active", state.repeat); if (state.mpdConnected) tryMpd("mpd_set_repeat", { enabled: state.repeat }); };
    $("#pb-eq").onclick = () => { state.eq = !state.eq; $("#pb-eq").classList.toggle("active", state.eq); };
    // seekbar
    $(".pb-seekbar").onmousedown = (e) => { seeking = true; seekFromMouse(e); };
    window.addEventListener("mousemove", (e) => { if (seeking) seekFromMouse(e); });
    window.addEventListener("mouseup", () => { if (seeking) tryMpd("mpd_seek_current", { seconds: state.currentTime }); seeking = false; });

    // ---- Queue UI wiring ----
    const goQueue = () => switchView("queue");
    const qbtn = document.getElementById("btn-queue-toggle");
    if (qbtn) qbtn.onclick = (e) => { e.stopPropagation(); goQueue(); };
    const pqbtn = document.getElementById("pb-queue");
    if (pqbtn) pqbtn.onclick = () => { goQueue(); };
    const clr = document.getElementById("btn-queue-clear");
    if (clr) clr.onclick = () => clearQueue();

    // ---- Resizable splitters ----
    initSplitters();
  }

  // Update the queue count badges in the nav tab, titlebar, and player bar.
  function updateQueueBadges() {
    const n = state.queue.length;
    const show = n > 0;
    ["queue-badge", "queue-badge-titlebar", "pb-queue-badge"].forEach((id) => {
      const e = document.getElementById(id);
      if (!e) return;
      e.textContent = String(n);
      e.style.display = show ? "" : "none";
    });
    // active state on titlebar button when queue view is showing
    const tb = document.getElementById("btn-queue-toggle");
    if (tb) tb.classList.toggle("active", state.view === "queue");
  }

  // ====================================================================
  //  Resizable splitters
  // ====================================================================
  // Each splitter lives between two panes. We look at the splitter's
  // previousElementSibling and nextElementSibling to find the panes it
  // controls. Sizes are persisted in localStorage under "splitter:<id>".
  // If a pane has no inline width/flex-basis, we set its width directly.
  function initSplitters() {
    const splitters = document.querySelectorAll(".splitter-v");
    splitters.forEach((sp) => bindSplitterDrag(sp));
    // Apply saved sizes on next tick so layout has stabilised
    requestAnimationFrame(() => applySavedSplitterSizes());
  }

  function applySavedSplitterSizes() {
    document.querySelectorAll(".splitter-v").forEach((sp) => {
      const id = sp.dataset.target;
      if (!id) return;
      const target = document.querySelector(id);
      if (!target) return;
      const saved = localStorage.getItem("splitter:" + id);
      if (saved) {
        const px = parseInt(saved, 10);
        if (!isNaN(px) && px > 40) {
          target.style.flex = "0 0 " + px + "px";
          target.style.width = px + "px";
        }
      }
    });
  }

  function bindSplitterDrag(sp) {
    // Determine which adjacent pane to resize. The "left" pane is the
    // previousElementSibling that isn't a splitter, the "right" pane is
    // nextElementSibling that isn't a splitter. Some splitters explicitly
    // set data-target="<css selector>" for fixed-target resizing.
    let target = null;
    if (sp.dataset.target) {
      target = document.querySelector(sp.dataset.target);
    } else {
      let p = sp.previousElementSibling;
      while (p && p.classList.contains("splitter-v")) p = p.previousElementSibling;
      if (p) target = p;
    }
    if (!target) return;
    target.dataset.splittable = "1";
    if (!sp.dataset.target) sp.dataset.target = "#" + (target.id || "") || (target.tagName.toLowerCase() + (target.className ? "." + target.className.trim().split(/\s+/).join(".") : ""));
    // Make the target have an explicit width we can mutate
    if (!target.style.flex && !target.style.width) {
      const rect = target.getBoundingClientRect();
      target.style.flex = "0 0 " + rect.width + "px";
      target.style.width = rect.width + "px";
    }
    sp.addEventListener("mousedown", (e) => {
      e.preventDefault();
      sp.classList.add("dragging");
      document.body.classList.add("resizing");
      const startX = e.clientX;
      const rect = target.getBoundingClientRect();
      const startW = rect.width;
      const minW = 60, maxW = window.innerWidth - 100;
      const onMove = (ev) => {
        const dx = ev.clientX - startX;
        let newW = startW + dx;
        if (newW < minW) newW = minW;
        if (newW > maxW) newW = maxW;
        target.style.flex = "0 0 " + newW + "px";
        target.style.width = newW + "px";
      };
      const onUp = () => {
        sp.classList.remove("dragging");
        document.body.classList.remove("resizing");
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        // Persist (use a stable key: the target's data-target selector)
        const key = "splitter:" + (sp.dataset.target || target.id || "x");
        try { localStorage.setItem(key, String(Math.round(target.getBoundingClientRect().width))); } catch (e) {}
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    });
  }

  // ====================================================================
  //  Boot
  // ====================================================================
  async function init() {
    try {
      state.tracks = await loadLibrary();
    } catch (e) {
      console.error("Failed to load library:", e);
      state.tracks = [];
      setMpdStatus(false, "Library load failed: " + e.message);
    }
    deriveCollections(); deriveMeta();
    buildTree();
    bindAll();
    switchView("music");
    switchNpTab("playing");
    setVolume(state.volume, false);
    updateSeekUI(); updatePlayerControls(); updateNowPlaying(); updateStatus();
    await syncMpdStatus();
    state.mpdStatusTimer = setInterval(syncMpdStatus, 3000);
  }
  document.addEventListener("DOMContentLoaded", init);
})();
