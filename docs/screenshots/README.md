# Screenshots

The SVGs in this directory are generated from the **real renderer**, not drawn by
hand. `examples/screenshots.rs` creates an `App` with the embedded catalogs, uses a
ratatui `TestBackend` at 100×30, and converts the buffer to SVG.

Regenerate:

```sh
cargo run --example screenshots
```

Files:

- `splash.svg` — splash and wordmark
- `board.svg` — board picker with level summaries and file counts
- `browse.svg` — PDF table, search/filter bar and multi-select markers
- `queue.svg` — compact download rows, retry messaging and progress meters
- `settings.svg` — grouped configuration rows
- `help.svg` — two-column keyboard-shortcut panel

The generator uses a simulated queue (including a compact retry state) and the
checked-in catalogs. Editing an SVG by hand will be overwritten the next time the
generator runs.

Rendering tests in `src/ui/mod.rs` exercise the same screens at the minimum
supported terminal size, 80×24. The documentation images use 100×30 for a little
more breathing room.
