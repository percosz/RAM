use embed_manifest::manifest::{ActiveCodePage, DpiAwareness, ExecutionLevel, SupportedOS};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Common Controls v6, per-monitor DPI awareness, and an explicit
        // asInvoker level: the app never asks for elevation.
        let manifest = new_manifest("Waypoint.AccountManager")
            .supported_os(SupportedOS::Windows10..=SupportedOS::Windows10)
            .active_code_page(ActiveCodePage::Utf8)
            .dpi_awareness(DpiAwareness::PerMonitorV2)
            .requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(manifest).expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
