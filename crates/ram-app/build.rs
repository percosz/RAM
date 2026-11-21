use std::io::Write;

/// Application manifest: Common Controls v6, per-monitor-v2 DPI awareness,
/// UTF-8 code page, and an explicit asInvoker level (the app never elevates).
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="RAM.AccountManager" version="1.0.0.0"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges>
      <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
    </requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
      processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1"><application>
    <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
  </application></compatibility>
</assembly>
"#;

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/RAM.ico");
        res.set_manifest(MANIFEST);
        res.set("ProductName", "RAM");
        res.set("FileDescription", "RAM \u{2014} a Roblox account manager");
        res.set("ProductVersion", version);
        res.set("FileVersion", version);
        res.set("CompanyName", "RAM contributors");
        res.set("LegalCopyright", "GPL-3.0-or-later. Not affiliated with Roblox Corporation.");
        res.set("OriginalFilename", "RAM.exe");
        res.set("InternalName", "RAM");
        if let Err(e) = res.compile() {
            // Never fail the build over resource embedding; warn and continue.
            let _ = writeln!(std::io::stderr(), "cargo:warning=resource embed failed: {e}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/RAM.ico");
}
