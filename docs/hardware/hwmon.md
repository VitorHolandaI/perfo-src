# Linux hwmon and Cooler Readings

## Canonical interface

The Linux hwmon sysfs contract is documented by the kernel in [Naming and data
format standards for sysfs files](https://docs.kernel.org/hwmon/sysfs-interface.html).
The Arch reference is [lm_sensors](https://wiki.archlinux.org/title/Lm_sensors).

Applications can discover chips through `/sys/class/hwmon/hwmon*`. Each chip
directory has a `name` file. Standard fan channels use:

```text
fan1_input       measured RPM
fan1_label       suggested display label
fan1_min         minimum threshold, if exposed
fan1_target      requested target, if exposed
fan1_fault       fault flag, if exposed
```

All attributes are optional. A monitor must only render a channel whose input
file exists and contains a valid value.

## Why there are no notebook profiles

For read-only monitoring, the kernel driver has already selected the hardware
interface. The monitor should discover available hwmon channels instead of
hard-coding ASUS, Lenovo, Dell, Framework, or another model.

The Arch [Fan speed control](https://wiki.archlinux.org/title/Fan_speed_control)
page documents why this differs from fan control:

- A laptop can use one fan for both CPU and GPU.
- A laptop can have separate CPU and GPU fans.
- Some fans are exposed through model-specific interfaces.
- Some hardware has no usable kernel support.
- Writing PWM or embedded-controller registers is model-specific and risky.

`perfo` only reads RPM. It does not write `pwm*`, `pwm*_enable`, EC
registers, or fan curves.

## Stable discovery

`hwmonN` is an enumeration index, not a persistent identity. The ArchWiki
notes that its order can change after reboot or module probe order changes.
Therefore:

- Discover channels on each snapshot or resolve them by stable device identity.
- Do not persist a bare `hwmonN` path in user configuration.
- Keep the chip name and channel index as display identity where no stronger
  identity is available.
- Treat labels as untrusted kernel-provided text and bound their length.

`perfo` currently re-enumerates `/sys/class/hwmon` for each snapshot. It reads
`fanN_input`, reads `fanN_label` when present, falls back to `Fan N`, and keeps
zero RPM readings visible.

## Duplicate views

One physical fan can appear through more than one driver. The Minimal Monitor
reference handles a known Framework case by suppressing `acpi_fan` when a
`cros_ec` fan exists. That is a heuristic, not a general deduplication proof.

`perfo` applies two narrow preferences based on observed duplicate views:
`cros_ec` replaces `acpi_fan`, and `acpi_fan` replaces `asus` when both are
present. This is not a general topology proof; it is intended to avoid an
unstable ASUS/WMI reading being counted as a second physical fan. Other chips
remain visible, and future hardware reports should add a targeted test before
expanding this heuristic.

## Temperature selection

ArchWiki documents that sensor numbering does not establish physical meaning:
`temp2` is not automatically the CPU temperature. Selection should prefer a
driver and label that identify the package, then use a bounded fallback.

The existing CPU collector prefers package/core labels exposed by sysinfo. A
future hwmon detail view can expose the source chip and label rather than
pretending every temperature has the same meaning.

## Missing support

ArchWiki lists examples where extra kernel modules are needed, including
`nct6775`, `nct6683`, `asus-wmi-sensors`, `thinkpad_acpi`, and
`dell_smm_hwmon`. `perfo` must not load these modules or ask for elevated
privileges. If a module is absent, the correct result is an unavailable fan
or temperature reading.

## Test plan

The reference fixture approach should cover at least:

- No hwmon directory.
- Fanless machine.
- One fan with no label.
- Multiple fans with labels.
- A stopped fan reporting `0`.
- A malformed or negative input.
- `cros_ec` plus duplicate `acpi_fan`.
- A label containing control characters or excessive length.

The current Rust unit tests cover parser and deduplication rules. Fake-tree
integration fixtures remain a follow-up.
