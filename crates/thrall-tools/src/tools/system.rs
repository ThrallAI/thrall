//! `system_info`: the computer Thrall runs on — system, CPU, memory, disks, uptime.

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Describe this computer.
pub struct SystemInfo;

fn gib(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let g = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{g:.1} GB")
}

/// The report, in plain lines.
pub fn report() -> String {
    use sysinfo::{Disks, System};
    let mut sys = System::new_all();
    sys.refresh_all();
    let mut out = String::new();
    out.push_str(&format!(
        "System: {}\n",
        System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned())
    ));
    if let Some(host) = System::host_name() {
        out.push_str(&format!("Computer name: {host}\n"));
    }
    let cpus = sys.cpus();
    let brand = cpus
        .first()
        .map(|c| c.brand().trim().to_owned())
        .unwrap_or_default();
    out.push_str(&format!(
        "CPU: {brand} ({} cores, {} threads)\n",
        System::physical_core_count().unwrap_or(0),
        cpus.len()
    ));
    out.push_str(&format!(
        "Memory: {} used of {}\n",
        gib(sys.used_memory()),
        gib(sys.total_memory())
    ));
    let disks = Disks::new_with_refreshed_list();
    for d in disks.list() {
        out.push_str(&format!(
            "Disk {}: {} free of {}\n",
            d.mount_point().display(),
            gib(d.available_space()),
            gib(d.total_space())
        ));
    }
    let up = System::uptime();
    out.push_str(&format!("Up for: {}h {}m\n", up / 3600, (up % 3600) / 60));
    out
}

impl Tool for SystemInfo {
    fn name(&self) -> &'static str {
        "system_info"
    }
    fn description(&self) -> &'static str {
        "Facts about this computer: operating system, CPU, memory in use, free disk space, uptime."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn title(&self, _args: &Value) -> String {
        "Checking this computer".to_owned()
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match tokio::task::spawn_blocking(report).await {
                Ok(text) => {
                    let first = text.lines().next().unwrap_or_default().to_owned();
                    ToolOutput::ok(text, first)
                }
                Err(e) => ToolOutput::error(format!("stopped: {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_report_names_the_basics() {
        let text = super::report();
        assert!(text.contains("System:"), "{text}");
        assert!(text.contains("CPU:"), "{text}");
        assert!(text.contains("Memory:"), "{text}");
    }
}
