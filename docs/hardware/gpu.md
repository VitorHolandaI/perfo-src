# GPU Monitoring Sources

## Core rule

GPU utilization is not one universal Linux sysfs field. The value must be
reported only when the selected backend provides a meaningful counter.
Missing support is represented as `null` in JSON, not `0`.

## AMD

The reviewed Omarchy references and the AMDGPU Linux interface use DRM device
paths such as `/sys/class/drm/cardN/device/`. The useful read-only fields are:

```text
gpu_busy_percent
mem_info_vram_used
mem_info_vram_total
```

The GPU can also expose hwmon values under its device tree, including
`temp*_input`, `power*_average`, and frequency values. These are optional and
must be discovered rather than assumed.

`perfo` identifies AMD DRM cards by `device/vendor == 0x1002` and reads the
available AMD fields.

## Intel

Intel i915 and Xe do not guarantee an AMD-style device-wide
`gpu_busy_percent` file. The DRM usage-stats interface exposes per-client
engine time in `/proc/<pid>/fdinfo/<fd>`, for example:

```text
drm-driver: i915
drm-client-id: 9
drm-pdev: 0000:00:02.0
drm-engine-render: 375132209868 ns
```

The current implementation discovers Intel i915 cards through DRM sysfs,
reads these fdinfo records, and deduplicates duplicated file descriptors by
`drm-client-id`. It computes the delta between samples and reports the busiest
engine as the device headline. This avoids summing parallel engines into an
arbitrary value above 100% and does not require `perf_event_open` or a relaxed
`kernel.perf_event_paranoid` setting.

The same samples are retained by PID for the GPU process table. Intel integrated
GPUs use shared system RAM, so per-process dedicated VRAM is unavailable and is
shown as `--`; the process `RAM%` column still reports its system-memory share.

The same engine-time model is suitable for Xe, but the current discovery path
still accepts only the i915 driver. Memory, temperature, and power remain
optional because their DRM and hwmon surfaces differ by device.

## NVIDIA

NVIDIA does not provide a portable equivalent sysfs utilization field across
the supported driver stack. The MenuVitals reference uses optional
`nvidia-smi`; NVML is the more direct library API for utilization, memory,
temperature, power, and process data.

`perfo` loads NVML dynamically from `libnvidia-ml.so.1` (falling back to
`libnvidia-ml.so`) and does not link it at build time. The backend is enabled
only when NVML initializes and reports at least one device, so AMD/Intel users
do not need an NVIDIA library or command installed. Utilization, VRAM,
temperature, and power are queried through the optional NVML entry points; a
metric that the driver does not expose remains `null`.

## Identity and multiple GPUs

DRM card numbers can change between boots, and connector entries also exist in
`/sys/class/drm`. Discovery must accept only names matching `card` followed by
digits and then inspect the device vendor/driver.

A machine can expose both an integrated and a discrete GPU. The JSON model is
therefore an array of devices rather than one global GPU scalar.

## JSON contract

The snapshot contains:

```json
{
  "gpu": {
    "devices": [
      {
        "name": "Intel GPU",
        "vendor": "Intel",
        "usage_percent": null,
        "memory_used_bytes": null,
        "memory_total_bytes": null,
        "temperature_c": null,
        "power_w": null,
        "processes": [
          {
            "pid": 1234,
            "gpu_percent": 12.5,
            "memory_used_bytes": null
          }
        ]
      }
    ]
  }
}
```

Every metric is optional because hardware and driver surfaces differ.
