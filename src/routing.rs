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
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const UNREACHABLE: u32 = 100_000;
const MAX_GEOMETRY: usize = 32_000_000;
const LEG_CACHE_TTL: u64 = 86_400;
const MAX_LEG_FILE: usize = 1_300_000;
pub(crate) fn read_bounded(path: impl AsRef<Path>, limit: usize) -> Result<Vec<u8>> {
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
        .sum::<usize>()
        + s.transit.as_ref().map_or(0, |t| t.geometry_bytes())
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
    // None is a legacy complete matrix; false is unqueried, not unreachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<Vec<bool>>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub points: Vec<Point>,
    pub profiles: Vec<Profile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transit: Option<crate::transit_snapshot::Timetable>,
}
type Snapshots = VecDeque<(String, Arc<Snapshot>)>;
static CACHE: OnceLock<Mutex<Snapshots>> = OnceLock::new();
fn cache() -> &'static Mutex<Snapshots> {
    CACHE.get_or_init(Mutex::default)
}
pub(crate) fn directory() -> PathBuf {
    std::env::var_os("DISPATCH_ROUTING_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".routing-cache".into())
}
pub(crate) fn key(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}
pub(crate) fn point_key(p: Point) -> (i64, i64) {
    ((p.lat * 1e6).round() as i64, (p.lon * 1e6).round() as i64)
}
enum RoadProvider {
    Valhalla(String),
    Motis(String),
}
fn road_provider() -> Result<RoadProvider> {
    match std::env::var("DISPATCH_ROAD_BACKEND")
        .unwrap_or_else(|_| "valhalla".into())
        .as_str()
    {
        "valhalla" => Ok(RoadProvider::Valhalla(
            std::env::var("DISPATCH_VALHALLA_URL")
                .unwrap_or_else(|_| "https://valhalla1.openstreetmap.de".into())
                .trim_end_matches('/')
                .to_owned(),
        )),
        "motis" => Ok(RoadProvider::Motis(crate::transit::configured_provider()?)),
        _ => Err("DISPATCH_ROAD_BACKEND must be motis or valhalla".into()),
    }
}
fn road_costing(profile: Transport) -> Result<&'static str> {
    match profile {
        Transport::Car => Ok("auto"),
        Transport::Walk => Ok("pedestrian"),
        Transport::Bicycle => Ok("bicycle"),
        Transport::Public => Err("Public transport requires a timetable-aware backend; no walking/speed substitution is made.".into()),
    }
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Invalid routing cache path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tmp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(tmp);
        return Err(e.to_string());
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct CachedLeg {
    identity: String,
    fetched_at: u64,
    // Missing data is corruption, not an explicit cached unreachable result.
    #[serde(deserialize_with = "Option::<Leg>::deserialize")]
    leg: Option<Leg>,
}
fn leg_identity(provider: &str, profile: Transport, from: Point, to: Point) -> Result<String> {
    Ok(format!(
        "road-leg-v1/{}/{}/{:?}/{:?}",
        provider.trim_end_matches('/'),
        road_costing(profile)?,
        point_key(from),
        point_key(to)
    ))
}
fn validate_cached_leg(leg: &Option<Leg>, same_point: bool) -> Result<()> {
    if let Some(leg) = leg {
        if leg.minutes > 100_001
            || leg.metres > 20_000_000
            || leg.shape.len() > 200_000
            || (leg.shape.is_empty() && !same_point)
            || (!leg.shape.is_empty() && decode_shape(&leg.shape).is_err())
        {
            return Err("Invalid directed road cache geometry/time/distance".into());
        }
    }
    Ok(())
}
// The outer option distinguishes a miss from a cached unreachable directed arc.
fn read_leg(
    root: &Path,
    identity: &str,
    now: u64,
    same_point: bool,
) -> Result<Option<Option<Leg>>> {
    let file = root.join(format!("leg-{}.json", key(identity.as_bytes())));
    match file.try_exists() {
        Ok(false) => return Ok(None),
        Err(e) => return Err(format!("Road cache unavailable: {e}")),
        Ok(true) => {}
    }
    let cached: CachedLeg = serde_json::from_slice(&read_bounded(&file, MAX_LEG_FILE)?)
        .map_err(|e| format!("Corrupt directed road cache: {e}"))?;
    if cached.identity != identity {
        return Err("Directed road cache identity mismatch".into());
    }
    validate_cached_leg(&cached.leg, same_point)?;
    if now
        .checked_sub(cached.fetched_at)
        .is_some_and(|age| age < LEG_CACHE_TTL)
    {
        Ok(Some(cached.leg))
    } else {
        Ok(None)
    }
}
fn write_leg(
    root: &Path,
    identity: String,
    fetched_at: u64,
    leg: Option<Leg>,
) -> Result<Option<Leg>> {
    let file = root.join(format!("leg-{}.json", key(identity.as_bytes())));
    let cached = CachedLeg {
        identity,
        fetched_at,
        leg,
    };
    let bytes = serde_json::to_vec(&cached).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_LEG_FILE {
        return Err("Directed road cache exceeds its size limit".into());
    }
    atomic_write(&file, &bytes)?;
    Ok(cached.leg)
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
    if n > 130 || s.profiles.len() > 4 {
        return Err("Invalid routing snapshot dimensions".into());
    }
    if let Some(timetable) = &mut s.transit {
        timetable.validate(n)?;
    }
    if geometry_bytes(s) > MAX_GEOMETRY {
        return Err("Routing geometry exceeds 32 MB".into());
    }
    if s.profiles.iter().any(|p| p.transport == Transport::Public) && s.transit.is_none() {
        return Err("Public profile requires a timetable snapshot".into());
    }
    for profile in &mut s.profiles {
        if profile.legs.len() != n || profile.legs.iter().any(|r| r.len() != n) {
            return Err("Invalid routing matrix dimensions".into());
        }
        if profile.coverage.as_ref().is_some_and(|coverage| {
            coverage.len() != n || coverage.iter().any(|row| row.len() != n)
        }) {
            return Err("Invalid routing coverage dimensions".into());
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
    atomic_write(&directory().join(format!("{id}.json")), &bytes)?;
    let source = s.transit.as_ref().map_or_else(
        || "OpenStreetMap · frozen street routes and static times, not live traffic".into(),
        |t| {
            format!(
                "OSM street routes + MOTIS / GTFS · {} Moscow UTC+03:00 · frozen departure timetables",
                t.date
            )
        },
    );
    remember(&id, s);
    Ok(RoutingRef { key: id, source })
}
pub(crate) fn agent() -> &'static ureq::Agent {
    static HTTP: OnceLock<ureq::Agent> = OnceLock::new();
    HTTP.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(45)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}
pub(crate) fn response(mut r: ureq::http::Response<ureq::Body>) -> Result<Value> {
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
    url: &str,
    profile: Transport,
    sources: &[Point],
    targets: &[Point],
) -> Result<Vec<Vec<Option<Leg>>>> {
    let costing = road_costing(profile)?;
    if sources.is_empty() || targets.is_empty() || sources.len() > 10 || targets.len() > 10 {
        return Err("Road matrix requests require 1–10 sources and targets".into());
    }
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
    let payload = json!({"sources":sources,"targets":targets,"costing":costing,"units":"kilometers","shape_format":"polyline6","verbose":true});
    // Public demos remain sequential; self-host for operational batches.
    let r = agent()
        .post(format!("{url}/sources_to_targets"))
        .send_json(&payload)
        .map_err(|e| format!("Valhalla unavailable: {e}. No straight-line fallback."))?;
    let value = response(r)?;
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

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn fill_road_profile(
    points: &[Point],
    profile: &mut Profile,
    root: &Path,
    provider: &str,
    now: u64,
    deadline: Instant,
    required: Option<&[Vec<Option<crate::transit_scope::Coverage>>]>,
    fetch: impl FnMut(Transport, &[Point], &[Point]) -> Result<Vec<Vec<Option<Leg>>>>,
) -> Result<()> {
    fill_road_profile_with(
        points, profile, root, provider, now, deadline, required, None, fetch,
    )
}

#[allow(clippy::too_many_arguments)]
fn fill_road_profile_with(
    points: &[Point],
    profile: &mut Profile,
    root: &Path,
    provider: &str,
    now: u64,
    deadline: Instant,
    required: Option<&[Vec<Option<crate::transit_scope::Coverage>>]>,
    motis: Option<(&str, &mut bool)>,
    mut fetch: impl FnMut(Transport, &[Point], &[Point]) -> Result<Vec<Vec<Option<Leg>>>>,
) -> Result<()> {
    let n = points.len();
    let old = profile.legs.len();
    if old > n || profile.legs.iter().any(|row| row.len() != old) {
        return Err("Invalid existing road matrix dimensions".into());
    }
    if required.is_some_and(|r| r.len() != n || r.iter().any(|row| row.len() != n)) {
        return Err("Invalid required road coverage dimensions".into());
    }
    let mut covered = profile
        .coverage
        .take()
        .unwrap_or_else(|| vec![vec![true; old]; old]);
    if covered.len() != old || covered.iter().any(|row| row.len() != old) {
        return Err("Invalid existing road coverage dimensions".into());
    }
    for row in &mut covered {
        row.resize(n, false);
    }
    covered.resize_with(n, || vec![false; n]);
    profile.coverage = Some(covered);
    let mut geometry_size: usize = profile
        .legs
        .iter()
        .flatten()
        .flatten()
        .map(|l| l.shape.len())
        .sum();
    for row in &mut profile.legs {
        row.resize(n, None);
    }
    profile.legs.resize_with(n, || vec![None; n]);
    let mut missing = vec![vec![false; n]; n];
    for i in 0..n {
        for j in 0..n {
            // Frozen snapshot arcs, including unreachable and diagonal cells, never refresh.
            if profile.coverage.as_ref().unwrap()[i][j] {
                continue;
            }
            if point_key(points[i]) == point_key(points[j]) {
                profile.legs[i][j] = Some(Leg {
                    metres: 0,
                    minutes: 0,
                    shape: String::new(),
                });
                profile.coverage.as_mut().unwrap()[i][j] = true;
                continue;
            }
            if required.is_some_and(|required| required[i][j].is_none()) {
                continue;
            }
            let identity = leg_identity(provider, profile.transport, points[i], points[j])?;
            if let Some(leg) = read_leg(root, &identity, now, false)? {
                geometry_size += leg.as_ref().map_or(0, |l| l.shape.len());
                profile.legs[i][j] = leg;
                profile.coverage.as_mut().unwrap()[i][j] = true;
            } else {
                missing[i][j] = true;
            }
            if geometry_size > MAX_GEOMETRY {
                return Err("Routing geometry exceeds 32 MB; reduce the planning area".into());
            }
        }
    }
    if let Some((base, ready)) = motis {
        let pending: Vec<_> = missing
            .iter()
            .enumerate()
            .flat_map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .filter_map(move |(j, missing)| missing.then_some((i, j)))
            })
            .collect();
        if !pending.is_empty() && !*ready {
            crate::street::check_provider(base, deadline)?;
            *ready = true;
        }
        let transport = profile.transport;
        // One queue for the entire profile: a slow leg never holds up the next
        // rectangle. Persist each completed leg immediately, including on failure.
        return crate::routing_work::run_bounded(
            pending.len(),
            crate::routing_work::configured_workers("DISPATCH_ROAD_WORKERS")?,
            |index| {
                let (i, j) = pending[index];
                crate::street::leg(base, transport, points[i], points[j], deadline)
            },
            |index, leg| {
                let (i, j) = pending[index];
                validate_cached_leg(&leg, false)?;
                geometry_size += leg.as_ref().map_or(0, |leg| leg.shape.len());
                if geometry_size > MAX_GEOMETRY {
                    return Err("Routing geometry exceeds 32 MB; reduce the planning area".into());
                }
                let identity = leg_identity(provider, transport, points[i], points[j])?;
                profile.legs[i][j] = write_leg(root, identity, now, leg)?;
                profile.coverage.as_mut().unwrap()[i][j] = true;
                Ok(())
            },
        );
    }
    while let Some(first) = missing.iter().position(|row| row.iter().any(|&v| v)) {
        if Instant::now() >= deadline {
            return Err("Routing preparation deadline exceeded; cached directed legs are retained. Retry or use a local road router.".into());
        }
        let full_row = first >= old && missing[first].iter().filter(|&&v| v).count() == n - 1;
        let targets: Vec<usize> = (0..n)
            .filter(|&j| missing[first][j] || (full_row && first == j))
            .take(10)
            .collect();
        // Greedily cover a rectangle of misses. Local diagonals may fill holes, but no
        // cached/frozen road arc is requested merely because it shares a block.
        let sources: Vec<usize> = (first..n)
            .filter(|&i| {
                targets.iter().any(|&j| missing[i][j])
                    && targets
                        .iter()
                        .all(|&j| missing[i][j] || (i == j && i >= old))
            })
            .take(10)
            .collect();
        let from: Vec<Point> = sources.iter().map(|&i| points[i]).collect();
        let to: Vec<Point> = targets.iter().map(|&j| points[j]).collect();
        let rows = fetch(profile.transport, &from, &to)?;
        if rows.len() != sources.len() || rows.iter().any(|row| row.len() != targets.len()) {
            return Err("Incomplete road matrix response".into());
        }
        for (i, row) in sources.into_iter().zip(rows) {
            for (&j, leg) in targets.iter().zip(row) {
                if !missing[i][j] {
                    continue;
                }
                validate_cached_leg(&leg, false)?;
                geometry_size += leg.as_ref().map_or(0, |l| l.shape.len());
                if geometry_size > MAX_GEOMETRY {
                    return Err("Routing geometry exceeds 32 MB; reduce the planning area".into());
                }
                let identity = leg_identity(provider, profile.transport, points[i], points[j])?;
                profile.legs[i][j] = write_leg(root, identity, now, leg)?;
                missing[i][j] = false;
                profile.coverage.as_mut().unwrap()[i][j] = true;
            }
        }
    }
    Ok(())
}

pub fn prepare(mut s: Scenario, confirm_coordinates: bool) -> Result<Scenario> {
    validate_input(&s)?;
    if !confirm_coordinates {
        return Err("Verify every job coordinate AND engineer base first; CSV coordinates/bases are fictional until corrected.".into());
    }
    if s.engineers.iter().any(|e| e.transport == Transport::Public) && s.routing.is_none() {
        crate::transit::configured_provider()?;
        crate::transit::midnight(
            s.transit_date
                .as_deref()
                .ok_or("Select a public transport service date (Moscow UTC+03:00)")?,
        )?;
    }
    let mut snapshot = match &s.routing {
        Some(r) => (*load(&r.key)?).clone(),
        None => Snapshot {
            points: vec![],
            profiles: vec![],
            transit: None,
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
    let root = directory();
    let provider = road_provider()?;
    let identity = match &provider {
        RoadProvider::Valhalla(url) => url.clone(),
        RoadProvider::Motis(url) => format!("{}/{url}", crate::street::CACHE_SETTINGS),
    };
    let mut motis_ready = false;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    // MOTIS returns geometry per directed path. Scope requests to feasible
    // transitions and bound concurrency; allow large cold datasets to finish.
    let deadline = Instant::now()
        + Duration::from_secs(match &provider {
            RoadProvider::Motis(_) => 1200,
            RoadProvider::Valhalla(_) => 240,
        });
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
                coverage: None,
            });
            snapshot.profiles.len() - 1
        };
        let required = crate::transit_scope::required_for(&s, &snapshot.points, transport)?;
        fill_road_profile_with(
            &snapshot.points,
            &mut snapshot.profiles[idx],
            &root,
            &identity,
            now,
            deadline,
            Some(&required),
            match &provider {
                RoadProvider::Motis(url) => Some((url.as_str(), &mut motis_ready)),
                _ => None,
            },
            |transport, sources, targets| match &provider {
                RoadProvider::Valhalla(url) => matrix(url, transport, sources, targets),
                RoadProvider::Motis(_) => unreachable!("MOTIS uses the directed work queue"),
            },
        )?;
        if geometry_bytes(&snapshot) > MAX_GEOMETRY {
            return Err("Routing geometry exceeds 32 MB; reduce the planning area".into());
        }
    }
    crate::transit::prepare(&s, &mut snapshot, deadline)?;
    s.routing = Some(save(snapshot)?);
    let note = "Городские маршруты: дорожные геометрия и статические времена по OpenStreetMap; общественный транспорт — снимок расписания MOTIS/GTFS на выбранную дату, Москва UTC+03:00, с ожиданием и пересадками. Выбирается доступный путь с наиболее ранним прибытием. Координаты подтверждены оператором; изменения движения после подготовки не учитываются.";
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
                    "BeeKeeper-Dispatcher/0.1 (local operator-controlled prototype)",
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
    fn snapshot_counts_transit_geometry_once_before_replaying_departures() {
        use crate::transit_snapshot::{Journey, Timetable, TransitLeg};
        let shape: Arc<str> = "??".repeat(10_000).into();
        let mut timetable = Timetable::new("2026-09-27".into(), "http://localhost".into());
        timetable.journeys = vec![
            vec![
                None,
                Some(
                    (0..2000)
                        .map(|index| Journey {
                            departure: index / 2,
                            arrival: index / 2 + 1,
                            leg: TransitLeg {
                                minutes: 1,
                                metres: index + 1,
                                shape: Arc::clone(&shape),
                            },
                            description: format!("BUS {index}"),
                        })
                        .collect(),
                ),
            ],
            vec![Some(vec![]), None],
        ];
        let mut snapshot = Snapshot {
            points: vec![
                Point { lat: 0.0, lon: 0.0 },
                Point {
                    lat: 0.0,
                    lon: 0.000001,
                },
            ],
            profiles: vec![timetable.profile()],
            transit: Some(timetable),
        };
        lower_bounds(&mut snapshot).unwrap();
        assert_eq!(geometry_bytes(&snapshot), shape.len());
        assert_eq!(snapshot.profiles[0].lower[0][1], 1);
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let mut restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
        lower_bounds(&mut restored).unwrap();
        let timetable = restored.transit.as_ref().unwrap();
        assert_eq!(timetable.journeys[0][1].as_ref().unwrap().len(), 2000);
        assert_eq!(
            timetable.journey(0, 1, 999).unwrap().description,
            "BUS 1998"
        );
        assert!(timetable.journey(0, 1, 1000).is_none());
    }

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
            transit: None,
            profiles: vec![Profile {
                transport: Transport::Car,
                legs: vec![
                    vec![leg(0), leg(2), leg(50)],
                    vec![None, leg(0), leg(3)],
                    vec![None, None, leg(0)],
                ],
                lower: vec![],
                coverage: None,
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
    struct TestCache(PathBuf);
    impl TestCache {
        fn new() -> Self {
            static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            Self(std::env::temp_dir().join(format!(
                "dispatch-road-{}-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
                SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        }
    }
    impl Drop for TestCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn empty_profile(transport: Transport) -> Profile {
        Profile {
            transport,
            legs: vec![],
            lower: vec![],
            coverage: None,
        }
    }
    fn road_points(n: usize) -> Vec<Point> {
        (0..n)
            .map(|i| Point {
                lat: i as f64 / 1000.0,
                lon: 0.0,
            })
            .collect()
    }
    fn test_leg() -> Option<Leg> {
        Some(Leg {
            metres: 10,
            minutes: 2,
            shape: "??AA".into(),
        })
    }
    #[test]
    fn directed_cache_survives_reordering_and_only_fetches_changed_coordinate() {
        let cache = TestCache::new();
        let mut points = road_points(12);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut profile = empty_profile(Transport::Car);
        fill_road_profile(
            &points,
            &mut profile,
            &cache.0,
            "provider",
            10,
            deadline,
            None,
            |_, from, to| {
                Ok(from
                    .iter()
                    .map(|p| {
                        to.iter()
                            .map(|q| {
                                if point_key(*p) > point_key(*q) {
                                    None
                                } else {
                                    test_leg()
                                }
                            })
                            .collect()
                    })
                    .collect())
            },
        )
        .unwrap();
        points.reverse();
        let mut reordered = empty_profile(Transport::Car);
        fill_road_profile(
            &points,
            &mut reordered,
            &cache.0,
            "provider",
            11,
            deadline,
            None,
            |_, _, _| {
                panic!("Fresh reachable and unreachable arcs must survive point reordering");
            },
        )
        .unwrap();
        assert!(reordered.legs[0][1].is_none());
        assert_eq!(reordered.legs[1][0].as_ref().unwrap().minutes, 2);
        points[5].lat = 0.5;
        let changed = point_key(points[5]);
        let mut fetched = std::collections::HashSet::new();
        let mut requests = 0;
        let mut edited = empty_profile(Transport::Car);
        fill_road_profile(
            &points,
            &mut edited,
            &cache.0,
            "provider",
            12,
            deadline,
            None,
            |_, from, to| {
                requests += 1;
                assert!(from.len() <= 10 && to.len() <= 10);
                for p in from {
                    for q in to {
                        let arc = (point_key(*p), point_key(*q));
                        if arc.0 != arc.1 {
                            assert!(arc.0 == changed || arc.1 == changed);
                            assert!(fetched.insert(arc), "Directed arc requested twice");
                        }
                    }
                }
                Ok(vec![vec![test_leg(); to.len()]; from.len()])
            },
        )
        .unwrap();
        assert_eq!(fetched.len(), 22);
        assert_eq!(requests, 4);
    }
    #[test]
    fn scoped_roads_preserve_plans_and_expand_without_refreshing_frozen_arcs() {
        let cache = TestCache::new();
        let mut scenario = crate::import::demo();
        scenario.engineers.truncate(1);
        scenario.jobs.truncate(3);
        for (i, job) in scenario.jobs.iter_mut().enumerate() {
            job.transport = None;
            job.skill = Skill::Local;
            job.duration = 30;
            job.window_start = 600 + i as u32 * 60;
            job.window_end = job.window_start;
        }
        let points: Vec<_> = scenario
            .jobs
            .iter()
            .map(|job| job.point)
            .chain(scenario.engineers.iter().map(|engineer| engineer.start))
            .collect();
        let required =
            crate::transit_scope::required_for(&scenario, &points, Transport::Car).unwrap();
        let n = points.len();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut full = empty_profile(Transport::Car);
        fill_road_profile(
            &points,
            &mut full,
            &cache.0,
            "full",
            10,
            deadline,
            None,
            |_, from, to| Ok(vec![vec![test_leg(); to.len()]; from.len()]),
        )
        .unwrap();
        let mut sparse = empty_profile(Transport::Car);
        let mut requested = 0;
        fill_road_profile(
            &points,
            &mut sparse,
            &cache.0,
            "sparse",
            10,
            deadline,
            Some(&required),
            |_, from, to| {
                for a in from {
                    for b in to {
                        let i = point_index(&points, *a).unwrap();
                        let j = point_index(&points, *b).unwrap();
                        assert!(i == j || required[i][j].is_some());
                        if i != j {
                            requested += 1;
                        }
                    }
                }
                Ok(vec![vec![test_leg(); to.len()]; from.len()])
            },
        )
        .unwrap();
        assert_eq!(requested, 6); // Three base legs and three forward job transitions, not twelve.
        for profile in [&full, &sparse] {
            scenario.routing = Some(register(Snapshot {
                points: points.clone(),
                profiles: vec![profile.clone()],
                transit: None,
            }));
            let plan = crate::app::run(crate::app::Request {
                scenario: scenario.clone(),
                seconds: 1.0,
                mode: crate::app::SearchMode::Exact,
                previous: None,
                event: None,
                last_event_time: 0,
            })
            .unwrap()
            .plan;
            assert_eq!(plan.metrics.unassigned, 0);
            assert_eq!(
                plan.routes[0]
                    .stops
                    .iter()
                    .map(|stop| &stop.job_id)
                    .collect::<Vec<_>>(),
                scenario.jobs.iter().map(|job| &job.id).collect::<Vec<_>>()
            );
            assert_eq!(plan.metrics.distance_m, 3 * test_leg().unwrap().metres);
        }
        // Window changes introduce old-point arcs; reject incomplete coverage before solving.
        scenario.jobs[0].window_end = 1000;
        assert!(Travel::new(&scenario)
            .err()
            .unwrap()
            .contains("Road coverage is incomplete"));
        let old = serde_json::to_value(&sparse.legs).unwrap();
        let covered = sparse.coverage.clone().unwrap();
        let expanded =
            crate::transit_scope::required_for(&scenario, &points, Transport::Car).unwrap();
        let mut added = 0;
        fill_road_profile(
            &points,
            &mut sparse,
            &cache.0,
            "sparse",
            LEG_CACHE_TTL * 2,
            deadline,
            Some(&expanded),
            |_, from, to| {
                for a in from {
                    for b in to {
                        let i = point_index(&points, *a).unwrap();
                        let j = point_index(&points, *b).unwrap();
                        assert!(!covered[i][j]);
                        added += 1;
                    }
                }
                Ok(vec![vec![test_leg(); to.len()]; from.len()])
            },
        )
        .unwrap();
        assert_eq!(added, 2);
        for (i, row) in covered.iter().enumerate() {
            for (j, known) in row.iter().enumerate() {
                if *known {
                    assert_eq!(serde_json::to_value(&sparse.legs[i][j]).unwrap(), old[i][j]);
                }
            }
        }
        scenario.routing = Some(register(Snapshot {
            points,
            profiles: vec![sparse],
            transit: None,
        }));
        assert!(Travel::new(&scenario).is_ok());
        assert_eq!(n, 4);
    }

    #[test]
    fn sparse_road_scope_keeps_every_feasible_order_across_skills_and_windows() {
        fn orders(prefix: Vec<usize>, count: usize, output: &mut Vec<Vec<usize>>) {
            output.push(prefix.clone());
            for job in 0..count {
                if !prefix.contains(&job) {
                    let mut next = prefix.clone();
                    next.push(job);
                    orders(next, count, output);
                }
            }
        }
        let mut permutations = vec![];
        orders(vec![], 4, &mut permutations);
        for seed in 0..24 {
            let mut scenario = crate::import::demo();
            scenario
                .engineers
                .retain(|engineer| engineer.transport != Transport::Public);
            scenario.jobs.truncate(4);
            for (index, job) in scenario.jobs.iter_mut().enumerate() {
                job.duration = 15 + ((seed + index * 7) % 4) as u32 * 15;
                job.window_start = 540 + ((seed * 3 + index * 5) % 8) as u32 * 30;
                job.window_end = job.window_start + ((seed + index) % 3) as u32 * 60;
                job.transport = if seed % 3 == 0 { None } else { job.transport };
            }
            let mut points = vec![];
            for point in scenario
                .jobs
                .iter()
                .map(|job| job.point)
                .chain(scenario.engineers.iter().map(|engineer| engineer.start))
            {
                if point_index(&points, point).is_err() {
                    points.push(point);
                }
            }
            let mut full = Snapshot {
                points: points.clone(),
                profiles: vec![],
                transit: None,
            };
            let mut sparse = full.clone();
            for transport in [Transport::Car, Transport::Walk, Transport::Bicycle] {
                let required =
                    crate::transit_scope::required_for(&scenario, &points, transport).unwrap();
                let legs: Vec<Vec<Option<Leg>>> = (0..points.len())
                    .map(|i| {
                        (0..points.len())
                            .map(|j| {
                                Some(Leg {
                                    minutes: if i == j {
                                        0
                                    } else {
                                        1 + ((i * 7 + j * 3 + seed) % 30) as u32
                                    },
                                    metres: if i == j {
                                        0
                                    } else {
                                        100 + (i * 13 + j * 7) as u32
                                    },
                                    shape: String::new(),
                                })
                            })
                            .collect()
                    })
                    .collect();
                full.profiles.push(Profile {
                    transport,
                    legs: legs.clone(),
                    lower: vec![],
                    coverage: None,
                });
                let coverage: Vec<Vec<bool>> = required
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        row.iter()
                            .enumerate()
                            .map(|(j, need)| i == j || need.is_some())
                            .collect()
                    })
                    .collect();
                let partial = legs
                    .into_iter()
                    .enumerate()
                    .map(|(i, row)| {
                        row.into_iter()
                            .enumerate()
                            .map(|(j, leg)| if coverage[i][j] { leg } else { None })
                            .collect()
                    })
                    .collect();
                sparse.profiles.push(Profile {
                    transport,
                    legs: partial,
                    lower: vec![],
                    coverage: Some(coverage),
                });
            }
            scenario.routing = Some(register(full));
            let full = Travel::new(&scenario).unwrap();
            scenario.routing = Some(register(sparse));
            let sparse = Travel::new(&scenario).unwrap();
            for e in 0..scenario.engineers.len() {
                for order in &permutations {
                    let expected = schedule(&scenario, &full, e, order);
                    let actual = schedule(&scenario, &sparse, e, order);
                    assert_eq!(
                        serde_json::to_value(actual).unwrap(),
                        serde_json::to_value(expected).unwrap(),
                        "seed {seed}, engineer {e}, order {order:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn directed_cache_separates_provider_profile_direction_and_expires() {
        let cache = TestCache::new();
        let points = road_points(2);
        let identity = leg_identity("one/", Transport::Car, points[0], points[1]).unwrap();
        write_leg(&cache.0, identity.clone(), 20, None).unwrap();
        let rounded = Point {
            lat: points[0].lat + 0.0000001,
            lon: 0.0,
        };
        let same = leg_identity("one", Transport::Car, rounded, points[1]).unwrap();
        assert!(matches!(
            read_leg(&cache.0, &same, 20 + LEG_CACHE_TTL - 1, false).unwrap(),
            Some(None)
        ));
        assert!(read_leg(&cache.0, &identity, 20 + LEG_CACHE_TTL, false)
            .unwrap()
            .is_none());
        for other in [
            leg_identity("two", Transport::Car, points[0], points[1]).unwrap(),
            leg_identity("one", Transport::Walk, points[0], points[1]).unwrap(),
            leg_identity("one", Transport::Car, points[1], points[0]).unwrap(),
        ] {
            assert!(read_leg(&cache.0, &other, 21, false).unwrap().is_none());
        }
        let mut refreshed = empty_profile(Transport::Car);
        fill_road_profile(
            &points,
            &mut refreshed,
            &cache.0,
            "one",
            20 + LEG_CACHE_TTL,
            Instant::now() + Duration::from_secs(30),
            None,
            |_, from, to| Ok(vec![vec![test_leg(); to.len()]; from.len()]),
        )
        .unwrap();
        assert!(refreshed.legs[0][1].is_some());
    }
    #[test]
    fn urgent_extension_never_refreshes_frozen_arcs_without_usable_cache() {
        let cache = TestCache::new();
        let points = road_points(3);
        let mut profile = Profile {
            transport: Transport::Car,
            legs: vec![vec![None, test_leg()], vec![None, None]],
            lower: vec![],
            coverage: None,
        };
        let old = serde_json::to_value(&profile.legs).unwrap();
        let new = point_key(points[2]);
        fill_road_profile(
            &points,
            &mut profile,
            &cache.0,
            "changed-provider",
            LEG_CACHE_TTL * 2,
            Instant::now() + Duration::from_secs(30),
            None,
            |_, from, to| {
                for p in from {
                    for q in to {
                        assert!(point_key(*p) == new || point_key(*q) == new);
                    }
                }
                Ok(vec![vec![test_leg(); to.len()]; from.len()])
            },
        )
        .unwrap();
        let preserved: Vec<Vec<Option<Leg>>> =
            profile.legs[..2].iter().map(|r| r[..2].to_vec()).collect();
        assert_eq!(serde_json::to_value(preserved).unwrap(), old);
        assert_eq!(profile.legs[2][2].as_ref().unwrap().minutes, 0);
    }
    #[test]
    fn directed_cache_rejects_corrupt_geometry_identity_and_oversized_files() {
        let cache = TestCache::new();
        let points = road_points(2);
        let identity = leg_identity("one", Transport::Car, points[0], points[1]).unwrap();
        let file = cache
            .0
            .join(format!("leg-{}.json", key(identity.as_bytes())));
        let invalid = Some(Leg {
            metres: 10,
            minutes: 1,
            shape: "_".into(),
        });
        write_leg(&cache.0, identity.clone(), 20, invalid).unwrap();
        assert!(read_leg(&cache.0, &identity, 21, false).is_err());
        let mismatched = CachedLeg {
            identity: "different".into(),
            fetched_at: 20,
            leg: None,
        };
        atomic_write(&file, &serde_json::to_vec(&mismatched).unwrap()).unwrap();
        assert!(read_leg(&cache.0, &identity, 21, false).is_err());
        atomic_write(
            &file,
            &serde_json::to_vec(&json!({"identity": identity, "fetched_at": 20})).unwrap(),
        )
        .unwrap();
        assert!(read_leg(&cache.0, &identity, 21, false).is_err());
        std::fs::File::create(&file)
            .unwrap()
            .set_len(MAX_LEG_FILE as u64 + 1)
            .unwrap();
        assert!(read_leg(&cache.0, &identity, 21, false).is_err());
    }
    pub(crate) fn register(s: Snapshot) -> RoutingRef {
        save(s).unwrap()
    }
}
