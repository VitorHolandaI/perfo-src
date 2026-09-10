//! Integration test through the library's public API (the way an external
//! consumer or the Omarchy widget would use it).

use perfo::data::cpu::{CollectionProfile, CpuMonitor};
use perfo::data::summary::WidgetSummaryMonitor;

#[test]
fn snapshot_has_core_sections() {
    let mut m = CpuMonitor::new();
    perfo::data::cpu::wait_sample_interval();
    m.refresh();
    let snap = m.snapshot();

    assert!(snap.core_count > 0, "no CPUs detected");
    assert_eq!(snap.per_core.len(), snap.core_count);
    assert!(!snap.processes.is_empty(), "no processes in snapshot");
    assert!(snap
        .processes
        .iter()
        .all(|process| !process.name.is_empty()));
    assert!(!snap.disks.is_empty(), "no disks in snapshot");
    assert!(!snap.net.ifaces.is_empty(), "no network interfaces");
}

#[test]
fn snapshot_serializes_to_json() {
    let mut m = CpuMonitor::new();
    perfo::data::cpu::wait_sample_interval();
    m.refresh();
    let snap = m.snapshot();

    let json = serde_json::to_value(&snap).expect("snapshot must serialize");
    assert!(json["net"]["totals"].is_object());
    assert!(json["disks"].is_array());
    assert!(json["per_core"].is_array());
    assert!(json["processes"].is_array());
    assert!(json["cpu_history"].is_array());
    assert!(json["net"]["rx_history"].is_array());
    assert!(json["net"]["tx_history"].is_array());
    assert!(json["gpu"]["devices"].is_array());
    assert!(json["npu"]["devices"].is_array());
    assert!(json["fans"]["fans"].is_array());
}

#[test]
fn widget_summary_contains_only_bar_metrics() {
    let mut monitor = WidgetSummaryMonitor::new();
    perfo::data::cpu::wait_sample_interval();
    monitor.refresh();

    let json = serde_json::to_value(monitor.snapshot()).expect("summary must serialize");
    let keys = json.as_object().expect("summary must be an object").keys();
    assert_eq!(
        keys.cloned().collect::<std::collections::BTreeSet<_>>(),
        [
            "gpu".to_string(),
            "npu".to_string(),
            "overall_percent".to_string(),
            "total_mem_bytes".to_string(),
            "used_mem_bytes".to_string(),
        ]
        .into_iter()
        .collect()
    );
    assert!(json["gpu"]["devices"].is_array());
    assert!(json["npu"]["devices"].is_array());
}

#[test]
fn dashboard_snapshot_omits_hidden_provider_details() {
    let mut monitor = CpuMonitor::new_for(CollectionProfile::Dashboard);
    perfo::data::cpu::wait_sample_interval();
    monitor.refresh_for(CollectionProfile::Dashboard, true);
    let snapshot = monitor.snapshot_for(CollectionProfile::Dashboard);

    assert!(snapshot.processes.is_empty());
    assert!(snapshot.per_core.is_empty());
    assert!(snapshot.net.proc_net.is_empty());
    assert!(snapshot.net.listening.is_empty());
    assert!(snapshot.disks.iter().all(|disk| disk.temp_c.is_none()));
    assert!(snapshot
        .gpu
        .devices
        .iter()
        .all(|device| device.processes.is_empty()));
    assert_eq!(snapshot.mem.psi_some_10, 0.0);
}

#[test]
fn cpu_snapshot_omits_non_cpu_providers() {
    let mut monitor = CpuMonitor::new_for(CollectionProfile::Cpu);
    perfo::data::cpu::wait_sample_interval();
    monitor.refresh_for(CollectionProfile::Cpu, true);
    let snapshot = monitor.snapshot_for(CollectionProfile::Cpu);

    assert!(!snapshot.per_core.is_empty());
    assert!(snapshot.disks.is_empty());
    assert!(snapshot.net.ifaces.is_empty());
    assert!(snapshot.gpu.devices.is_empty());
    assert!(snapshot.npu.devices.is_empty());
    assert_eq!(snapshot.total_mem_bytes, 0);
}
