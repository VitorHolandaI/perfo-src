# `src/tui/cpu` — the four-pane dashboard and its fullscreen views

Draws everything the interactive TUI shows apart from the history mode and the
overlays' content: the CPU/MEM/IO/NET quadrants, each pane's fullscreen
expansion, and the process table. Nothing here collects: it reads a
`CpuSnapshot` produced by `src/data` and turns it into ratatui `Line`s.

Callers outside this folder touch `draw` and the `Ui` struct in `mod.rs`.
Everything else is `pub(super)` or private.

## Files

- **`mod.rs`** — `Ui`, the borrowed view model every drawing function takes,
  plus `Pane`, `Row`, `SortKey` and the top-level `draw` that routes to a
  quadrant layout or a fullscreen pane. Holds the unit tests for `format.rs`.
- **`format.rs`** — the shared vocabulary: `human_bytes`, `short_bytes`,
  `bar`, `block`, `pipe`, `truncate`, `temp_color` and the two trend graphs.
  `sparkline` buckets a whole ring into `width` columns and suits a graph
  about as wide as the ring is long, like the CPU history. `sparkline_recent`
  draws the newest `width` samples one per column and is what the narrow
  per-row rate graphs use, so the line scrolls a step on every refresh.
  Imported by every other file here.
- **`panes.rs`** — the CPU quadrant and its fullscreen view: per-core bars,
  load, frequency, temperature, and the process table.
- **`io.rs`** — the IO quadrant: per-disk rates, queue depth, temperature.
- **`net.rs`** — the NET pane frame: totals, per-interface rows
  (`busiest_interfaces` keeps the idle `veth`/bridge links from pushing the
  tables off screen), the per-process socket table and the listening-port
  table. Delegates the port traffic table to `port_table.rs`.
- **`port_table.rs`** — the PORT TRAFFIC table alone, split out when `net.rs`
  passed the 500 line ceiling. Owns `row_scale`, the decision that each row is
  drawn against its own peak rather than one scale shared across the table.
  `draw_net` in `net.rs` is the only caller.
- **`overlay.rs`** — the help and kill-confirmation overlays painted on top of
  whatever pane is active.

## Flow

`tui::run` builds a `Ui` borrowing the current `CpuSnapshot`
→ `mod::draw` picks quadrants or a fullscreen pane
→ `panes` / `io` / `net` push `Line`s, calling into `format` for every cell
→ `net::draw_net` calls `port_table::port_traffic_rows` for the port table
→ `overlay` paints last, over the result.

## Why the graphs scale the way they do

A rate graph scaled to one value shared across a table collapses every row
quieter than the busiest to a flat line, which throws away the history the
graph exists to show. Each row is therefore scaled to its own peak, the way
bpytop scales each of its graphs, and the rate column beside the graph carries
the magnitude that the height no longer can.
