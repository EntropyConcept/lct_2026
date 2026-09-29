//! Frozen, departure-aware MOTIS journeys. No network access while solving.
use crate::{
    model::*,
    routing::{self, Snapshot},
    transit_scope::{self, Coverage},
    transit_snapshot::{Journey, Timetable, TransitLeg},
};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub fn midnight(date: &str) -> Result<DateTime<FixedOffset>> {
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "Transit date must be YYYY-MM-DD (Moscow, UTC+03:00)")?;
    if day.format("%Y-%m-%d").to_string() != date {
        return Err("Transit date must be YYYY-MM-DD".into());
    }
    FixedOffset::east_opt(3 * 3600)
        .unwrap()
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .ok_or_else(|| "Invalid transit service date".into())
}

fn encode_shape(points: &[Point]) -> String {
    let mut shape = String::new();
    let mut previous = [0i64; 2];
    for p in points {
        for (axis, value) in [(p.lat * 1e6).round() as i64, (p.lon * 1e6).round() as i64]
            .into_iter()
            .enumerate()
        {
            let delta = value - previous[axis];
            previous[axis] = value;
            let mut encoded = ((delta << 1) ^ (delta >> 63)) as u64;
            while encoded >= 32 {
                shape.push(char::from((encoded as u8 & 31) + 95));
                encoded >>= 5;
            }
            shape.push(char::from(encoded as u8 + 63));
        }
    }
    shape
}

fn timestamp(value: &Value, origin: DateTime<FixedOffset>) -> Result<i64> {
    DateTime::parse_from_rfc3339(value.as_str().ok_or("Missing transit timestamp")?)
        .map(|t| t.signed_duration_since(origin).num_seconds())
        .map_err(|_| "Invalid transit timestamp".into())
}

fn parse_journeys(v: &Value, origin: DateTime<FixedOffset>) -> Result<Vec<Journey>> {
    let itineraries = v["itineraries"]
        .as_array()
        .ok_or("MOTIS response omitted itineraries")?;
    if itineraries.len() > 10_000 {
        return Err("Too many transit itineraries".into());
    }
    let mut paths: HashMap<Vec<&str>, Arc<str>> = HashMap::new();
    let mut geometries = HashMap::<&str, (Vec<Point>, Option<u32>)>::new();
    let mut points = Vec::new();
    let mut result = Vec::with_capacity(itineraries.len());
    for itinerary in itineraries {
        let departure = timestamp(&itinerary["startTime"], origin)?;
        let arrival = timestamp(&itinerary["endTime"], origin)?;
        if arrival < departure {
            return Err("Transit arrival precedes departure".into());
        }
        // The dispatcher models one Moscow calendar day; do not wrap midnight.
        if departure < 0 || arrival > 86_400 {
            continue;
        }
        let legs = itinerary["legs"]
            .as_array()
            .ok_or("Transit journey omitted legs")?;
        if legs.is_empty() || legs.len() > 128 {
            return Err("Invalid transit journey leg count".into());
        }
        points.clear();
        let mut leg_shapes = Vec::with_capacity(legs.len());
        let mut description = Vec::new();
        let mut transit = false;
        let mut previous_end = departure;
        let mut distance = 0u32;
        for (index, leg) in legs.iter().enumerate() {
            let start = timestamp(&leg["startTime"], origin)?;
            let end = timestamp(&leg["endTime"], origin)?;
            if start < previous_end
                || end < start
                || end > arrival
                || (index == 0 && start != departure)
            {
                return Err("Invalid transit transfer/leg times".into());
            }
            previous_end = end;
            let mode = leg["mode"].as_str().ok_or("Transit leg omitted mode")?;
            let is_transit = matches!(
                mode,
                "TRAM"
                    | "SUBWAY"
                    | "SUBURBAN"
                    | "METRO"
                    | "BUS"
                    | "FERRY"
                    | "CABLE_CAR"
                    | "FUNICULAR"
                    | "AERIAL_LIFT"
                    | "MONORAIL"
                    | "RAIL"
                    | "REGIONAL_RAIL"
                    | "HIGHSPEED_RAIL"
                    | "NIGHT_RAIL"
                    | "LONG_DISTANCE"
                    | "COACH"
                    | "OTHER"
            );
            if !is_transit && mode != "WALK" {
                return Err(format!("Unsupported public journey mode: {mode}"));
            }
            transit |= is_transit;
            let shape = leg["legGeometry"]["points"]
                .as_str()
                .ok_or("MOTIS omitted journey geometry")?;
            if shape.len() > 200_000 {
                return Err("Transit leg geometry exceeds limit".into());
            }
            leg_shapes.push(shape);
            let (decoded, cached_distance) = match geometries.entry(shape) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((routing::decode_shape(shape)?, None))
                }
            };
            if decoded.len() < 2 && end > start {
                return Err("MOTIS omitted routed walking/transit geometry".into());
            }
            // MOTIS only guarantees a distance field for non-transit legs. Transit
            // distance is the sum along the returned route geometry, not the OD chord.
            let leg_distance = if !is_transit {
                let d = leg["distance"]
                    .as_f64()
                    .ok_or("MOTIS walking leg omitted distance")?;
                if !d.is_finite() || !(0.0..=20_000_000.0).contains(&d) {
                    return Err("Invalid walking distance".into());
                }
                d.ceil() as u32
            } else if let Some(distance) = *cached_distance {
                distance
            } else {
                let distance = decoded
                    .windows(2)
                    .try_fold(0u32, |sum, p| sum.checked_add(metres(p[0], p[1])))
                    .ok_or("Transit distance overflow")?;
                *cached_distance = Some(distance);
                distance
            };
            distance = distance
                .checked_add(leg_distance)
                .filter(|&d| d <= 20_000_000)
                .ok_or("Transit distance exceeds limit")?;
            if let (Some(a), Some(b)) = (points.last().copied(), decoded.first().copied()) {
                if metres(a, b) > 100 {
                    return Err("Disconnected MOTIS journey geometry".into());
                }
            }
            points.extend_from_slice(decoded);
            if points.len() > 100_000 {
                return Err("Transit geometry exceeds point limit".into());
            }
            let name = leg["displayName"]
                .as_str()
                .or_else(|| leg["routeShortName"].as_str())
                .unwrap_or(mode);
            let from = leg["from"]["name"].as_str().unwrap_or("");
            let to = leg["to"]["name"].as_str().unwrap_or("");
            description.push(format!(
                "{} {} {}–{}: {} → {}",
                mode,
                name,
                clock((start / 60) as u32),
                clock(((end + 59) / 60) as u32),
                from,
                to
            ));
        }
        if previous_end != arrival {
            return Err("Transit itinerary end disagrees with its legs".into());
        }
        if !transit {
            return Err("MOTIS returned walking-only travel as a transit itinerary; no substitution allowed".into());
        }
        let departure = (departure / 60) as u32;
        let arrival = ((arrival + 59) / 60) as u32;
        let shape = if let Some(shared) = paths.get(&leg_shapes) {
            Arc::clone(shared)
        } else {
            let shared: Arc<str> = encode_shape(&points).into();
            paths.insert(leg_shapes, Arc::clone(&shared));
            shared
        };
        let description = description.join("; ");
        if shape.len() > 200_000 || description.len() > 8192 || points.len() < 2 {
            return Err("Transit journey geometry/details exceed limit".into());
        }
        result.push(Journey {
            departure,
            arrival,
            leg: TransitLeg {
                minutes: arrival - departure,
                metres: distance,
                shape,
            },
            description,
        });
    }
    result.sort_by_key(|j| (j.departure, j.arrival, j.leg.metres));
    result.dedup_by(|a, b| {
        a.departure == b.departure
            && a.arrival == b.arrival
            && a.leg.metres == b.leg.metres
            && a.leg.shape == b.leg.shape
    });
    Ok(result)
}

#[derive(Serialize, Deserialize)]
struct CachedPair {
    version: u32,
    identity: String,
    fetched_at: u64,
    timetable: Timetable,
}

fn cache_age(fetched_at: u64, now: u64) -> bool {
    now.checked_sub(fetched_at).is_some_and(|age| age < 86_400)
}

fn fetch(
    provider: &str,
    date: &str,
    from: Point,
    to: Point,
    coverage: Coverage,
) -> Result<Vec<Journey>> {
    let origin = midnight(date)?;
    let query = json!([
        provider,
        date,
        routing::point_key(from),
        routing::point_key(to)
    ]);
    let full_identity = format!("motis-v6-day-v1/{query}");
    let full_id = routing::key(full_identity.as_bytes());
    let identity = format!(
        "motis-v6-window-v1/{query}/{}/{}",
        coverage.start, coverage.end
    );
    let file = routing::directory().join(format!(
        "transit-v3-{full_id}-{}-{}.json",
        coverage.start, coverage.end
    ));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let read_cache = |file: &Path, identity: &str, version: u32| -> Result<Option<CachedPair>> {
        if !file.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let cached: CachedPair = serde_json::from_slice(&routing::read_bounded(file, 32_000_000)?)
            .map_err(|e| format!("Corrupt transit cache: {e}"))?;
        if cached.version != version
            || cached.identity != identity
            || cached.timetable.date != date
            || cached.timetable.provider != provider
            || cached.timetable.journeys.len() != 2
            || cached.timetable.coverage[0][1].is_none_or(|c| !c.contains(coverage))
        {
            return Err("Transit cache identity/coverage mismatch".into());
        }
        Ok(cache_age(cached.fetched_at, now).then_some(cached))
    };
    if let Some(mut cached) = read_cache(&file, &identity, 3)? {
        return cached.timetable.journeys[0][1]
            .take()
            .ok_or("Transit cache omitted journeys".into());
    }
    let store = |mut journeys: Vec<Journey>, fetched_at: u64| -> Result<Vec<Journey>> {
        // Do not filter by arrival: expansion must retain every previously queried
        // departure, including journeys that used to arrive after the job window.
        journeys.retain(|j| coverage.start <= j.departure && j.departure <= coverage.end);
        let mut timetable = Timetable::new(date.into(), provider.into());
        let journeys = timetable.intern(journeys)?;
        timetable.journeys = vec![vec![None, Some(journeys)], vec![None, None]];
        timetable.coverage = vec![vec![None, Some(coverage)], vec![None, None]];
        let mut cached = CachedPair {
            version: 3,
            identity: identity.clone(),
            fetched_at,
            timetable,
        };
        routing::atomic_write(
            &file,
            &serde_json::to_vec(&cached).map_err(|e| e.to_string())?,
        )?;
        Ok(cached.timetable.journeys[0][1].take().unwrap())
    };
    // Reuse the full-day data already downloaded by earlier releases. Retain it
    // for other departure intervals; bounded derivatives keep the original TTL.
    let full_file = routing::directory().join(format!("transit-v2-{full_id}.json"));
    if let Some(mut cached) = read_cache(&full_file, &full_identity, 2)? {
        let journeys = cached.timetable.journeys[0][1]
            .take()
            .ok_or("Transit cache omitted journeys")?;
        return store(journeys, cached.fetched_at);
    }
    let legacy = routing::directory().join(format!("transit-{full_id}.json"));
    if let Ok(modified) = legacy.metadata().and_then(|m| m.modified()) {
        let fetched_at = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        if cache_age(fetched_at, now) {
            let value = serde_json::from_slice(&routing::read_bounded(&legacy, 32_000_000)?)
                .map_err(|e| format!("Corrupt transit cache: {e}"))?;
            return store(parse_journeys(&value, origin)?, fetched_at);
        }
    }
    let response = routing::agent()
        .get(format!("{}/api/v6/plan", provider.trim_end_matches('/')))
        .query("fromPlace", format!("{},{}", from.lat, from.lon))
        .query("toPlace", format!("{},{}", to.lat, to.lon))
        .query(
            "time",
            (origin + chrono::Duration::minutes(coverage.start.into())).to_rfc3339(),
        )
        .query(
            "searchWindow",
            ((coverage.end - coverage.start + 1) * 60)
                .min(86_400)
                .to_string(),
        )
        .query("numItineraries", "1")
        .query("timetableView", "true")
        .query("arriveBy", "false")
        .query("transitModes", "TRANSIT")
        .query("directModes", "")
        .query("preTransitModes", "WALK")
        .query("postTransitModes", "WALK")
        .query("useRoutedTransfers", "true")
        .query("detailedTransfers", "true")
        .query("detailedLegs", "true")
        .call()
        .map_err(|e| format!("MOTIS unavailable: {e}. No walking/speed fallback."))?;
    store(parse_journeys(&routing::response(response)?, origin)?, now)
}

pub fn configured_provider() -> Result<String> {
    let url = std::env::var("DISPATCH_MOTIS_URL").map_err(|_| "Public transport requires DISPATCH_MOTIS_URL pointing to a MOTIS instance with Moscow GTFS and OSM data; no walking/speed substitution is made")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 2048 {
        return Err("DISPATCH_MOTIS_URL must be an HTTP(S) server base URL".into());
    }
    Ok(url.trim_end_matches('/').to_owned())
}

pub fn prepare(s: &Scenario, snapshot: &mut Snapshot, deadline: Instant) -> Result<()> {
    if !s.engineers.iter().any(|e| e.transport == Transport::Public) {
        snapshot.transit = None;
        return Ok(());
    }
    let date = s
        .transit_date
        .as_deref()
        .ok_or("Select the public transport service date (Moscow UTC+03:00)")?;
    midnight(date)?;
    let n = snapshot.points.len();
    let mut timetable = match snapshot.transit.take() {
        Some(t) if t.date == date => t,
        Some(_) => {
            return Err(
                "Transit date changed: discard the old snapshot and prepare a new day".into(),
            )
        }
        None => Timetable::new(date.into(), configured_provider()?),
    };
    for row in &mut timetable.journeys {
        row.resize(n, None);
    }
    timetable.journeys.resize(n, vec![None; n]);
    for row in &mut timetable.coverage {
        row.resize(n, None);
    }
    timetable.coverage.resize(n, vec![None; n]);
    let required = transit_scope::required(s, &snapshot.points)?;
    let mut pending = Vec::new();
    for (from, row) in required.iter().enumerate() {
        for (to, &requested) in row.iter().enumerate() {
            let Some(requested) = requested else { continue };
            match timetable.coverage[from][to] {
                Some(covered) => {
                    pending.extend(
                        covered
                            .missing(requested)
                            .into_iter()
                            .map(|range| (from, to, range)),
                    );
                }
                None => pending.push((from, to, requested)),
            }
        }
    }
    let provider = timetable.provider.clone();
    crate::routing_work::run_bounded(
        pending.len(),
        crate::routing_work::configured_workers("DISPATCH_TRANSIT_WORKERS")?,
        |index| {
            if Instant::now() >= deadline {
                return Err("Transit preparation deadline exceeded; cached pairs are retained. Retry with the same service date.".into());
            }
            let (from, to, range) = pending[index];
            fetch(
                &provider,
                date,
                snapshot.points[from],
                snapshot.points[to],
                range,
            )
        },
        |index, journeys| {
            let (from, to, range) = pending[index];
            let journeys = timetable.intern(journeys)?;
            let cell = timetable.journeys[from][to].get_or_insert_with(Vec::new);
            cell.extend(journeys);
            cell.sort_by_key(|j| (j.departure, j.arrival, j.leg.metres));
            timetable.coverage[from][to] =
                Some(timetable.coverage[from][to].map_or(range, |old| old.union(range)));
            Ok(())
        },
    )?;
    snapshot
        .profiles
        .retain(|p| p.transport != Transport::Public);
    snapshot.profiles.push(timetable.profile());
    snapshot.transit = Some(timetable);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn departures_waiting_and_overtaking_are_time_dependent() {
        let journey = |departure, arrival, distance| Journey {
            departure,
            arrival,
            leg: TransitLeg {
                minutes: arrival - departure,
                metres: distance,
                shape: "??AA".into(),
            },
            description: "BUS 1".into(),
        };
        let mut t = Timetable::new("2026-09-27".into(), "http://localhost".into());
        t.journeys = vec![
            vec![
                None,
                Some(vec![
                    journey(540, 600, 100),
                    journey(550, 580, 200),
                    journey(600, 630, 100),
                ]),
            ],
            vec![Some(vec![]), None],
        ];
        t.validate(2).unwrap();
        assert_eq!(t.journey(0, 1, 540).unwrap().arrival, 580);
        assert_eq!(t.journey(0, 1, 550).unwrap().arrival, 580);
        assert_eq!(t.journey(0, 1, 551).unwrap().arrival, 630);
        assert!(t.journey(0, 1, 601).is_none());
        assert!(t.journey(1, 0, 540).is_none());
    }
    #[test]
    fn calendar_validation_rejects_invalid_dates() {
        assert!(midnight("2026-02-29").is_err());
        assert!(midnight("2028-02-29").is_ok());
        assert!(midnight("2026-9-27").is_err());
    }
}
