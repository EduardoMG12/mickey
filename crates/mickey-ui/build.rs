//! On Windows, puts the Ratão icon inside the .exe, where Explorer, the
//! Start menu, the Apps list and a pinned taskbar button look for it (an
//! open window sets its own icon at run time), and the program's name,
//! version and description (code signing checks them). Nothing on other
//! systems.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/mickey.ico");
    println!("cargo:rerun-if-changed=../../assets/icons/mickey.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The version for the file's Details, from Cargo.toml.
        let version = |part: &str| {
            let number = std::env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap_or_default();
            format!("VERSION_{part}={number}")
        };
        let macros = [
            "MICKEY_UI".to_string(),
            version("MAJOR"),
            version("MINOR"),
            version("PATCH"),
        ];
        embed_resource::compile("../../assets/icons/mickey.rc", macros)
            .manifest_optional()
            .expect("the Windows icon resource compiles");
    }
}
