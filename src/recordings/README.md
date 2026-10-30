# `src/recordings/` — the flight recorder's on-disk format

This folder owns everything about a *saved* recording: where it lives, how it
is written, how a long one is scrubbed without being loaded whole, and the
`perfo record …` subcommands that expose all of it.

It has two producers and two consumers, which is the single most important
thing to know before changing anything here:

| | produces a recording | replays one |
| --- | --- | --- |
| **TUI** | `src/tui/history/session.rs` calls `save_session_data` with `Vec<HistorySample>` | `HistoryState::load_session` via `load_recording_payload` |
| **widget** | `HistoryPage.qml` pipes JSON into `perfo record save` | `HistoryPage.qml` calls `record timeline` and `record inspect` |

The two producers do not write identical JSON. `HistorySample`
(`src/tui/history/mod.rs:136`) serialises `cpu`/`mem`/`gpu` as `f32` and the
process list as `top_procs`; the widget rounds those three to integers and
names the list `processes`. Anything in this folder that reads a recorded
sample has to accept both shapes. Reading only one of them is what made a
TUI-made recording replay as three flat lines at zero in the widget.

## Files

`mod.rs` — the module's public surface and the `dispatch` router that turns
`perfo record <subcmd>` into one of the calls below. Owns `RecordingMetadata`
(what the pickers list) and `RecordingPayload` (a whole recording in memory).
Called from the CLI layer; calls every other file here.

`paths.rs` — decides where recordings live (`$XDG_DATA_HOME/perfo/recordings`,
overridable with `PERFO_RECORDINGS_DIR`) and resolves a user-supplied id or
path to a real file, refusing anything outside the store. Every other file in
this folder asks it for the directory rather than building the path itself.

`format.rs` — the duration labels (`1h 10m`) and the local timestamp strings
that name a recording (`rec-20261025-002943`). Used by `store.rs` when saving
and by `timeline.rs` when a recording has no `duration_label`.

`store.rs` — the read/write/list/delete half. `save_session_data` is the one
entry point that writes a recording, so both producers get the same header;
`prune_recordings` keeps the store at `DEFAULT_MAX_RECORDINGS`. Reads go
through `load_recording_payload`, which the TUI replay uses.

`timeline.rs` — the half that makes a long recording usable. Two jobs:

1. **Reduce.** `record timeline` turns every sample into the handful of series
   the chart draws, caching the result in a `.timeline.json` sidecar. The
   sidecar carries `TIMELINE_SCHEMA`; a sidecar from an older reducer is
   rebuilt even though it is newer than its source, because the broken one
   always is. `HistoryPage.qml` reads the sidecar directly and checks the same
   number in its `timelineSchema` property, so bump both together.
2. **Index.** `record inspect` returns the process table around one sample.
   `load_offsets` scans the byte offset of each sample once into a
   `.offsets.bin` sidecar, so scrubbing seeks instead of parsing 51 MB.

`export.rs` — `record export`-style reports written to the user's home from a
JSON payload on stdin. Independent of the store: it never touches the
recordings directory.

## Flow

Saving: producer → `save_session_data` → `prune_recordings` → a `.json` in the
store. The two sidecars are written lazily, on the first `timeline` or
`inspect` of that file, and are always best-effort: a read-only store still
replays, it just rebuilds them each time.

Replaying in the widget: `record timeline` for the chart, then one
`record inspect <path> <index> 25` per scrub position for the process table.
