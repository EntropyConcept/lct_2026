//! Serve the Vite build without coupling CLI/solver builds to Node.
use std::path::{Component, Path, PathBuf};

pub fn asset(url: &str) -> (u16, &'static str, Vec<u8>) {
    let root = std::env::var_os("DISPATCH_FRONT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("front/dist"));
    read_asset(&root, url)
}

fn read_asset(root: &Path, url: &str) -> (u16, &'static str, Vec<u8>) {
    let error = |status, message: &str| {
        (
            status,
            "text/plain; charset=utf-8",
            message.as_bytes().to_vec(),
        )
    };
    let path = url.split('?').next().unwrap_or(url).trim_start_matches('/');
    // Vite emits plain ASCII asset paths. Reject encoded paths and traversal,
    // and resolve symlinks before checking containment in the build directory.
    if path.contains('%')
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return error(404, "Not found");
    }
    let Ok(root) = root.canonicalize() else {
        return error(
            503,
            "Frontend is not built. Run: npm --prefix front ci && npm --prefix front run build",
        );
    };
    let path = root.join(if path.is_empty() { "index.html" } else { path });
    let Ok(path) = path.canonicalize() else {
        return error(404, "Not found");
    };
    if !path.starts_with(&root) || !path.is_file() {
        return error(404, "Not found");
    }
    let content_type = match path.extension().and_then(|v| v.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    };
    match std::fs::read(path) {
        Ok(bytes) => (200, content_type, bytes),
        Err(_) => error(404, "Not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serves_build_and_rejects_traversal() {
        let root = std::env::temp_dir().join(format!("dispatch-front-{}", std::process::id()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "<div id=app></div>").unwrap();
        std::fs::write(root.join("assets/font.woff2"), [0, 159, 255]).unwrap();
        assert_eq!(read_asset(&root, "/?v=1").0, 200);
        assert_eq!(
            read_asset(&root, "/assets/font.woff2?v=1"),
            (200, "font/woff2", vec![0, 159, 255])
        );
        for path in [
            "/../Cargo.toml",
            "/%2e%2e/Cargo.toml",
            "/assets/../../Cargo.toml",
            "/missing.js",
        ] {
            assert_eq!(read_asset(&root, path).0, 404);
        }
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(read_asset(&root, "/").0, 503);
    }
}
