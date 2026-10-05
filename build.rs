//! Build script: embed a Windows application manifest so the binary opts into
//! long paths (> 260 chars, effective where the system enables them) and a
//! UTF-8 active code page. `new_manifest` ships both as its defaults; on
//! non-Windows targets nothing is embedded.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest::embed_manifest(embed_manifest::new_manifest("skill-one.agents-skills"))
            .expect("embed Windows application manifest");
    }
}
