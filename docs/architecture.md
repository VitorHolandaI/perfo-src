# Implementation Decisions

## Engine and plugin boundary

The Rust binary owns collection, deltas, history, process attribution, and
JSON serialization. The Quickshell plugin owns presentation, navigation, and
launching the TUI. This avoids duplicating collectors in QML and keeps the
TUI and bar widget on the same data contract.

## Data source priority

Use a regular file or standard kernel interface first. Use a syscall only
when the metric has no file equivalent. Current examples are:

- procfs for CPU, memory, network, and process data.
- sysfs/hwmon for fans, temperatures, and optional hardware values.
- DRM sysfs for AMD GPU values.
- DRM `/proc/<pid>/fdinfo` engine times for Intel i915 utilization.
- dynamically loaded NVML for NVIDIA GPU values when the driver is available.

This follows the project data map and the Linux interfaces documented in
`docs/hardware/`.

## Honest absence

Metrics are optional by hardware and driver. An unavailable GPU utilization,
temperature, fan, or power reading is `null` or absent from the relevant
detail list. Zero is used only when the source actually reports zero, such as
a stopped fan or an idle counter delta.

## Portability

Never identify hardware by `hwmonN` or `cardN` alone. Resolve the device by
chip name, DRM vendor/driver, or the relevant sysfs relation. Keep display
labels bounded and sanitize kernel-provided strings before they reach QML.

## Process naming

Short names and command lines are different fields. The short process name is
for compact UI labels; the full command line remains available for the TUI.
This prevents arguments containing paths or JSON from corrupting the label.

## Dashboard language

Use explicit labels:

- `TOP PROCESSES` for process summaries.
- `COOLING / FANS` for fan RPM values.
- `Storage` for filesystem capacity.
- `Disk I/O` for read/write activity.

Do not make a compact `TOP` label carry an ambiguous meaning.

## Sampling and UI

The stream emits complete snapshots once per second and flushes after each
line. The plugin must replace the snapshot reference rather than mutate a
nested object in place, so QML bindings update.

The bar should prefer the busiest logical CPU core when presenting a single
CPU headline. The aggregate CPU value remains available for the detail view.

## Security boundary

The plugin runs unsandboxed inside the shell. It must not install packages,
load modules, write hardware-control files, or execute user-provided command
strings. Optional integrations such as NVIDIA NVML must be bounded and
explicitly documented; loading the shared library must remain optional.

## Release dependency scope

The current release keeps `crossterm`, `libc`, `ratatui`, `serde`,
`serde_json`, and `sysinfo`. The resolved lockfile contains platform-specific
packages that are not all compiled into a Linux build. Direct Linux file-first
collection remains the preferred approach, but a zero-dependency implementation
is deferred to a future release.

Supply-chain validation for Rust changes uses `cargo fmt --check`, clippy with
warnings denied, tests, `cargo audit`, and `cargo deny check`, plus `rustqual`
and `rust-doctor` when installed. The current audit reports allowed transitive
warnings for `paste` and `lru` through `ratatui`; they are tracked rather than
hidden.

GitHub runs the dependency inventory from
`.github/workflows/dependencies.yml`. The workflow uses the committed
`Cargo.lock`, prints the dependency tree and duplicate versions in the job
summary, and runs dependency review on pull requests. GitHub Actions are pinned
to commit SHAs and the release workflow separates read-only builds from the
write-capable publication job.
The dedicated `.github/workflows/security.yml` runs the advisory and policy
checks.

## Current gaps

- Fake hwmon fixture trees covering multiple notebook shapes are not yet in
  the repository.
- NVIDIA GPU support is optional and uses dynamically loaded NVML.
- GPU values are exposed in JSON and the widget GPU page; richer device memory
  and power fields remain backend-dependent.
- QML validation currently checks manifest references; a full Quickshell runtime
  test is not available on the CI runner.
