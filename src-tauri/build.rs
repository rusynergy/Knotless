fn main() {
    if std::env::var("PROFILE").as_deref()==Ok("release") {
        let path="resources/bundled-xray/current/xray/manifest.json";
        println!("cargo:rerun-if-changed={path}");
        let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(path).expect("Use scripts/build.ps1 to prepare bundled Xray")).expect("Invalid Xray manifest");
        let arch=if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref()==Ok("aarch64"){"arm64"}else{"x64"};
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref()==Ok("macos"){println!("cargo:rustc-env=KNOTLESS_MAC_XRAY_HASH={}",manifest["files"]["xray"].as_str().expect("Missing macOS Xray checksum"));}
        assert_eq!(manifest["architecture"].as_str(),Some(arch),"Bundled Xray architecture mismatch; use scripts/build.ps1");
    }
    tauri_build::build()
}
