//! The two cfgs the platform code is written against, so a Termux build (Android's C library and kernel,
//! but a desktop X11 window and Linux conventions) takes the desktop Linux paths:
//! `android_app` is the Android app (native activity, JNI), `linux_desktop` is desktop Linux or Termux.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(android_app)");
    println!("cargo::rustc-check-cfg=cfg(linux_desktop)");
    println!("cargo::rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let termux = std::env::var_os("CARGO_FEATURE_TERMUX").is_some();
    if os == "android" && !termux {
        println!("cargo::rustc-cfg=android_app");
    }
    if os == "linux" || (os == "android" && termux) {
        println!("cargo::rustc-cfg=linux_desktop");
    }
}
