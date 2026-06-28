# musicbee-iced

Native Rust MusicBee-style MPD client built with [`iced`](https://iced.rs/).

The app talks directly to MPD over TCP (`MPD_HOST`, `MPD_PORT`, and
`MPD_PASSWORD` are supported) and renders a native desktop UI: library search,
track list, album art, and a bottom dock with transport controls.

```sh
cargo run
```
