//! On Windows, puts the Ratão icon inside the .exe, where Explorer, the
//! Start menu, the Apps list and a pinned taskbar button look for it (an
//! open window sets its own icon at run time). Nothing on other systems.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/mickey.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../assets/icons/mickey.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the Windows icon resource compiles");
    }
}
