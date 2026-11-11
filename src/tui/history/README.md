# `src/tui/history/` — the flight recorder's TUI page

This folder is the HISTORY page: the live rolling chart, the session recorder,
and the replay of a saved recording. It owns the only writer of
`HistorySample`, which means it also owns the on-disk shape that
`src/recordings/` persists and that `HistoryPage.qml` reads back. Change a
field here and three other places have to follow; `src/recordings/README.md`
lists them.

It reads `CpuSnapshot` from `src/data/cpu/` and writes through
`src/recordings/`. Nothing here talks to `/proc` directly.

## The two things this folder is

**A ring buffer of live samples** (`samples`, capped at `max_samples`), drawn
as a chart you can scrub. This exists whether or not a recording is running.

**A session recorder** that, while armed, copies each sample into
`session_record_buffer` and on stop hands the whole buffer to
`save_session_data`. Nothing reaches disk until then, which is where the RAM
cost of a long recording comes from.

## Files

`mod.rs` — the types everything else here shares: `HistorySample` and
`HistoryProcess` (the serialised shape), `SampleThermals`, `HistoryMetric` (the
chart's six pages: CPU, MEM, IO, NET, GPU, TEMP), `HistorySpan`, and the
`HistoryState` that holds all of it. The entry point for the rest of the TUI.

`record.rs` — the collector side. `record_snapshot` folds one `CpuSnapshot`
into a `HistorySample`: process depth, disk and net rates, GPU attribution, and
`thermals`, which reduces each per-device list to its hottest member. Also
applies the `RecordingMask`, zeroing the subsystems the picker left unticked.
Called once per tick from the TUI loop.

`session.rs` — recording lifecycle and the picker's row constants
(`DEPTH_ROW_IDX` and friends). Those constants assume the subsystem checkboxes
come first, and `modals::tests::the_picker_rows_line_up_with_the_mask_flags`
pins them to `RecordingMask`, so adding a flag fails a test instead of
silently pointing the depth row at the new checkbox. Loads a saved recording
through `crate::recordings::load_recording_payload`.

`duration.rs` — turning a picked duration into a sample count, including the
custom-entry parsing and its error messages.

`nav.rs` — cursor movement and playback: scrubbing, play/pause, jump to live.
Pure state, no drawing.

`draw.rs` — the page layout and the control bar, including the metric and span
pills. The top-level `draw` the TUI calls; delegates the chart to `chart.rs`
and everything under it to `tables.rs`.

`chart.rs` — the bar chart. `metric_ceiling` decides the Y axis per metric
(percentages and temperatures are fixed at 100, rates scale to what is in
view); `bar_column_spans` picks each column's peak. The densest drawing here.

`tables.rs` — the SAMPLE/PEAKS/AVG strip and the per-process table under the
chart. NET gets its own table shape. On the TEMP page the strip grows a third
line with the per-sensor breakdown and the per-core temperatures, because the
shared strip has no room for four more cells on an 80-column terminal.

`modals.rs` — the saved-sessions list and the recording picker, including
`record_modal_items`, the one list of subsystem checkboxes.

`export.rs` — a recording as a text or JSON report, plus `sort_processes` and
`metric_cell_text`, which decide what the process table shows per metric. TEMP
sorts and reads as CPU does, since no process has a temperature: what a
thermal page wants from the table is what was running while the machine
heated up.

`format.rs` — the local timestamp on a sample and process-name cleanup.

`traceable.rs` — whether a process in a replayed sample is still the same
process, so the replay does not offer to trace a reused pid.

## Flow

```
TUI tick ──> record.rs::record_snapshot ──> samples (ring)
                                       └──> session_record_buffer (while armed)
                                                 │ on stop
                                                 ▼
                                   session.rs::stop_and_save_session
                                                 ▼
                                   crate::recordings::save_session_data

draw.rs::draw ──> chart.rs (the bars)
             └──> tables.rs (the strip and the process table)
```

## Adding a chart page

`HistoryMetric` is matched exhaustively in `chart.rs` (twice), `tables.rs` and
`export.rs` (twice), so the compiler names every site. Add the variant, then
let it fail its way through. Remember `draw.rs`'s pill list and the matching
`model:` arrays in `HistoryPage.qml`, neither of which the compiler checks.
