//! Street-graph snapshots, prepared outside SAT. Never fall back to straight lines.
use crate::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const UNREACHABLE: u32 = 100_000;
const MAX_GEOMETRY: usize = 32_000_000;
fn read_bounded(path: impl AsRef<Path>, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Routing cache file exceeds its size limit".into());
    }
    Ok(bytes)
}
fn geometry_bytes(s: &Snapshot) -> usize {
    s.profiles
        .iter()
        .flat_map(|p| &p.legs)
        .flatten()
        .flatten()
        .map(|l| l.shape.len())
        .sum()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingRef {
    pub key: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Leg {
    pub metres: u32,
    pub minutes: u32,
    pub shape: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub transport: Transport,
    pub legs: Vec<Vec<Option<Leg>>>,
    #[serde(skip)]
    pub lower: Vec<Vec<u32>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub points: Vec<Point>,
    pub profiles: Vec<Profile>,
}
type Snapshots = VecDeque<(String, Arc<Snapshot>)>;
static CACHE: OnceLock<Mutex<Snapshots>> = OnceLock::new();
fn cache() -> &'static Mutex<Snapshots> {
    CACHE.get_or_init(Mutex::default)
}
fn directory() -> PathBuf {
    std::env::var_os("DISPATCH_ROUTING_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".routing-cache".into())
}
fn key(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}
fn point_key(p: Point) -> (i64, i64) {
    ((p.lat * 1e6).round() as i64, (p.lon * 1e6).round() as i64)
}
pub fn point_index(points: &[Point], p: Point) -> Result<usize> {
    points
        .iter()
        .position(|&q| point_key(q) == point_key(p))
        .ok_or_else(|| "Coordinates changed: prepare the city routing snapshot again".into())
}
fn remember(id: &str, snap: Snapshot) -> Arc<Snapshot> {
    let snap = Arc::new(snap);
    let mut c = cache().lock().unwrap();
    c.retain(|(k, _)| k != id);
    c.push_back((id.into(), snap.clone()));
    // ponytail: four hot snapshots; older snapshots remain on disk for replay/replanning.
    while c.len() > 4 {
        c.pop_front();
    }
    snap
}
fn lower_bounds(s: &mut Snapshot) -> Result<()> {
    let n = s.points.len();
    if geometry_bytes(s) > MAX_GEOMETRY {
        return Err("Routing geometry exceeds 32 MB".into());
    }
    if n > 130 || s.profiles.len() > 3 {
        return Err("Invalid routing snapshot dimensions".into());
    }
    for profile in &mut s.profiles {
        if profile.legs.len() != n || profile.legs.iter().any(|r| r.len() != n) {
            return Err("Invalid routing matrix dimensions".into());
        }
        profile.lower = profile
            .legs
            .iter()
            .map(|r| {
                r.iter()
                    .map(|l| l.as_ref().map_or(UNREACHABLE, |l| l.minutes))
                    .collect()
            })
            .collect();
        // Only a LOWER bound: actual scheduled legs still use the original directed matrix.
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    profile.lower[i][j] =
                        profile.lower[i][j].min(profile.lower[i][k] + profile.lower[k][j]);
                }
            }
        }
    }
    Ok(())
}
pub fn load(id: &str) -> Result<Arc<Snapshot>> {
    if id.len() != 16 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid routing snapshot key".into());
    }
    if let Some((_, s)) = cache().lock().unwrap().iter().find(|(k, _)| k == id) {
        return Ok(s.clone());
    }
    let bytes = read_bounded(directory().join(format!("{id}.json")), 128_000_000).map_err(|e| {
        format!("Routing snapshot unavailable: {e}. Keep .routing-cache with exported plans.")
    })?;
    if bytes.len() > 128_000_000 || key(&bytes) != id {
        return Err("Damaged routing snapshot".into());
    }
    let mut s: Snapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    lower_bounds(&mut s)?;
    Ok(remember(id, s))
}
pub(crate) fn save(mut s: Snapshot) -> Result<RoutingRef> {
    lower_bounds(&mut s)?;
    let bytes = serde_json::to_vec(&s).map_err(|e| e.to_string())?;
    if bytes.len() > 128_000_000 {
        return Err("Routing snapshot exceeds 128 MB".into());
    }
    let id = key(&bytes);
    std::fs::create_dir_all(directory()).map_err(|e| e.to_string())?;
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tmp = directory().join(format!(
        "{id}-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, directory().join(format!("{id}.json"))).map_err(|e| e.to_string())?;
    remember(&id, s);
    Ok(RoutingRef {
        key: id,
        source: "Valhalla / OpenStreetMap · static street-time estimates, not live traffic".into(),
    })
}
fn agent() -> &'static ureq::Agent {
    static HTTP: OnceLock<ureq::Agent> = OnceLock::new();
    HTTP.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(45)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}
fn response(mut r: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = r.status();
    let v: Value = r
        .body_mut()
        .with_config()
        .limit(32_000_000)
        .read_json()
        .map_err(|e| format!("Routing response: {e}"))?;
    if !status.is_success() || v.get("error").is_some() {
        return Err(format!(
            "Routing provider ({status}): {}",
            v.get("error")
                .unwrap_or(&v)
                .to_string()
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    Ok(v)
}
fn matrix(
    profile: Transport,
    sources: &[Point],
    targets: &[Point],
) -> Result<Vec<Vec<Option<Leg>>>> {
    let costing = match profile { Transport::Car => "auto", Transport::Walk => "pedestrian", Transport::Bicycle => "bicycle", Transport::Public => return Err("Public transport requires a timetable-aware backend and Moscow GTFS. It is NOT approximated with a speed or walking route. Explicitly deselect public-transport engineers for street routing.".into()) };
    let requested = (sources.len(), targets.len());
    let (mut from, mut to) = (sources.to_vec(), targets.to_vec());
    // Valhalla selects TimeDistanceMatrix for <=5 walking/cycling sources OR
    // targets, which ignores shape_format. Duplicate padding selects CostMatrix;
    // discard the dummy rows/columns, never change the requested routes/costs.
    if profile != Transport::Car {
        while from.len() < 6 {
            from.push(sources[0]);
        }
        while to.len() < 6 {
            to.push(targets[0]);
        }
    }
    let (sources, targets) = (from.as_slice(), to.as_slice());
    let url = std::env::var("DISPATCH_VALHALLA_URL")
        .unwrap_or_else(|_| "https://valhalla1.openstreetmap.de".into());
    let payload = json!({"sources":sources,"targets":targets,"costing":costing,"units":"kilometers","shape_format":"polyline6","verbose":true});
    let id = key(format!("matrix-v1/{url}/{payload}").as_bytes());
    let file = directory().join(format!("matrix-{id}.json"));
    let fresh = file
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .is_some_and(|t| {
            t.elapsed()
                .is_ok_and(|age| age < Duration::from_secs(86_400))
        });
    let value = if fresh {
        serde_json::from_slice(&read_bounded(&file, 32_000_000)?)
            .map_err(|e| format!("Corrupt matrix cache: {e}"))?
    } else {
        let r = agent()
            .post(format!("{}/sources_to_targets", url.trim_end_matches('/')))
            .send_json(&payload)
            .map_err(|e| format!("Valhalla unavailable: {e}. No straight-line fallback."))?;
        let v = response(r)?;
        // Public demo is a convenience for small evaluations; self-host for operational batches.
        parse_matrix(&v, sources, targets)?;
        std::fs::create_dir_all(directory()).map_err(|e| e.to_string())?;
        std::fs::write(file, serde_json::to_vec(&v).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        v
    };
    let mut rows = parse_matrix(&value, sources, targets)?;
    rows.truncate(requested.0);
    for row in &mut rows {
        row.truncate(requested.1);
    }
    Ok(rows)
}
fn parse_matrix(v: &Value, sources: &[Point], targets: &[Point]) -> Result<Vec<Vec<Option<Leg>>>> {
    if v["units"] != "kilometers" {
        return Err("Provider did not return kilometres".into());
    }
    let rows = v["sources_to_targets"]
        .as_array()
        .ok_or("Missing routing matrix")?;
    if rows.len() != sources.len() {
        return Err("Incomplete routing matrix".into());
    }
    rows.iter().enumerate().map(|(i, row)| {
        let row = row.as_array().ok_or("Invalid matrix row")?;
        if row.len() != targets.len() { return Err("Incomplete matrix row".into()); }
        row.iter().enumerate().map(|(j, cell)| {
            if cell["from_index"].as_u64() != Some(i as u64) || cell["to_index"].as_u64() != Some(j as u64) { return Err("Wrong routing matrix indices".into()); }
            if cell.get("time") == Some(&Value::Null) && cell.get("distance") == Some(&Value::Null) { return Ok(None); }
            let seconds = cell["time"].as_f64().ok_or("Missing travel time")?;
            let km = cell["distance"].as_f64().ok_or("Missing travel distance")?;
            if !seconds.is_finite() || !(0.0..=6_000_000.0).contains(&seconds) || !km.is_finite() || !(0.0..=20_000.0).contains(&km) { return Err("Invalid routing time/distance".into()); }
            let shape = cell["shape"].as_str().unwrap_or("");
            if shape.len() > 200_000 || (!shape.is_empty() && decode_shape(shape).is_err()) || (shape.is_empty() && point_key(sources[i]) != point_key(targets[j])) { return Err("Provider omitted/returned invalid street geometry; use a Valhalla version with matrix shape_format support".into()); }
            // Matrix seconds can be truncated; reserve one second before rounding to minutes.
            let minutes = ((seconds + if km > 0.0 { 1.0 } else { 0.0 }) / 60.0).ceil() as u32;
            Ok(Some(Leg { minutes, metres: (km * 1000.0).ceil() as u32, shape: shape.into() }))
        }).collect()
    }).collect()
}
pub fn decode_shape(shape: &str) -> Result<Vec<Point>> {
    let mut bytes = shape.bytes();
    let mut position = [0i64; 2];
    let mut points = vec![];
    while let Some(first) = bytes.next() {
        for (axis, start) in [Some(first), None].into_iter().enumerate() {
            let mut byte = start.or_else(|| bytes.next()).ok_or("Truncated polyline")?;
            let mut value = 0i64;
            let mut shift = 0;
            loop {
                if !(63..=126).contains(&byte) || shift > 30 {
                    return Err("Invalid polyline".into());
                }
                let b = byte - 63;
                value |= ((b & 31) as i64) << shift;
                shift += 5;
                if b < 32 {
                    break;
                }
                byte = bytes.next().ok_or("Truncated polyline")?;
            }
            position[axis] += if value & 1 != 0 {
                !(value >> 1)
            } else {
                value >> 1
            };
        }
        let p = Point {
            lat: position[0] as f64 / 1e6,
            lon: position[1] as f64 / 1e6,
        };
        if p.lat.abs() > 90.0 || p.lon.abs() > 180.0 {
            return Err("Polyline outside world".into());
        }
        points.push(p);
    }
    if points.is_empty() {
        return Err("Empty polyline".into());
    }
    Ok(points)
}

pub fn prepare(mut s: Scenario, confirm_coordinates: bool) -> Result<Scenario> {
    validate_input(&s)?;
    if !confirm_coordinates {
        return Err("Verify every job coordinate AND engineer base first; CSV coordinates/bases are fictional until corrected.".into());
    }
    if s.engineers.iter().any(|e| e.transport == Transport::Public) {
        return Err("Public transport is unavailable: this provider has no Moscow GTFS/timetables. Explicitly deselect public-transport engineers, or stay in the labeled offline demo. No walking/speed substitution is made.".into());
    }
    let mut snapshot = match &s.routing {
        Some(r) => (*load(&r.key)?).clone(),
        None => Snapshot {
            points: vec![],
            profiles: vec![],
        },
    };
    for p in s
        .jobs
        .iter()
        .map(|j| j.point)
        .chain(s.engineers.iter().map(|e| e.start))
    {
        if point_index(&snapshot.points, p).is_err() {
            snapshot.points.push(p);
        }
    }
    if snapshot.points.len() > 130 {
        return Err(
            "Snapshot limit: 130 distinct points; start a new day before adding more".into(),
        );
    }
    snapshot
        .profiles
        .retain(|p| s.engineers.iter().any(|e| e.transport == p.transport));
    let mut geometry_size = geometry_bytes(&snapshot);
    let n = snapshot.points.len();
    let deadline = Instant::now() + Duration::from_secs(240);
    for transport in [Transport::Car, Transport::Walk, Transport::Bicycle] {
        if !s.engineers.iter().any(|e| e.transport == transport) {
            continue;
        }
        let idx = if let Some(i) = snapshot
            .profiles
            .iter()
            .position(|p| p.transport == transport)
        {
            i
        } else {
            snapshot.profiles.push(Profile {
                transport,
                legs: vec![],
                lower: vec![],
            });
            snapshot.profiles.len() - 1
        };
        let p = &mut snapshot.profiles[idx];
        let old = p.legs.len();
        for row in &mut p.legs {
            row.resize(n, None);
        }
        p.legs.resize(n, vec![None; n]);
        // Preserve ALL old arcs across an event. Only new rows/columns are fetched.
        for (sr, tr) in [(0..old, old..n), (old..n, 0..n)] {
            for a in (sr.start..sr.end).step_by(10) {
                for b in (tr.start..tr.end).step_by(10) {
                    if Instant::now() >= deadline {
                        return Err("Routing preparation exceeded four minutes; cached blocks are retained. Retry or use a local Valhalla instance.".into());
                    }
                    let rows = matrix(
                        transport,
                        &snapshot.points[a..(a + 10).min(sr.end)],
                        &snapshot.points[b..(b + 10).min(tr.end)],
                    )?;
                    for (i, row) in rows.into_iter().enumerate() {
                        for (j, leg) in row.into_iter().enumerate() {
                            geometry_size += leg.as_ref().map_or(0, |l| l.shape.len());
                            if geometry_size > MAX_GEOMETRY {
                                return Err(
                                    "Routing geometry exceeds 32 MB; reduce the planning area"
                                        .into(),
                                );
                            }
                            p.legs[a + i][b + j] = leg;
                        }
                    }
                }
            }
        }
    }
    s.routing = Some(save(snapshot)?);
    let note = "Городские маршруты: геометрия, направленные расстояния и времена Valhalla/OSM по виду транспорта. Координаты подтверждены оператором; оценки статические, без онлайн-пробок. Общественный транспорт требует отдельного расписания GTFS и сейчас недоступен.";
    if s.notes.len() < 32 && !s.notes.iter().any(|n| n == note) {
        s.notes.push(note.into());
    }
    Ok(s)
}

pub fn geocode(mut s: Scenario) -> Result<Scenario> {
    validate_input(&s)?;
    s.routing = None;
    s.notes.retain(|n| !n.starts_with("Nominatim:"));
    if s.notes.len() >= 32 {
        return Err("Leave room for the geocoding diagnostic in scenario notes".into());
    }
    let deadline = Instant::now() + Duration::from_secs(240);
    let url = std::env::var("DISPATCH_NOMINATIM_URL")
        .unwrap_or_else(|_| "https://nominatim.openstreetmap.org".into());
    let mut failed = vec![];
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    for j in &mut s.jobs {
        if Instant::now() >= deadline {
            return Err(
                "Geocoding exceeded four minutes; cached matches are retained for a retry".into(),
            );
        }
        if j.address.trim().is_empty() {
            j.geocode_match = Some("Адрес пуст — координаты не найдены".into());
            failed.push(j.id.clone());
            continue;
        }
        let query = j
            .address
            .replace("Город Москва", "Москва")
            .replace("ул.", "улица ")
            .replace("д.", " ")
            .replace("проезд.", "проезд ")
            .replace("ш.", "шоссе ");
        let id = key(format!("geocode/{url}/{query}").as_bytes());
        let file = directory().join(format!("geo-{id}.json"));
        let v: Value = if file.exists() {
            serde_json::from_slice(&read_bounded(&file, 32_000_000)?).map_err(|e| e.to_string())?
        } else {
            let mut last = LAST.get_or_init(Mutex::default).lock().unwrap();
            if let Some(t) = *last {
                std::thread::sleep(Duration::from_millis(1100).saturating_sub(t.elapsed()));
            }
            *last = Some(Instant::now());
            let r = agent()
                .get(format!("{}/search", url.trim_end_matches('/')))
                .query("q", &query)
                .query("format", "jsonv2")
                .query("limit", "1")
                .query("countrycodes", "ru")
                .header(
                    "User-Agent",
                    "Kontur-Dispatcher/0.1 (local operator-controlled prototype)",
                )
                .call()
                .map_err(|e| format!("Geocoder unavailable: {e}"))?;
            let v = response(r)?;
            std::fs::create_dir_all(directory()).map_err(|e| e.to_string())?;
            std::fs::write(file, serde_json::to_vec(&v).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            v
        };
        let p = v.as_array().and_then(|a| a.first()).and_then(|v| {
            Some(Point {
                lat: v["lat"].as_str()?.parse().ok()?,
                lon: v["lon"].as_str()?.parse().ok()?,
            })
        });
        if let Some(p) = p {
            j.point = p;
            j.geocode_match = v[0]["display_name"]
                .as_str()
                .map(|s| s.chars().take(1000).collect());
        } else {
            j.geocode_match =
                Some("НЕ НАЙДЕНО — старые координаты оставлены; исправьте вручную".into());
            failed.push(j.id.clone());
        }
    }
    s.notes.push(format!("Nominatim: предполагаемые совпадения адресов, не подтверждённая геолокация. Проверьте ВСЕ точки и задайте реальные базы инженеров. Не найдены (старые координаты оставлены): {}", if failed.is_empty() { "нет".into() } else { format!("{}; первые 10: {}", failed.len(), failed.iter().take(10).cloned().collect::<Vec<_>>().join(", ")) }));
    validate_input(&s)?;
    Ok(s)
}

#[cfg(test)]
pub(crate) use tests::register as test_snapshot;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directions_unreachable_geometry_and_lower_bounds() {
        let p = Point { lat: 0.0, lon: 0.0 };
        let q = Point {
            lat: 0.000001,
            lon: 0.000001,
        };
        let v = json!({"units":"kilometers","sources_to_targets":[[{"from_index":0,"to_index":0,"time":60,"distance":0.1,"shape":"??AA"},{"from_index":0,"to_index":1,"time":null,"distance":null}]]});
        let row = parse_matrix(&v, &[p], &[q, p]).unwrap().remove(0);
        assert_eq!(row[0].as_ref().unwrap().minutes, 2);
        assert!(row[1].is_none());
        assert_eq!(decode_shape("??AA").unwrap().len(), 2);
        assert!(decode_shape("_").is_err());
        let leg = |n| {
            Some(Leg {
                minutes: n,
                metres: 10,
                shape: "??AA".into(),
            })
        };
        let mut s = Snapshot {
            points: vec![p, q, p],
            profiles: vec![Profile {
                transport: Transport::Car,
                legs: vec![
                    vec![leg(0), leg(2), leg(50)],
                    vec![None, leg(0), leg(3)],
                    vec![None, None, leg(0)],
                ],
                lower: vec![],
            }],
        };
        lower_bounds(&mut s).unwrap();
        assert_eq!(s.profiles[0].lower[0][2], 5);
        assert_eq!(s.profiles[0].legs[0][2].as_ref().unwrap().minutes, 50);
        assert_eq!(s.profiles[0].lower[2][0], UNREACHABLE);
        let mut bad = v;
        bad["sources_to_targets"][0][0]["shape"] = json!("");
        assert!(parse_matrix(&bad, &[p], &[q, p]).is_err());
    }
    pub(crate) fn register(s: Snapshot) -> RoutingRef {
        save(s).unwrap()
    }
}
