fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
}

fn embed_windows_resources() {
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let level = if release {
        "requireAdministrator"
    } else {
        "asInvoker"
    };

    let manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="{level}" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
</assembly>
"#
    );

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "HS Reconnect")
        .set("FileDescription", "HS Reconnect")
        .set("OriginalFilename", "HSReconnect.exe")
        .set_manifest(&manifest);
    res.compile().expect("failed to embed Windows resources");

    println!("cargo:rerun-if-changed=assets/icon.ico");
}
