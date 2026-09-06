use std::ffi::{c_char, c_uint, c_void, CStr, CString};

use super::gpu::{GpuInfo, GpuProcessInfo};

const NVML_SUCCESS: NvmlReturn = 0;
const NVML_ERROR_NOT_FOUND: NvmlReturn = 6;
const NVML_ERROR_INSUFFICIENT_SIZE: NvmlReturn = 7;
const NVML_VALUE_NOT_AVAILABLE: u64 = u64::MAX;
const NVML_TEMPERATURE_GPU: c_uint = 0;
const DEVICE_NAME_SIZE: usize = 96;

type NvmlReturn = i32;
type NvmlDevice = *mut c_void;
type NvmlInit = unsafe extern "C" fn() -> NvmlReturn;
type NvmlShutdown = unsafe extern "C" fn() -> NvmlReturn;
type NvmlDeviceGetCount = unsafe extern "C" fn(*mut c_uint) -> NvmlReturn;
type NvmlDeviceGetHandleByIndex = unsafe extern "C" fn(c_uint, *mut NvmlDevice) -> NvmlReturn;
type NvmlDeviceGetName = unsafe extern "C" fn(NvmlDevice, *mut c_char, c_uint) -> NvmlReturn;
type NvmlDeviceGetUtilizationRates =
    unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> NvmlReturn;
type NvmlDeviceGetMemoryInfo = unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> NvmlReturn;
type NvmlDeviceGetTemperature = unsafe extern "C" fn(NvmlDevice, c_uint, *mut c_uint) -> NvmlReturn;
type NvmlDeviceGetPowerUsage = unsafe extern "C" fn(NvmlDevice, *mut c_uint) -> NvmlReturn;
type NvmlDeviceGetRunningProcesses =
    unsafe extern "C" fn(NvmlDevice, *mut c_uint, *mut NvmlProcessInfo) -> NvmlReturn;
type NvmlDeviceGetProcessUtilization = unsafe extern "C" fn(
    NvmlDevice,
    *mut NvmlProcessUtilizationSample,
    *mut c_uint,
    u64,
) -> NvmlReturn;

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NvmlProcessInfo {
    pid: c_uint,
    used_gpu_memory: u64,
    gpu_instance_id: c_uint,
    compute_instance_id: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NvmlProcessUtilizationSample {
    pid: c_uint,
    time_stamp: u64,
    sm_util: c_uint,
    mem_util: c_uint,
    enc_util: c_uint,
    dec_util: c_uint,
}

struct NvmlApi {
    library: *mut c_void,
    initialized: bool,
    init: NvmlInit,
    shutdown: NvmlShutdown,
    device_get_count: NvmlDeviceGetCount,
    device_get_handle_by_index: NvmlDeviceGetHandleByIndex,
    device_get_name: NvmlDeviceGetName,
    device_get_utilization_rates: NvmlDeviceGetUtilizationRates,
    device_get_memory_info: NvmlDeviceGetMemoryInfo,
    device_get_temperature: Option<NvmlDeviceGetTemperature>,
    device_get_power_usage: Option<NvmlDeviceGetPowerUsage>,
    device_get_compute_processes: Option<NvmlDeviceGetRunningProcesses>,
    device_get_graphics_processes: Option<NvmlDeviceGetRunningProcesses>,
    device_get_process_utilization: Option<NvmlDeviceGetProcessUtilization>,
}

impl NvmlApi {
    fn load() -> Option<Self> {
        let library = load_library()?;
        let Some(mut api) = (unsafe { Self::from_library(library) }) else {
            unsafe { libc::dlclose(library) };
            return None;
        };
        api.initialized = unsafe { (api.init)() } == NVML_SUCCESS;
        if !api.initialized {
            return None;
        }
        Some(api)
    }

    unsafe fn from_library(library: *mut c_void) -> Option<Self> {
        Some(Self {
            library,
            initialized: false,
            init: load_symbol(library, "nvmlInit_v2")
                .or_else(|| load_symbol(library, "nvmlInit"))?,
            shutdown: load_symbol(library, "nvmlShutdown")?,
            device_get_count: load_symbol(library, "nvmlDeviceGetCount_v2")
                .or_else(|| load_symbol(library, "nvmlDeviceGetCount"))?,
            device_get_handle_by_index: load_symbol(library, "nvmlDeviceGetHandleByIndex_v2")
                .or_else(|| load_symbol(library, "nvmlDeviceGetHandleByIndex"))?,
            device_get_name: load_symbol(library, "nvmlDeviceGetName")?,
            device_get_utilization_rates: load_symbol(library, "nvmlDeviceGetUtilizationRates")?,
            device_get_memory_info: load_symbol(library, "nvmlDeviceGetMemoryInfo")?,
            device_get_temperature: load_symbol(library, "nvmlDeviceGetTemperature"),
            device_get_power_usage: load_symbol(library, "nvmlDeviceGetPowerUsage"),
            device_get_compute_processes: load_symbol(
                library,
                "nvmlDeviceGetComputeRunningProcesses_v3",
            )
            .or_else(|| load_symbol(library, "nvmlDeviceGetComputeRunningProcesses_v2")),
            device_get_graphics_processes: load_symbol(
                library,
                "nvmlDeviceGetGraphicsRunningProcesses_v3",
            )
            .or_else(|| load_symbol(library, "nvmlDeviceGetGraphicsRunningProcesses_v2")),
            device_get_process_utilization: load_symbol(library, "nvmlDeviceGetProcessUtilization"),
        })
    }

    fn device_count(&self) -> Option<c_uint> {
        let mut count = 0;
        let result = unsafe { (self.device_get_count)(&mut count) };
        (result == NVML_SUCCESS).then_some(count)
    }

    fn device_handle(&self, index: c_uint) -> Option<NvmlDevice> {
        let mut device = std::ptr::null_mut();
        let result = unsafe { (self.device_get_handle_by_index)(index, &mut device) };
        (result == NVML_SUCCESS && !device.is_null()).then_some(device)
    }

    fn device_name(&self, device: NvmlDevice) -> String {
        let mut buffer = [0_u8; DEVICE_NAME_SIZE];
        let result = unsafe {
            (self.device_get_name)(device, buffer.as_mut_ptr().cast(), buffer.len() as c_uint)
        };
        if result != NVML_SUCCESS {
            return "NVIDIA GPU".into();
        }
        device_name_from_buffer(&buffer)
    }
}

impl Drop for NvmlApi {
    fn drop(&mut self) {
        unsafe {
            if self.initialized {
                (self.shutdown)();
            }
            libc::dlclose(self.library);
        }
    }
}

struct NvidiaDevice {
    handle: NvmlDevice,
    name: String,
    usage_percent: Option<f32>,
    memory_used_bytes: Option<u64>,
    memory_total_bytes: Option<u64>,
    temperature_c: Option<f32>,
    power_w: Option<f32>,
    processes: Vec<GpuProcessInfo>,
    last_process_timestamp: u64,
}

pub(crate) struct NvidiaBackend {
    api: NvmlApi,
    devices: Vec<NvidiaDevice>,
}

impl NvidiaBackend {
    pub(crate) fn discover() -> Option<Self> {
        let api = NvmlApi::load()?;
        let count = api.device_count()?;
        let devices = (0..count)
            .filter_map(|index| api.device_handle(index))
            .map(|handle| NvidiaDevice {
                name: api.device_name(handle),
                handle,
                usage_percent: None,
                memory_used_bytes: None,
                memory_total_bytes: None,
                temperature_c: None,
                power_w: None,
                processes: Vec::new(),
                last_process_timestamp: 0,
            })
            .collect::<Vec<_>>();
        (!devices.is_empty()).then_some(Self { api, devices })
    }

    pub(crate) fn refresh(&mut self) {
        for device in &mut self.devices {
            refresh_device(&self.api, device);
        }
    }

    pub(crate) fn snapshots(&self) -> Vec<GpuInfo> {
        self.devices.iter().map(snapshot_device).collect()
    }
}

fn refresh_device(api: &NvmlApi, device: &mut NvidiaDevice) {
    let mut utilization = NvmlUtilization::default();
    device.usage_percent = nvml_percent(
        unsafe { (api.device_get_utilization_rates)(device.handle, &mut utilization) },
        utilization.gpu as f32,
    );

    let mut memory = NvmlMemory::default();
    if unsafe { (api.device_get_memory_info)(device.handle, &mut memory) } == NVML_SUCCESS {
        device.memory_used_bytes = Some(memory.used);
        device.memory_total_bytes = Some(memory.total);
    } else {
        device.memory_used_bytes = None;
        device.memory_total_bytes = None;
    }

    device.temperature_c = api.device_get_temperature.and_then(|get_temperature| {
        let mut temperature = 0;
        let result =
            unsafe { get_temperature(device.handle, NVML_TEMPERATURE_GPU, &mut temperature) };
        nvml_value(result, temperature as f32)
    });
    device.power_w = api.device_get_power_usage.and_then(|get_power_usage| {
        let mut power_mw = 0;
        let result = unsafe { get_power_usage(device.handle, &mut power_mw) };
        nvml_value(result, power_watts(power_mw))
    });
    device.processes = process_snapshots(api, device);
}

fn process_snapshots(api: &NvmlApi, device: &mut NvidiaDevice) -> Vec<GpuProcessInfo> {
    let mut processes = running_process_memory(api, device.handle);
    for sample in process_utilization(api, device) {
        let entry = processes.entry(sample.pid).or_insert(GpuProcessInfo {
            pid: sample.pid,
            gpu_percent: None,
            memory_used_bytes: None,
        });
        let usage = process_gpu_percent(sample);
        entry.gpu_percent = Some(entry.gpu_percent.unwrap_or_default().max(usage));
    }
    let mut snapshots: Vec<GpuProcessInfo> = processes.into_values().collect();
    snapshots.sort_by(|a, b| {
        b.gpu_percent
            .unwrap_or_default()
            .total_cmp(&a.gpu_percent.unwrap_or_default())
    });
    snapshots
}

fn process_gpu_percent(sample: NvmlProcessUtilizationSample) -> f32 {
    [
        sample.sm_util,
        sample.mem_util,
        sample.enc_util,
        sample.dec_util,
    ]
    .into_iter()
    .max()
    .unwrap_or_default()
    .min(100) as f32
}

fn running_process_memory(
    api: &NvmlApi,
    device: NvmlDevice,
) -> std::collections::HashMap<u32, GpuProcessInfo> {
    let mut processes = std::collections::HashMap::new();
    for query in [
        api.device_get_compute_processes,
        api.device_get_graphics_processes,
    ] {
        for process in running_processes(query, device) {
            let entry = processes.entry(process.pid).or_insert(GpuProcessInfo {
                pid: process.pid,
                gpu_percent: None,
                memory_used_bytes: None,
            });
            if process.used_gpu_memory != NVML_VALUE_NOT_AVAILABLE {
                entry.memory_used_bytes = Some(
                    entry
                        .memory_used_bytes
                        .unwrap_or_default()
                        .saturating_add(process.used_gpu_memory),
                );
            }
        }
    }
    processes
}

fn running_processes(
    query: Option<NvmlDeviceGetRunningProcesses>,
    device: NvmlDevice,
) -> Vec<NvmlProcessInfo> {
    let Some(query) = query else {
        return Vec::new();
    };
    let mut count = 0;
    let result = unsafe { query(device, &mut count, std::ptr::null_mut()) };
    if result != NVML_SUCCESS && result != NVML_ERROR_INSUFFICIENT_SIZE {
        return Vec::new();
    }
    if count == 0 {
        return Vec::new();
    }
    let mut processes = vec![NvmlProcessInfo::default(); count as usize];
    let result = unsafe { query(device, &mut count, processes.as_mut_ptr()) };
    if result != NVML_SUCCESS {
        return Vec::new();
    }
    processes.truncate(count as usize);
    processes
}

fn process_utilization(
    api: &NvmlApi,
    device: &mut NvidiaDevice,
) -> Vec<NvmlProcessUtilizationSample> {
    let Some(query) = api.device_get_process_utilization else {
        return Vec::new();
    };
    let mut count = 0;
    let result = unsafe {
        query(
            device.handle,
            std::ptr::null_mut(),
            &mut count,
            device.last_process_timestamp,
        )
    };
    if result == NVML_ERROR_NOT_FOUND || count == 0 {
        return Vec::new();
    }
    if result != NVML_SUCCESS && result != NVML_ERROR_INSUFFICIENT_SIZE {
        return Vec::new();
    }
    let mut samples = vec![NvmlProcessUtilizationSample::default(); count as usize];
    let result = unsafe {
        query(
            device.handle,
            samples.as_mut_ptr(),
            &mut count,
            device.last_process_timestamp,
        )
    };
    if result != NVML_SUCCESS {
        return Vec::new();
    }
    samples.truncate(count as usize);
    if let Some(timestamp) = samples.iter().map(|sample| sample.time_stamp).max() {
        device.last_process_timestamp = timestamp;
    }
    samples
}

fn snapshot_device(device: &NvidiaDevice) -> GpuInfo {
    GpuInfo {
        name: device.name.clone(),
        vendor: "NVIDIA".into(),
        usage_percent: device.usage_percent,
        memory_used_bytes: device.memory_used_bytes,
        memory_total_bytes: device.memory_total_bytes,
        temperature_c: device.temperature_c,
        power_w: device.power_w,
        processes: device.processes.clone(),
    }
}

fn nvml_value<T>(result: NvmlReturn, value: T) -> Option<T> {
    (result == NVML_SUCCESS).then_some(value)
}

fn nvml_percent(result: NvmlReturn, value: f32) -> Option<f32> {
    nvml_value(result, value).map(|value| value.clamp(0.0, 100.0))
}

fn power_watts(power_mw: c_uint) -> f32 {
    power_mw as f32 / 1_000.0
}

fn device_name_from_buffer(buffer: &[u8]) -> String {
    CStr::from_bytes_until_nul(buffer)
        .ok()
        .and_then(|name| name.to_str().ok())
        .filter(|name| !name.is_empty())
        .unwrap_or("NVIDIA GPU")
        .into()
}

fn load_library() -> Option<*mut c_void> {
    ["libnvidia-ml.so.1", "libnvidia-ml.so"]
        .iter()
        .filter_map(|name| CString::new(*name).ok())
        .find_map(|name| {
            let library = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };
            (!library.is_null()).then_some(library)
        })
}

unsafe fn load_symbol<T: Copy>(library: *mut c_void, name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let symbol = libc::dlsym(library, name.as_ptr());
    (!symbol.is_null()).then(|| std::mem::transmute_copy(&symbol))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvml_values_only_accept_success() {
        assert_eq!(nvml_value(NVML_SUCCESS, 42), Some(42));
        assert_eq!(nvml_value(3, 42), None);
    }

    #[test]
    fn nvml_percentages_are_clamped() {
        assert_eq!(nvml_percent(NVML_SUCCESS, 55.5), Some(55.5));
        assert_eq!(nvml_percent(NVML_SUCCESS, 150.0), Some(100.0));
        assert_eq!(nvml_percent(3, 55.5), None);
    }

    #[test]
    fn power_is_converted_from_milliwatts() {
        assert_eq!(power_watts(75_500), 75.5);
    }

    #[test]
    fn process_gpu_percent_uses_the_busy_engine() {
        let sample = NvmlProcessUtilizationSample {
            sm_util: 12,
            mem_util: 64,
            enc_util: 4,
            dec_util: 2,
            ..Default::default()
        };
        assert_eq!(process_gpu_percent(sample), 64.0);
    }

    #[test]
    fn device_names_are_read_from_nul_terminated_buffer() {
        let buffer = *b"GeForce RTX 4090\0unused-padding";
        assert_eq!(device_name_from_buffer(&buffer), "GeForce RTX 4090");
    }

    #[test]
    fn missing_or_invalid_device_names_use_a_bounded_fallback() {
        assert_eq!(device_name_from_buffer(b""), "NVIDIA GPU");
        assert_eq!(device_name_from_buffer(b"not-terminated"), "NVIDIA GPU");
    }
}
