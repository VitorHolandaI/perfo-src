//! Rendering a recording as a text or JSON report.

use super::format::{clean_process_name, format_local_datetime};
use super::tables::{format_proc_conns, format_proc_net_io};
use super::HistorySample;
use super::{HistoryMetric, HistoryProcess, HistoryState};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;

impl HistoryState {
    pub fn export(&mut self) {
        if self.samples.is_empty() {
            self.export_status = Some(("No history to export".to_string(), Instant::now()));
            return;
        }

        let now = std::time::SystemTime::now();
        let date_str = format_local_datetime(now);
        let base_name = format!("perfo-history-{}", date_str);

        let home_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        let txt_path = home_dir.join(format!("{}.txt", base_name));
        let json_path = home_dir.join(format!("{}.json", base_name));

        let text_content = self.generate_export_text(&date_str);
        let json_content = self.generate_export_json(&date_str);

        let res_txt = fs::write(&txt_path, text_content);
        let res_json = fs::write(&json_path, json_content);

        if res_txt.is_ok() && res_json.is_ok() {
            self.export_status = Some((
                format!("Saved: ~/{}.{{txt,json}}", base_name),
                Instant::now(),
            ));
        } else {
            self.export_status = Some(("Export failed".to_string(), Instant::now()));
        }
    }

    /// The banner and the recording's identity at the top of the report.
    fn export_header(&self, date_str: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let border =
            "================================================================================";
        lines.push(border.to_string());
        lines.push("                   PERFO - SYSTEM HISTORY & ANALYSIS REPORT".to_string());
        lines.push(border.to_string());
        lines.push(format!("Generated at         : {}", date_str));
        lines.push(format!("Active Metric Focus  : {}", self.metric.label()));
        lines.push(format!(
            "Timeline View Span   : {} ({}s)",
            self.span.label(),
            self.span.seconds(self.sample_count())
        ));
        lines.push(format!(
            "Total Recorded Time  : {}s ({} samples)",
            self.sample_count(),
            self.sample_count()
        ));
        lines.push(format!(
            "Current View State   : {}",
            if self.is_live() { "LIVE" } else { "SCRUBBED" }
        ));
        lines.push(String::new());

        lines
    }

    /// The sample the cursor sits on, as the report's "at this moment" section.
    fn export_cursor_sample(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let border =
            "================================================================================";
        let sub_border =
            "--------------------------------------------------------------------------------";
        let eff = self.effective_index();
        let sample = self.get_sample(eff);
        lines.push(border.to_string());
        lines.push(format!(
            "                   1. SNAPSHOT AT SELECTED TIMING ({})",
            sample.map(|s| s.timestamp.as_str()).unwrap_or("--")
        ));
        lines.push(border.to_string());

        if let Some(s) = sample {
            lines.push(format!("Overall CPU Usage    : {:.1}%", s.cpu));
            lines.push(format!("Overall Memory Usage : {:.1}%", s.mem));
            lines.push(format!("Total Disk I/O Rate  : {:.1} MB/s", s.io_mb));
            lines.push(format!(
                "Total Network Rate   : {} (RX: {}, TX: {})",
                crate::tui::cpu::human_bytes(s.net_rx_bps + s.net_tx_bps),
                crate::tui::cpu::human_bytes(s.net_rx_bps),
                crate::tui::cpu::human_bytes(s.net_tx_bps)
            ));
            lines.push(format!("GPU Usage            : {:.1}%", s.gpu));
            lines.push(String::new());
            lines.push("Active Processes at this timing:".to_string());
            if self.metric == HistoryMetric::Net {
                lines.push(format!(
                    "{:<8} | {:<16} | {:>12} | {:>12} | {:<22} | COMMAND",
                    "PID", "PROCESS", "IN (RX)", "OUT (TX)", "TOTAL / CONNS"
                ));
                lines.push(sub_border.to_string());

                let mut procs = s.top_procs.clone();
                self.sort_processes(&mut procs);
                for p in procs.iter().take(15) {
                    let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
                    let rx_str = format_proc_net_io(p.net_rx_bps, p.net_rx_bytes);
                    let tx_str = format_proc_net_io(p.net_tx_bps, p.net_tx_bytes);
                    let conns_str = format_proc_conns(p);
                    lines.push(format!(
                        "{:<8} | {:<16} | {:>12} | {:>12} | {:<22} | {}",
                        p.pid, p_name, rx_str, tx_str, conns_str, p.cmd
                    ));
                }
            } else {
                lines.push(format!(
                    "{:<8} | {:>8} | {:>10} | {:<18} | COMMAND",
                    "PID",
                    self.metric.label(),
                    if self.metric == HistoryMetric::Gpu {
                        "VRAM"
                    } else {
                        "RAM"
                    },
                    "PROCESS"
                ));
                lines.push(sub_border.to_string());

                let mut procs = s.top_procs.clone();
                self.sort_processes(&mut procs);
                for p in procs.iter().take(15) {
                    let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
                    let primary = self.metric_cell_text(p);
                    let secondary = if self.metric == HistoryMetric::Gpu {
                        crate::tui::cpu::human_bytes(p.vram_bytes)
                    } else {
                        crate::tui::cpu::human_bytes(p.mem_bytes)
                    };
                    lines.push(format!(
                        "{:<8} | {:>8} | {:>10} | {:<18} | {}",
                        p.pid, primary, secondary, p_name, p.cmd
                    ));
                }
            }
        }
        lines.push(String::new());

        lines
    }

    /// Peaks and averages across the whole recording, plus the process table.
    fn export_summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let border =
            "================================================================================";
        // Peaks & summary
        lines.push(border.to_string());
        lines.push("                   2. TIMELINE METRICS & PEAKS SUMMARY".to_string());
        lines.push(border.to_string());

        let mut peak_cpu = 0.0f32;
        let mut peak_mem = 0.0f32;
        let mut peak_io = 0.0f32;
        let mut peak_net = 0u64;
        let mut peak_gpu = 0.0f32;
        let mut total_cpu = 0.0f64;
        let mut total_mem = 0.0f64;

        let count = self.sample_count();
        for idx in 0..count {
            if let Some(s) = self.get_sample(idx) {
                total_cpu += s.cpu as f64;
                total_mem += s.mem as f64;
                if s.cpu > peak_cpu {
                    peak_cpu = s.cpu;
                }
                if s.mem > peak_mem {
                    peak_mem = s.mem;
                }
                if s.io_mb > peak_io {
                    peak_io = s.io_mb;
                }
                let net_tot = s.net_rx_bps + s.net_tx_bps;
                if net_tot > peak_net {
                    peak_net = net_tot;
                }
                if s.gpu > peak_gpu {
                    peak_gpu = s.gpu;
                }
            }
        }

        let cnt_f64 = count.max(1) as f64;
        lines.push(format!("Peak CPU Usage       : {:.1}%", peak_cpu));
        lines.push(format!("Peak Memory Usage    : {:.1}%", peak_mem));
        lines.push(format!("Peak Disk I/O Rate   : {:.1} MB/s", peak_io));
        lines.push(format!(
            "Peak Network Rate    : {}/s",
            crate::tui::cpu::human_bytes(peak_net)
        ));
        lines.push(format!("Peak GPU Usage       : {:.1}%", peak_gpu));
        lines.push(format!(
            "Average CPU Usage    : {:.1}%",
            total_cpu / cnt_f64
        ));
        lines.push(format!(
            "Average Memory Usage : {:.1}%",
            total_mem / cnt_f64
        ));
        lines.push(String::new());

        lines.push(border.to_string());
        lines.push("End of Perfo History Report".to_string());
        lines.push(String::new());
        lines
    }

    fn generate_export_text(&self, date_str: &str) -> String {
        let mut lines = self.export_header(date_str);
        lines.extend(self.export_cursor_sample());
        lines.extend(self.export_summary());
        lines.join("\n")
    }

    fn generate_export_json(&self, date_str: &str) -> String {
        #[derive(Serialize)]
        struct ExportJson<'a> {
            version: &'static str,
            generator: &'static str,
            generated_at: &'a str,
            metric_focus: &'static str,
            zoom_span: &'static str,
            total_samples: usize,
            is_live: bool,
            selected_sample: Option<&'a HistorySample>,
            samples: Vec<&'a HistorySample>,
        }

        let eff = self.effective_index();
        let selected = self.get_sample(eff);
        let count = self.sample_count();
        let step = (count / 1000).max(1);
        let mut sampled: Vec<&HistorySample> = Vec::new();
        for idx in (0..count).step_by(step) {
            if let Some(s) = self.get_sample(idx) {
                sampled.push(s);
            }
        }

        let export = ExportJson {
            version: "1.0",
            generator: "perfo",
            generated_at: date_str,
            metric_focus: self.metric.label(),
            zoom_span: self.span.label(),
            total_samples: count,
            is_live: self.is_live(),
            selected_sample: selected,
            samples: sampled,
        };

        serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".to_string())
    }

    pub(super) fn sort_processes(&self, list: &mut Vec<HistoryProcess>) {
        match self.metric {
            HistoryMetric::Gpu => {
                list.retain(|p| p.gpu_percent > 0.0 || p.vram_bytes > 0);
                list.sort_by(|a, b| {
                    b.gpu_percent
                        .partial_cmp(&a.gpu_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.vram_bytes.cmp(&a.vram_bytes))
                });
            }
            HistoryMetric::Io => {
                list.sort_by_key(|p| std::cmp::Reverse(p.read_bps + p.write_bps));
            }
            HistoryMetric::Mem => {
                list.sort_by_key(|p| std::cmp::Reverse(p.mem_bytes));
            }
            HistoryMetric::Cpu => {
                list.sort_by(|a, b| {
                    b.cpu_percent
                        .partial_cmp(&a.cpu_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            HistoryMetric::Net => {
                list.retain(|p| {
                    p.net_rx_bps > 0
                        || p.net_tx_bps > 0
                        || p.net_rx_bytes > 0
                        || p.net_tx_bytes > 0
                        || p.tcp_est > 0
                        || p.udp > 0
                });
                list.sort_by_key(|p| {
                    std::cmp::Reverse(
                        (p.net_rx_bps + p.net_tx_bps) * 1000
                            + (p.net_rx_bytes + p.net_tx_bytes)
                            + (p.tcp_est as u64 * 100)
                            + p.udp as u64,
                    )
                });
            }
        }
    }

    pub(super) fn metric_cell_text(&self, proc: &HistoryProcess) -> String {
        match self.metric {
            HistoryMetric::Mem => crate::tui::cpu::human_bytes(proc.mem_bytes),
            HistoryMetric::Gpu => {
                if proc.gpu_percent > 0.0 {
                    format!("{:.0}%", proc.gpu_percent)
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Io => {
                let total = proc.read_bps + proc.write_bps;
                if total > 0 {
                    format!("{}/s", crate::tui::cpu::human_bytes(total))
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Net => {
                let total_bps = proc.net_rx_bps + proc.net_tx_bps;
                let total_bytes = proc.net_rx_bytes + proc.net_tx_bytes;
                if total_bps > 0 {
                    format!("{}/s", crate::tui::cpu::human_bytes(total_bps))
                } else if total_bytes > 0 {
                    crate::tui::cpu::human_bytes(total_bytes)
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Cpu => format!("{:.0}%", proc.cpu_percent),
        }
    }
}
