//! Static directed MOTIS street routes; never substitute transit or straight lines.
use crate::{
    model::{Point, Result, Transport},
    routing::{agent, decode_shape, point_key, response, Leg},
    routing_work::run_bounded,
};
use serde_json::Value;
use std::time::{Duration, Instant};

pub(crate) const CACHE_SETTINGS: &str = "motis-street-v2/graph-only/max-21600/match-250";
const MAX_DIRECT_TIME: f64 = 21_600.0;

fn mode(profile: Transport) -> Result<&'static str> {
    match profile {
        Transport::Car => Ok("CAR"),
        Transport::Walk => Ok("WALK"),
        Transport::Bicycle => Ok("BIKE"),
        Transport::Public => Err("MOTIS street routing does not support public transport".into()),
    }
}

fn parse_provider(v: &Value) -> Result<()> {
    let config = &v["serverConfig"];
    if config["hasStreetRouting"].as_bool() != Some(true) {
        return Err(
            "MOTIS street routing is unavailable: serverConfig.hasStreetRouting must be true"
                .into(),
        );
    }
    if !config["maxDirectTimeLimit"]
        .as_f64()
        .is_some_and(|limit| limit.is_finite() && limit >= MAX_DIRECT_TIME)
    {
        return Err(
            "MOTIS street routing requires serverConfig.maxDirectTimeLimit >= 21600".into(),
        );
    }
    Ok(())
}

fn request(
    http: &ureq::Agent,
    url: String,
    deadline: Instant,
) -> Result<ureq::RequestBuilder<ureq::typestate::WithoutBody>> {
    check_deadline(deadline)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    // The shared agent's 45-second cap is for ordinary provider calls. Native
    // street searches share the preparation deadline, including response bodies.
    Ok(http
        .get(url)
        .config()
        .timeout_global(Some(remaining))
        .timeout_resolve(Some(Duration::from_secs(5)))
        .timeout_connect(Some(Duration::from_secs(5)))
        .build())
}

pub(crate) fn check_provider(base: &str, deadline: Instant) -> Result<()> {
    let r = request(
        agent(),
        format!("{}/api/v1/map/initial", base.trim_end_matches('/')),
        deadline,
    )?
    .call()
    .map_err(|e| format!("MOTIS street provider unavailable: {e}. No fallback."))?;
    parse_provider(&response(r)?)
}

fn check_cancelled(v: &Value) -> Result<()> {
    if v.get("cancelled")
        .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err("MOTIS returned a cancelled or invalid street itinerary/leg".into());
    }
    Ok(())
}

fn parse_direct(v: &Value, requested_mode: &str) -> Result<Option<Leg>> {
    let direct = v["direct"]
        .as_array()
        .ok_or("MOTIS street response omitted direct routes")?;
    // An explicit empty array alone means no route within the graph/time limit.
    // Malformed routes, mode substitutions and provider errors are not no-path.
    let mut best: Option<(f64, Leg)> = None;
    for itinerary in direct {
        check_cancelled(itinerary)?;
        let legs = itinerary["legs"]
            .as_array()
            .ok_or("MOTIS street itinerary omitted legs")?;
        if legs.len() != 1 {
            return Err("MOTIS street itinerary must contain exactly one direct leg".into());
        }
        let leg = &legs[0];
        check_cancelled(leg)?;
        if leg["mode"].as_str() != Some(requested_mode) {
            return Err(format!(
                "MOTIS street leg does not use requested mode {requested_mode}"
            ));
        }
        // OSR's synthetic near-point route has no OSM references, even though
        // it looks like a valid WALK leg. Endpoint connector steps can omit
        // osmWay, but a real graph path still references at least one node.
        if !leg["steps"].as_array().is_some_and(|steps| {
            steps.iter().any(|step| {
                ["osmWay", "fromOsmNode", "toOsmNode"]
                    .iter()
                    .any(|key| step[*key].as_u64().is_some_and(|id| id > 0))
            })
        }) {
            return Err(
                "MOTIS street leg has no OSM graph references; synthetic straight-line routes are not supported. Rebuild the local router with sh scripts/build-motis.sh."
                    .into(),
            );
        }
        // MOTIS timestamps truncate to minutes. Only the routed leg's duration
        // has the seconds used by its street router; keep its geometry and cost.
        let seconds = leg["duration"]
            .as_f64()
            .ok_or("MOTIS street leg omitted duration")?;
        let distance = leg["distance"]
            .as_f64()
            .ok_or("MOTIS street leg omitted distance")?;
        if !seconds.is_finite()
            || !(0.0..=MAX_DIRECT_TIME).contains(&seconds)
            || !distance.is_finite()
            || !(0.0..=20_000_000.0).contains(&distance)
        {
            return Err("Invalid MOTIS street duration/distance".into());
        }
        let geometry = &leg["legGeometry"];
        let shape = geometry["points"]
            .as_str()
            .ok_or("MOTIS street leg omitted geometry")?;
        if shape.len() > 200_000
            || geometry
                .get("precision")
                .is_some_and(|p| p.as_u64() != Some(6))
        {
            return Err("Invalid MOTIS street polyline6 geometry".into());
        }
        let points = decode_shape(shape)?;
        if points.len() < 2
            || geometry
                .get("length")
                .is_some_and(|n| n.as_u64() != Some(points.len() as u64))
        {
            return Err("Incomplete MOTIS street geometry".into());
        }
        // Match road snapshots' conservative rounding for truncated seconds.
        // Positive travel must never become a zero-minute non-diagonal arc.
        let minutes = ((seconds + if distance > 0.0 { 1.0 } else { 0.0 }) / 60.0).ceil() as u32;
        let metres = distance.ceil() as u32;
        if best
            .as_ref()
            .is_none_or(|(time, old)| seconds < *time || (seconds == *time && metres < old.metres))
        {
            best = Some((
                seconds,
                Leg {
                    minutes,
                    metres,
                    shape: shape.into(),
                },
            ));
        }
    }
    Ok(best.map(|(_, leg)| leg))
}

fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err("MOTIS street preparation deadline exceeded".into());
    }
    Ok(())
}

pub(crate) fn matrix(
    base: &str,
    profile: Transport,
    sources: &[Point],
    targets: &[Point],
    deadline: Instant,
) -> Result<Vec<Vec<Option<Leg>>>> {
    let requested_mode = mode(profile)?;
    if sources.is_empty() || targets.is_empty() || sources.len() > 10 || targets.len() > 10 {
        return Err("Road matrix requests require 1–10 sources and targets".into());
    }
    check_deadline(deadline)?;
    let base = base.trim_end_matches('/');
    let mut rows = vec![vec![None; targets.len()]; sources.len()];
    run_bounded(
        sources.len() * targets.len(),
        4,
        |index| {
            check_deadline(deadline)?;
            let from = sources[index / targets.len()];
            let to = targets[index % targets.len()];
            if point_key(from) == point_key(to) {
                return Ok(Some(Leg {
                    minutes: 0,
                    metres: 0,
                    shape: String::new(),
                }));
            }
            let r = request(agent(), format!("{base}/api/v6/plan"), deadline)?
                .query("fromPlace", format!("{},{}", from.lat, from.lon))
                .query("toPlace", format!("{},{}", to.lat, to.lon))
                .query("transitModes", "")
                .query("directModes", requested_mode)
                .query("detailedLegs", "true")
                .query("maxDirectTime", "21600")
                .query("maxMatchingDistance", "250")
                .call()
                .map_err(|e| format!("MOTIS {requested_mode} street route {},{} -> {},{} unavailable: {e}. No fallback.", from.lat, from.lon, to.lat, to.lon))?;
            parse_direct(&response(r)?, requested_mode)
        },
        |index, leg| {
            // Returning an error here cancels queued work before dispatching more.
            check_deadline(deadline)?;
            rows[index / targets.len()][index % targets.len()] = leg;
            Ok(())
        },
    )?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn direct(mode: &str, duration: f64, distance: f64, shape: &str) -> Value {
        json!({"direct": [{"duration": duration, "legs": [{
            "mode": mode, "duration": duration, "distance": distance,
            "startTime": "2026-09-27T12:00:00Z", "endTime": "2026-09-27T12:00:00Z",
            "steps": [{"osmWay": 123}],
            "legGeometry": {"points": shape, "precision": 6, "length": 2}
        }]}]})
    }

    #[test]
    fn only_explicit_empty_direct_routes_are_unreachable() {
        assert!(parse_direct(&json!({"direct": []}), "CAR")
            .unwrap()
            .is_none());
        for value in [json!({}), json!({"direct": null}), json!({"direct": [{}]})] {
            assert!(parse_direct(&value, "CAR").is_err());
        }
    }

    #[test]
    fn rejects_mode_substitution_transfers_and_cancellation() {
        let valid = direct("CAR", 60.0, 100.0, "??AA");
        assert!(parse_direct(&direct("BUS", 60.0, 100.0, "??AA"), "CAR").is_err());
        let mut missing_mode = valid.clone();
        missing_mode["direct"][0]["legs"][0]
            .as_object_mut()
            .unwrap()
            .remove("mode");
        assert!(parse_direct(&missing_mode, "CAR").is_err());
        let mut transfers = valid.clone();
        transfers["direct"][0]["legs"]
            .as_array_mut()
            .unwrap()
            .push(valid["direct"][0]["legs"][0].clone());
        assert!(parse_direct(&transfers, "CAR").is_err());
        let mut cancelled = valid.clone();
        cancelled["direct"][0]["cancelled"] = json!(true);
        assert!(parse_direct(&cancelled, "CAR").is_err());
        let mut cancelled_leg = valid;
        cancelled_leg["direct"][0]["legs"][0]["cancelled"] = json!(true);
        assert!(parse_direct(&cancelled_leg, "CAR").is_err());
    }

    #[test]
    fn geometry_must_be_complete_polyline6() {
        let valid = direct("CAR", 60.0, 100.0, "??AA");
        for geometry in [
            Value::Null,
            json!({"points": ""}),
            json!({"points": "?"}),
            json!({"points": "??"}),
            json!({"points": "??AA", "precision": 5}),
            json!({"points": "??AA", "length": 3}),
        ] {
            let mut value = valid.clone();
            value["direct"][0]["legs"][0]["legGeometry"] = geometry;
            assert!(parse_direct(&value, "CAR").is_err());
        }
    }

    #[test]
    fn directed_routes_keep_their_own_costs_shapes_and_second_precision() {
        let forward = parse_direct(&direct("CAR", 60.0, 100.1, "??AA"), "CAR")
            .unwrap()
            .unwrap();
        let reverse = parse_direct(&direct("CAR", 119.0, 125.0, "AA@@"), "CAR")
            .unwrap()
            .unwrap();
        assert_eq!(
            (forward.minutes, forward.metres, forward.shape.as_str()),
            (2, 101, "??AA")
        );
        assert_eq!(
            (reverse.minutes, reverse.metres, reverse.shape.as_str()),
            (2, 125, "AA@@")
        );
        let tiny = parse_direct(&direct("WALK", 0.0, 0.1, "??AA"), "WALK")
            .unwrap()
            .unwrap();
        assert_eq!((tiny.minutes, tiny.metres), (1, 1));
    }

    #[test]
    fn rejects_unrouted_short_walk_even_when_walking_was_requested() {
        // Actual MOTIS 2.11.3 output for Восток points 3.8 m apart. OSR
        // try_direct fabricates this segment before searching any graph.
        let mut value = direct("WALK", 60.0, 3.0, "{lbjiBopb|fAf@bB");
        value["direct"][0]["legs"][0]["steps"] = json!([{
            "distance": 3.0,
            "polyline": {"points": "{lbjiBopb|fAf@bB", "precision": 6, "length": 2}
        }]);
        assert!(parse_direct(&value, "WALK").is_err());
    }

    #[test]
    fn short_graph_route_can_connect_through_an_osm_node_without_a_way_id() {
        let mut value = direct("CAR", 5.0, 46.0, "??AA");
        value["direct"][0]["legs"][0]["steps"] = json!([
            {"toOsmNode": 3007346893_u64},
            {"fromOsmNode": 3007346893_u64}
        ]);
        let leg = parse_direct(&value, "CAR").unwrap().unwrap();
        assert_eq!(
            (leg.minutes, leg.metres, leg.shape.as_str()),
            (1, 46, "??AA")
        );
    }

    #[test]
    fn alternatives_never_mix_cost_and_geometry() {
        let mut value = direct("BIKE", 120.0, 100.0, "??AA");
        value["direct"]
            .as_array_mut()
            .unwrap()
            .push(direct("BIKE", 60.0, 200.0, "AA@@")["direct"][0].clone());
        let leg = parse_direct(&value, "BIKE").unwrap().unwrap();
        assert_eq!(
            (leg.minutes, leg.metres, leg.shape.as_str()),
            (2, 200, "AA@@")
        );
    }

    #[test]
    fn rejects_missing_negative_and_excessive_costs() {
        for (field, invalid) in [
            ("duration", Value::Null),
            ("duration", json!(-1)),
            ("duration", json!(21601)),
            ("distance", Value::Null),
            ("distance", json!(-1)),
            ("distance", json!(20_000_001)),
        ] {
            let mut value = direct("CAR", 60.0, 100.0, "??AA");
            value["direct"][0]["legs"][0][field] = invalid;
            assert!(parse_direct(&value, "CAR").is_err());
        }
    }

    #[test]
    fn street_disabled_and_insufficient_limit_are_configuration_errors() {
        for config in [
            json!({}),
            json!({"hasStreetRouting": false, "maxDirectTimeLimit": 21600}),
            json!({"hasStreetRouting": true, "maxDirectTimeLimit": 21599}),
        ] {
            assert!(parse_provider(&json!({"serverConfig": config})).is_err());
        }
        assert!(parse_provider(
            &json!({"serverConfig": {"hasStreetRouting": true, "maxDirectTimeLimit": 21600}})
        )
        .is_ok());
    }

    fn delayed_server(
        delay_headers: bool,
        delay: Duration,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let worker = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= until {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&socket);
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let body = direct("CAR", 125.0, 900.0, "??AA").to_string();
            if delay_headers {
                std::thread::sleep(delay);
            }
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if !delay_headers {
                std::thread::sleep(delay);
            }
            let _ = socket.write_all(body.as_bytes());
        });
        (url, worker)
    }

    #[test]
    fn street_deadline_overrides_short_agent_timeout_for_headers_and_body() {
        let http: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(20)))
            .build()
            .into();
        for delay_headers in [true, false] {
            let (url, server) = delayed_server(delay_headers, Duration::from_millis(100));
            let r = request(&http, url, Instant::now() + Duration::from_secs(5))
                .unwrap()
                .call()
                .unwrap();
            let leg = parse_direct(&response(r).unwrap(), "CAR").unwrap().unwrap();
            server.join().unwrap();
            assert_eq!(
                (leg.minutes, leg.metres, leg.shape.as_str()),
                (3, 900, "??AA")
            );
        }
    }

    #[test]
    fn preparation_deadline_still_limits_headers_and_response_body() {
        for delay_headers in [true, false] {
            let (url, server) = delayed_server(delay_headers, Duration::from_millis(750));
            let result = request(agent(), url, Instant::now() + Duration::from_millis(250))
                .unwrap()
                .call();
            if delay_headers {
                assert!(matches!(result, Err(ureq::Error::Timeout(_))));
            } else {
                let r = result.expect("Headers should arrive before the body stall");
                assert!(response(r).is_err());
            }
            server.join().unwrap();
        }
    }
}
