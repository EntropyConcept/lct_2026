mod app;
mod fast;
mod import;
mod model;
mod norms;
mod routing;
mod sat;
#[cfg(test)]
mod tests;

use model::Result;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use tiny_http::{Header, Response, Server, StatusCode};

fn datasets() -> Result<Vec<PathBuf>> {
    let mut paths: Vec<_> = std::fs::read_dir("dataset")
        .map_err(|e| e.to_string())?
        .filter_map(|p| p.ok().map(|p| p.path()))
        .filter(|p| {
            p.to_string_lossy().contains("Синтетические")
                && p.extension().is_some_and(|x| x == "csv")
        })
        .collect();
    paths.sort();
    Ok(paths)
}
fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}
fn api(method: &str, url: &str, body: &str) -> Result<String> {
    match (method, url) {
        ("GET", "/api/demo") => json(&import::demo()),
        ("GET", "/api/city-demo") => json(&import::parse(
            "city-example.json",
            include_str!("../city-example.json"),
        )?),
        ("GET", "/api/norms") => json(&norms::load()?),
        ("POST", "/api/routing") | ("POST", "/api/geocode") => {
            #[derive(serde::Deserialize)]
            struct Prepare {
                scenario: model::Scenario,
                #[serde(default)]
                confirm_coordinates: bool,
            }
            let req: Prepare =
                serde_json::from_str(body).map_err(|e| format!("Invalid request: {e}"))?;
            if url == "/api/geocode" {
                json(&routing::geocode(req.scenario)?)
            } else {
                json(&routing::prepare(req.scenario, req.confirm_coordinates)?)
            }
        }
        ("GET", "/api/datasets") => json(
            &datasets()
                .unwrap_or_default()
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
        ),
        ("POST", "/api/plan") => {
            let req = serde_json::from_str(body).map_err(|e| format!("Invalid request: {e}"))?;
            json(&app::run(req)?)
        }
        ("POST", "/api/import") => {
            #[derive(serde::Deserialize)]
            struct Upload {
                name: String,
                text: String,
            }
            let upload: Upload = serde_json::from_str(body).map_err(|e| e.to_string())?;
            json(&import::parse(&upload.name, &upload.text)?)
        }
        _ if method == "GET" && url.starts_with("/api/dataset/") => {
            let index: usize = url[13..].parse().map_err(|_| "Invalid dataset index")?;
            let files = datasets()?;
            json(&import::load(files.get(index).ok_or("Unknown dataset")?)?)
        }
        _ => Err("Unknown endpoint".into()),
    }
}
fn serve(port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Expected TCP listener")?
        .port();
    eprintln!("Dispatcher: http://127.0.0.1:{port} (Ctrl-C to stop)");
    // ponytail: one solve at a time for the local demo; add a bounded worker pool for multiple dispatchers.
    for mut req in server.incoming_requests() {
        let method = req.method().as_str().to_owned();
        let url = req.url().to_owned();
        let hosts = ["127.0.0.1", "localhost"].map(|h| {
            if port == 80 {
                h.to_owned()
            } else {
                format!("{h}:{port}")
            }
        });
        let trusted_host = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .is_some_and(|h| hosts.iter().any(|host| h.value.as_str() == host));
        let trusted_origin = req
            .headers()
            .iter()
            .filter(|h| h.field.equiv("Origin"))
            .all(|h| {
                hosts
                    .iter()
                    .any(|host| h.value.as_str() == format!("http://{host}"))
            });
        let (status, content_type, text) = if !trusted_host || !trusted_origin {
            (
                403,
                "application/json; charset=utf-8",
                "{\"error\":\"Only same-origin localhost requests are allowed\"}".into(),
            )
        } else if method == "GET" && url == "/" {
            (
                200,
                "text/html; charset=utf-8",
                include_str!("../web/index.html").to_owned(),
            )
        } else if method == "GET" && url == "/vendor/leaflet.js" {
            (
                200,
                "text/javascript; charset=utf-8",
                include_str!("../web/vendor/leaflet.js").to_owned(),
            )
        } else if method == "GET" && url == "/vendor/leaflet.css" {
            (
                200,
                "text/css; charset=utf-8",
                include_str!("../web/vendor/leaflet.css").to_owned(),
            )
        } else {
            let mut body = String::new();
            let read = req.as_reader().take(2_000_001).read_to_string(&mut body);
            let result = if read.is_err() || body.len() > 2_000_000 {
                Err("Request body must be UTF-8 and at most 2 MB".into())
            } else {
                api(&method, &url, &body)
            };
            match result {
                Ok(text) => (200, "application/json; charset=utf-8", text),
                Err(error) => (
                    400,
                    "application/json; charset=utf-8",
                    serde_json::json!({"error":error}).to_string(),
                ),
            }
        };
        let response = Response::from_string(text)
            .with_status_code(StatusCode(status))
            .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
            .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap());
        if let Err(e) = req.respond(response) {
            eprintln!("HTTP response: {e}");
        }
    }
    Ok(())
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(args.get(1).map(|s| s.parse()).transpose().map_err(|_| "Invalid port")?.unwrap_or(8080)),
        Some("solve") if args.len() <= 4 => {
            let scenario = match args.get(1).map(String::as_str) { None | Some("demo") => import::demo(), Some(path) => import::load(Path::new(path))? };
            let seconds = args.get(2).map(|s| s.parse()).transpose().map_err(|_| "Invalid seconds")?.unwrap_or(5.0);
            let mode = match args.get(3).map(String::as_str).unwrap_or("fast") {
                "fast" => app::SearchMode::Fast, "exact" => app::SearchMode::Exact,
                _ => return Err("Mode must be fast or exact".into()),
            };
            let result = app::run(app::Request { scenario, seconds, mode, previous: None, event: None, last_event_time: 0 })?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
            Ok(())
        }
        Some("benchmark-matrix") if args.len() == 3 => {
            if std::env::var_os("DISPATCH_ROUTING_CACHE").is_none() { return Err("Benchmark matrices require an explicit isolated DISPATCH_ROUTING_CACHE".into()); }
            let mut scenario = import::load(Path::new(&args[1]))?;
            let snapshot: routing::Snapshot = serde_json::from_str(&std::fs::read_to_string(&args[2]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if snapshot.points.iter().any(|p| !p.lat.is_finite() || !p.lon.is_finite() || p.lat.abs()>90.0 || p.lon.abs()>180.0)
                || snapshot.profiles.iter().any(|p| p.transport == model::Transport::Public || p.legs.iter().flatten().flatten().any(|l| l.minutes>100_000 || l.metres>20_000_000 || !l.shape.is_empty())) {
                return Err("Invalid table-only benchmark matrix".into());
            }
            let mut reference = routing::save(snapshot)?;
            reference.source = "External road table / benchmark only / no route geometry".into();
            scenario.routing = Some(reference);
            model::Travel::new(&scenario)?; // Check every job/base/profile resolves.
            println!("{}", json(&scenario)?);
            Ok(())
        }
        Some("heuristic") if args.len() == 3 => {
            let scenario = import::load(Path::new(&args[1]))?;
            let started = std::time::Instant::now();
            let travel = model::Travel::new(&scenario)?;
            let insertion = match args[2].as_str() { "append" => false, "insertion" => true, _ => return Err("Heuristic must be append or insertion".into()) };
            let plan = model::greedy(&scenario, &travel, insertion, None)?;
            println!("{}", serde_json::json!({"plan":plan,"elapsed_ms":started.elapsed().as_millis()})); Ok(())
        }
        Some("check-orders") if args.len() == 3 => {
            let scenario = import::load(Path::new(&args[1]))?;
            let orders: Vec<Vec<usize>> = serde_json::from_str(&std::fs::read_to_string(&args[2]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if orders.len()!=scenario.engineers.len() || orders.iter().flatten().any(|&j|j>=scenario.jobs.len()) { return Err("Invalid benchmark job indices".into()); }
            let plan = model::make_plan(&scenario, &model::Travel::new(&scenario)?, &orders)?;
            println!("{}", json(&plan)?); Ok(())
        }
        Some("norms") if args.len() == 1 => { println!("{}", json(&norms::load()?)?); Ok(()) }
        Some("route") if args.len() == 3 && args[2] == "--confirmed" => {
            println!("{}", json(&routing::prepare(import::load(Path::new(&args[1]))?, true)?)?); Ok(())
        }
        Some("geocode") if args.len() == 2 => {
            println!("{}", json(&routing::geocode(import::load(Path::new(&args[1]))?)?)?); Ok(())
        }
        Some("import") if args.len() == 2 => {
            println!("{}", serde_json::to_string_pretty(&import::load(Path::new(&args[1]))?).map_err(|e| e.to_string())?); Ok(())
        }
        _ => Err("Usage: dispatch-sat [serve [PORT] | solve [demo|FILE.json|FILE.csv] [SECONDS] [fast|exact] | import FILE.csv | norms | geocode FILE | route FILE.json --confirmed]".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
