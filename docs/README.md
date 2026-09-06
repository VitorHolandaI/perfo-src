# Perfo Documentation

This directory records the Linux interfaces, Omarchy plugin references, and
implementation decisions used by `perfo`.

## Contents

- [Research sources](research/omarchy-plugins.md)
- [Hardware monitoring](hardware/hwmon.md)
- [GPU monitoring](hardware/gpu.md)
- [Process metrics](runtime/processes.md)
- [UI pages and formulas](ui-pages.md)
- [Implementation decisions](architecture.md)

## Scope

`perfo` is a read-only system monitor. It reads Linux procfs, sysfs, hwmon,
and kernel counters. It does not control fans, write EC registers, change PWM
values, load kernel modules, install daemons, or require `lm_sensors`.

This release is not zero-dependency. It keeps a small set of Rust crates for
terminal rendering/input, system enumeration, JSON serialization, and Linux
FFI. Reducing that set is a future release goal, not a requirement for the
current plugin or terminal binary.

Values that are unavailable on a machine are represented as unavailable. The
monitor must not turn an unsupported sensor into a fabricated zero.

## Verification date

External references were reviewed on 2026-09-04. Upstream repositories can
change after that date; links identify the source, not a vendored dependency.
