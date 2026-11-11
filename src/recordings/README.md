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

`timeline.rs` — **reduce.** `record timeline` turns every sample into the
handful of scalar series the chart draws, caching the result in a
`.timeline.json` sidecar. Owns `TIMELINE_SCHEMA`: a sidecar from an older
reducer is rebuilt even though it is newer than its source, because the broken
one always is. `HistoryPage.qml` reads the sidecar directly and checks the same
number in its `timelineSchema` property, so **bump both together**.

Exports `process_list` and `sidecar_is_current` to `offsets.rs`, which needs
the same sample-shape rule and the same freshness rule.

The thermal series are the exception to this file's "never a null" rule. Every
other series defaults a missing value to 0, which draws as a gap; zero degrees
is instead a real cold measurement, so `temp_field` and `per_core_temps` keep
`null`. Inventing a cold machine out of an absent sensor is not a gap, it is a
wrong reading, and Intel GPUs report no temperature at all.

`offsets.rs` — **index.** `record inspect` returns the process table around one
sample. `load_offsets` scans the byte offset of each sample once into a
`.offsets.bin` sidecar of fixed 16-byte `(u64 start, u64 end)` pairs, so
scrubbing seeks with `lseek` instead of parsing 51 MB. That sidecar is gated on
mtime only, with no schema: its format has never changed, but if it ever does
it needs the same treatment `timeline.rs` gives its own.

`test_support.rs` — `#[cfg(test)]` only. A throwaway recording store on disk
(`recording_fixture`) and a way to age a file (`filetime_set`), shared by the
tests of `timeline.rs` and `offsets.rs` because both exercise the sidecar
freshness checks.

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

## Changing what a sample holds

Adding a series touches seven places, and missing one of them fails quietly
rather than loudly:

1. `HistorySample` in `src/tui/history/mod.rs` — the field, with
   `#[serde(default)]` so older recordings still load.
2. `HistoryState::record_snapshot` in `src/tui/history/record.rs` — filling it,
   and zeroing it when its `RecordingMask` flag is off.
3. `CollectionProfile::History` in `src/data/cpu/collection.rs` — the collector
   flag, if the data is gated. Without it the *live* History page keeps
   whatever the visible page happened to collect.
4. `RecordingMask::to_needs`, in the same file, and this one is the trap: while
   `plan.recording` is on, `CollectionPlan::needs` throws History's own needs
   away and uses the mask's instead. A flag that only item 3 sets is never
   collected during a recording, which is the one case it was added for. Check
   whether the reading rides on a device list, too: disk and GPU temperatures
   need `disks` and `gpu` on, so `to_needs` turns those on for `temp`.
5. `recordingMask` in `BarWidget.qml` — the widget's picker reaches the
   collector only through this string, and the collector ignores a name it does
   not know, so an omission here reads as "the user unticked it".
6. `timeline_sample` here, plus a `TIMELINE_SCHEMA` bump and the matching
   `timelineSchema` in `HistoryPage.qml`.
7. `sampleFromSnapshot` in `Panel.qml`, so widget-made recordings carry it too.
   The two producers disagree on shape already; do not widen the gap.
