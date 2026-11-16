use std::{env, fs, path::Path};
fn visit(dir: &Path, root: &Path, entries: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("web assets") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit(&path, root, entries);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let mime = match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
                "html" => "text/html; charset=utf-8",
                "js" => "application/javascript; charset=utf-8",
                "css" => "text/css; charset=utf-8",
                "woff2" => "font/woff2",
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "svg" => "image/svg+xml",
                _ => "application/octet-stream",
            };
            entries.push(format!(
                "{name:?} => Some(({mime:?}, include_bytes!({:?}).as_slice())),",
                path.canonicalize().unwrap()
            ));
        }
    }
}
fn main() {
    println!("cargo:rerun-if-changed=web");
    let mut entries = Vec::new();
    visit(Path::new("web"), Path::new("web"), &mut entries);
    entries.sort();
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("web_assets.rs"),format!("fn web_asset(path: &str) -> Option<(&'static str, &'static [u8])> {{ match path {{ {} _ => None }} }}",entries.join("\n"))).unwrap();
}
