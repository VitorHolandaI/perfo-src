# Intel NPU Monitoring Sources

Perfo reads Intel NPU metrics directly from the Linux `intel_vpu` driver. It
does not execute `intel-npu-smi`, access Intel PMT telemetry, or require root.

## Discovery

The collector enumerates `/sys/class/accel/accel*`, but never treats the
`accelN` number as stable identity. A device is accepted only when its sysfs
data reports:

- PCI vendor `0x8086` (Intel).
- `DRIVER=intel_vpu` in the device `uevent`.
- A `PCI_SLOT_NAME` used as the stable display identity.

Unsupported accelerators are ignored and an absent NPU is represented by an
empty `npu.devices` array.

## Utilization

`npu_busy_time_us` is a cumulative microsecond counter for time with at least
one job submitted to the NPU firmware. Perfo samples it with the same monotonic
clock used by the rest of the monitor, at most once per second as recommended
by the kernel ABI:

```text
utilization_percent = 100 * delta_busy_us / delta_elapsed_us
```

The first sample and a counter reset are reported as `null`, not `0%`. Valid
results are bounded to `0..100%`.

## Other metrics

- `npu_current_frequency_mhz`: current frequency; `0` is valid while idle or suspended.
- `npu_max_frequency_mhz`: hardware maximum frequency.
- `npu_memory_utilization`: allocated NPU memory in bytes.

These sysfs files are readable without root on the supported driver. Raw Intel
PMT telemetry is a separate interface and is not used, so Perfo does not claim
NPU power or temperature. The current device-wide busy counter also cannot
attribute utilization to individual processes, so no per-process NPU load is
fabricated.

## Kernel references

- `https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-driver-ivpu`
- `https://github.com/torvalds/linux/blob/master/drivers/accel/ivpu/ivpu_sysfs.c`
