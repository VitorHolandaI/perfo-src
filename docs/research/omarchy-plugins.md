# Omarchy Plugin Research

This page compares public Omarchy system-monitor plugins that informed the
`perfo` design. The comparison is based on their READMEs, manifests, and
collector sources. These projects are references, not dependencies.

## omarchy-sysmonitor

Source: [PixelatedContinuum/omarchy-sysmonitor](https://github.com/PixelatedContinuum/omarchy-sysmonitor)

Manifest version: `3.0.0`, author `jharrison`.

Implementation:

- QML panel and bar widget.
- `Model.js` contains parsing and formatting helpers.
- QML `Process` objects run shell collectors.
- Reads procfs and sysfs, including `/proc/stat`, `/proc/net/dev`,
  `/proc/diskstats`, `/proc/pressure`, and hwmon paths.
- Uses an AMD GPU path based on `gpu_busy_percent` and VRAM sysfs files.
- Its public README describes AMD GPU support; the reviewed source does not
  establish equivalent device-wide utilization for every GPU vendor.

Useful ideas:

- One aggregated dashboard plus focused detail views.
- Per-core charts and top-process details.
- Hardware discovery by driver or sensor identity instead of fixed indices.

Limitations relevant to `perfo`:

- It is primarily a QML and shell collector architecture.
- AMD sysfs support cannot be treated as a universal GPU backend.

References: [README](https://github.com/PixelatedContinuum/omarchy-sysmonitor#readme),
[Model.js](https://github.com/PixelatedContinuum/omarchy-sysmonitor/blob/main/Model.js).

## System Monitor

Marketplace: [Omarchy Plugin Marketplace](https://plugins.omarchy.org/)

Source: [rmacy/omarchy-system-monitor](https://github.com/rmacy/omarchy-system-monitor)

The marketplace listing describes a live CPU, memory, network, load, uptime,
sensor, process-list, and Task Manager plugin. It is a useful visual reference
for a dense system panel and for how Omarchy presents install, compatibility,
license, and verification metadata.

Relevant distinction for `perfo`:

- The listing installs an upstream repository with
  `omarchy plugin add ... --enable`.
- Marketplace verification covers one exact commit; the listing warns that
  installing the repository HEAD can select a different commit.
- Third-party plugins run unsandboxed, so source inspection remains part of the
  install decision.
- `perfo` keeps its collector in one Rust binary and uses QML only as a thin
  view, while this plugin is primarily a shell/QML reference.

References: [marketplace](https://plugins.omarchy.org/),
[plugin development guide](https://plugins.omarchy.org/develop.html),
[upstream repository](https://github.com/rmacy/omarchy-system-monitor).

## MenuVitals

Source: [ierror/menuvitals](https://github.com/ierror/menuvitals)

Manifest version: `0.1.2`, id `io.github.ierror.menuvitals`.

Implementation:

- A `service` owns shared sampling for the bar widget and panel.
- QML `FileView` reads bounded procfs and sysfs values directly.
- `probe.sh` resolves changing DRM, hwmon, block, mount, and network paths.
- `procs.sh` samples `/proc/<pid>/stat` twice and calculates live CPU deltas.
- GPU discovery recognizes `amdgpu`, `i915`, `xe`, `nouveau`, `nvidia`, and
  `radeon`, but utilization is only shown when the selected driver exposes a
  usable device-wide load source.
- NVIDIA uses a bounded `nvidia-smi --query-gpu ...` stream when required.
- The bar can show CPU, GPU, memory, disk, network, and battery metrics.

Useful ideas:

- Shared service instead of one collector per visible widget.
- Discovery separated from recurring sampling.
- Explicit omission of Intel GPU load when no trustworthy device-wide source
  exists.
- Process name taken from `/proc` data rather than a full command line.

Limitations relevant to `perfo`:

- The recurring design is QML plus shell tooling, while `perfo` keeps data
  collection in the Rust process.
- `nvidia-smi` is an optional external integration, not a vendor-independent
  kernel interface.

References: [README](https://github.com/ierror/menuvitals#readme),
[Model.js](https://github.com/ierror/menuvitals/blob/main/Model.js),
[probe.sh](https://github.com/ierror/menuvitals/blob/main/probe.sh),
[procs.sh](https://github.com/ierror/menuvitals/blob/main/procs.sh).

## Minimal Monitor

Source: [andreireanu/omarchy-minimal-monitor](https://github.com/andreireanu/omarchy-minimal-monitor)

Manifest version: `0.5.2`, id `io.github.andreireanu.minimal-monitor`.

Implementation:

- `scripts/sysread` reads `/proc/stat`, `/proc/meminfo`, and hwmon.
- Fans are enumerated from `fanN_input` and named with `fanN_label`.
- A stopped fan remains visible as `0 RPM`.
- CPU temperature prefers `k10temp`/`zenpower` package labels and Intel
  `coretemp`, then falls back to the hottest sane sensor.
- The known Framework duplicate view is handled by dropping `acpi_fan` when a
  `cros_ec` fan is present.
- `MONITOR_HWMON_ROOT` allows tests to use fake hardware trees.
- The repository documents nine fake hardware shapes in its test workflow.

Useful ideas:

- Generic hwmon enumeration instead of notebook model profiles.
- Keep stopped hardware visible.
- Test hardware shape variations without requiring the hardware.

Limitations relevant to `perfo`:

- The `cros_ec`/`acpi_fan` rule is a known duplicate heuristic, not a Linux
  guarantee for every laptop.
- It monitors fans; it does not solve model-specific fan control.

References: [README](https://github.com/andreireanu/omarchy-minimal-monitor#readme),
[sysread](https://github.com/andreireanu/omarchy-minimal-monitor/blob/main/scripts/sysread),
[Model.js](https://github.com/andreireanu/omarchy-minimal-monitor/blob/main/Model.js).

## System Metrics

Source: [alextakitani/omarchy-sysmetrics](https://github.com/alextakitani/omarchy-sysmetrics)

Manifest version: `1.3.0`, id `takitani.sysmetrics`.

Implementation:

- QML `FileView` and bounded readers sample procfs and sysfs.
- Metric sampling follows visibility and bar configuration.
- The bar CPU gauge uses the busiest logical core, while the popup also shows
  the aggregate average.
- Process lists are gated until their sections are expanded.
- Storage capacity is separated from disk I/O.
- GPU load is shown only when a usable DRM `gpu_busy_percent` source exists.
- Fixed-width display contracts keep bar click targets from moving as values
  change.
- Its contract documents explicit bounds, first-sample behavior, and QML
  repaint dependencies.

Useful ideas:

- Busiest-core headline for responsiveness.
- Live process deltas instead of lifetime `ps %CPU`.
- Bounded input and honest first-sample states.
- Separate storage capacity from I/O activity.

Limitations relevant to `perfo`:

- It is a QML reader architecture and does not replace the Rust engine.
- GPU utilization remains conditional on the driver interface.

References: [README](https://github.com/alextakitani/omarchy-sysmetrics#readme),
[Readers.qml](https://github.com/alextakitani/omarchy-sysmetrics/blob/main/Readers.qml),
[Sampler.qml](https://github.com/alextakitani/omarchy-sysmetrics/blob/main/Sampler.qml),
[contract](https://github.com/alextakitani/omarchy-sysmetrics/blob/main/docs/CONTRACT.md).

## System stats

Source: [harbefas/omarchy-system-stats](https://github.com/harbefas/omarchy-system-stats)

Manifest version: `1.0.0`, id `harbefas.system-stats`.

Implementation:

- `Panel.qml` invokes `bin/stats`.
- `bin/stats` uses Bash, `awk`, `ps`, `df`, procfs, and hwmon.
- It reports CPU, memory, swap, root filesystem, load, uptime, cores, and the
  top five processes.
- Process names are read from `/proc/<pid>/comm`, which is the correct short
  identity for a display label.
- Its process CPU values come from `ps %cpu`, so they are lifetime averages,
  not the live delta used by `perfo`.
- The documented temperature path is Intel `coretemp`; AMD `k10temp` omits
  that temperature display.
- It has no GPU or fan/RPM collector in the reviewed source.

Useful ideas:

- Very small plugin surface.
- Short process names from `/proc/<pid>/comm`.

Limitations relevant to `perfo`:

- Repeated shell commands and `ps` are less suitable for a rich continuous
  data engine.
- Hardware coverage is narrower than the other references.

References: [README](https://github.com/harbefas/omarchy-system-stats#readme),
[bin/stats](https://github.com/harbefas/omarchy-system-stats/blob/master/bin/stats).

## Decisions adopted by perfo

- Use Rust for collection and keep QML as a thin presentation layer.
- Emit a dedicated process `name` field and never derive it from the final `/`
  in a complete command line.
- Show `TOP PROCESSES` explicitly.
- Enumerate fans through hwmon and preserve `0 RPM`.
- Keep unsupported GPU metrics unavailable instead of emitting `0%`.
- Add tests around parsers and hardware-shape behavior.
