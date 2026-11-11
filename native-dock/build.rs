//! Discover native source packages at build time. No DLL loading or installers.
use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .join("widgets");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .expect("widgets folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut modules = String::new();
    let mut definitions = String::from("vec![\n");
    for dir in dirs {
        let manifest = dir.join("widget.json");
        if !manifest.is_file() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).expect("valid widget.json");
        let Some(entry) = value["nativeEntry"].as_str() else {
            continue;
        };
        let id = value["id"].as_str().expect("widget id");
        let default_module = id.replace('-', "_");
        let module = value["nativeModule"].as_str().unwrap_or(&default_module);
        assert!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid package id"
        );
        assert!(
            module.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "native module must be a Rust identifier"
        );
        let source = dir.join(entry).canonicalize().expect("nativeEntry exists");
        assert!(
            source.starts_with(dir.canonicalize().unwrap()),
            "nativeEntry must stay in package"
        );
        modules.push_str(&format!(
            "#[path = {:?}] mod {module};\n",
            source.to_str().unwrap()
        ));
        if id != "launcher" {
            definitions.push_str(&format!("crate::{module}::package::definition(),\n"));
        }
    }
    definitions.push(']');
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("widget_modules.rs"), modules).unwrap();
    fs::write(out.join("widget_definitions.rs"), definitions).unwrap();
}
