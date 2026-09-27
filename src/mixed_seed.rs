use crate::model::{make_plan, validate_plan, Plan, Result, Scenario, Transport, Travel};
use std::time::Instant;

/// Seed the mixed search without restricting its engineers, routes, or proof scope.
pub(crate) fn road_only(s: &Scenario, seconds: f64) -> Result<Option<Plan>> {
    if !seconds.is_finite() || seconds < 0.01 {
        return Ok(None);
    }
    let started = Instant::now();
    let road_count = s
        .engineers
        .iter()
        .filter(|e| e.transport != Transport::Public)
        .count();
    if road_count == 0 || road_count == s.engineers.len() {
        return Ok(None);
    }

    // Keep every job: public-required work remains unassigned in this seed only.
    let mut road = s.clone();
    road.engineers.retain(|e| e.transport != Transport::Public);
    let travel = Travel::new(s)?;
    let remaining = seconds - started.elapsed().as_secs_f64();
    if remaining < 0.01 {
        return Ok(None);
    }
    // No Public engineers remain, so optimize cannot recurse into timetable search.
    // Subset bounds and proof flags say nothing about the full mixed problem.
    let (seed, _, _) = crate::sat::optimize(&road, remaining.min(300.0))?;
    let mut orders = vec![vec![]; s.engineers.len()];
    for route in &seed.routes {
        let e = s
            .engineers
            .iter()
            .position(|e| e.id == route.engineer_id)
            .ok_or("Unknown engineer in road-only seed")?;
        for stop in &route.stops {
            let j = s
                .jobs
                .iter()
                .position(|j| j.id == stop.job_id)
                .ok_or("Unknown job in road-only seed")?;
            orders[e].push(j);
        }
    }
    // Rebuild schedules and metrics in the original fleet, including public
    // engineers already counted as used by earlier work in a replanning session.
    let plan = make_plan(s, &travel, &orders)?;
    validate_plan(s, &plan)?;
    Ok(Some(plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Engineer, Job, Point, Skill};

    fn mixed() -> Scenario {
        let point = Point {
            lat: 55.0,
            lon: 37.0,
        };
        let job = |id: &str, transport| Job {
            id: id.into(),
            address: String::new(),
            point,
            duration: 10,
            work_type: None,
            geocode_match: None,
            window_start: 540,
            window_end: 600,
            skill: Skill::Local,
            transport: Some(transport),
            urgent: false,
        };
        let engineer = |id: &str, transport, already_used| Engineer {
            id: id.into(),
            name: id.into(),
            start: point,
            shift_start: 540,
            shift_end: 660,
            skills: vec![Skill::Local],
            transport,
            already_used,
        };
        Scenario {
            name: "Mixed seed".into(),
            jobs: vec![
                job("bike-job", Transport::Bicycle),
                job("car-job", Transport::Car),
            ],
            engineers: vec![
                engineer("public-used", Transport::Public, true),
                engineer("car", Transport::Car, true),
                engineer("public-unused", Transport::Public, false),
                engineer("bike", Transport::Bicycle, false),
            ],
            routing: None,
            transit_date: None,
            notes: vec![],
        }
    }

    #[test]
    fn interleaved_engineers_keep_job_ids_and_daily_staff_accounting() {
        let s = mixed();
        let plan = road_only(&s, 1.0).unwrap().unwrap();
        validate_plan(&s, &plan).unwrap();
        let routes: Vec<_> = plan
            .routes
            .iter()
            .map(|r| {
                (
                    r.engineer_id.as_str(),
                    r.stops
                        .iter()
                        .map(|stop| stop.job_id.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            routes,
            vec![
                ("public-used", vec![]),
                ("car", vec!["car-job"]),
                ("public-unused", vec![]),
                ("bike", vec!["bike-job"]),
            ]
        );
        assert_eq!(plan.metrics.key(), [0, 0, 3, 0]);
    }

    #[test]
    fn public_required_work_is_retained_as_unassigned() {
        let mut s = mixed();
        let mut public_job = s.jobs[0].clone();
        public_job.id = "public-job".into();
        public_job.transport = Some(Transport::Public);
        public_job.urgent = true;
        s.jobs.insert(1, public_job);
        let plan = road_only(&s, 1.0).unwrap().unwrap();
        validate_plan(&s, &plan).unwrap();
        assert_eq!(
            plan.unassigned
                .iter()
                .map(|j| j.job_id.as_str())
                .collect::<Vec<_>>(),
            vec!["public-job"]
        );
        assert_eq!(plan.metrics.key(), [1, 1, 3, 0]);
    }

    #[test]
    fn unavailable_public_journeys_do_not_discard_road_routes() {
        use crate::routing::{test_snapshot, Leg, Profile, Snapshot};
        use crate::transit_snapshot::Timetable;

        let mut s = mixed();
        s.engineers.retain(|e| e.transport != Transport::Bicycle);
        let base = s.engineers[0].start;
        let site = Point {
            lat: base.lat + 0.001,
            lon: base.lon,
        };
        for job in &mut s.jobs {
            job.point = site;
            job.transport = None;
        }
        let mut timetable = Timetable::new("2026-09-27".into(), "http://localhost".into());
        // Complete coverage with no available public journeys, not missing data.
        timetable.journeys = vec![vec![Some(vec![]); 2]; 2];
        timetable.validate(2).unwrap();
        s.transit_date = Some(timetable.date.clone());
        let leg = |metres, minutes| {
            Some(Leg {
                metres,
                minutes,
                shape: "??AA".into(),
            })
        };
        s.routing = Some(test_snapshot(Snapshot {
            points: vec![base, site],
            profiles: vec![
                Profile {
                    transport: Transport::Car,
                    legs: vec![vec![leg(0, 0), leg(500, 1)], vec![leg(500, 1), leg(0, 0)]],
                    lower: vec![],
                },
                timetable.profile(),
            ],
            transit: Some(timetable),
        }));
        let travel = Travel::new(&s).unwrap();
        assert!(crate::model::schedule(&s, &travel, 0, &[0]).is_none());
        let plan = road_only(&s, 1.0).unwrap().unwrap();
        validate_plan(&s, &plan).unwrap();
        assert_eq!(plan.metrics.key(), [0, 0, 2, 500]);
        let car = plan.routes.iter().find(|r| r.engineer_id == "car").unwrap();
        let mut assigned: Vec<_> = car.stops.iter().map(|stop| stop.job_id.as_str()).collect();
        assigned.sort_unstable();
        assert_eq!(assigned, vec!["bike-job", "car-job"]);
    }

    #[test]
    fn skips_missing_mixed_subset_and_insufficient_budget() {
        let mut s = mixed();
        for seconds in [0.0, -1.0, 0.001, f64::NAN, f64::INFINITY] {
            assert!(road_only(&s, seconds).unwrap().is_none());
        }
        s.engineers.retain(|e| e.transport != Transport::Public);
        assert!(road_only(&s, 1.0).unwrap().is_none());
        for e in &mut s.engineers {
            e.transport = Transport::Public;
        }
        assert!(road_only(&s, 1.0).unwrap().is_none());
        s.engineers.clear();
        assert!(road_only(&s, 1.0).unwrap().is_none());
    }
}
