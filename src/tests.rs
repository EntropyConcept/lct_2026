use crate::{app, import, model::*, sat};

fn tiny() -> Scenario {
    let mut s = import::demo();
    s.jobs.truncate(4);
    s.engineers.truncate(2);
    for e in &mut s.engineers {
        e.skills = vec![Skill::Local, Skill::Connection, Skill::Emergency];
        e.transport = Transport::Car;
    }
    s
}

// Independent exhaustive enumeration of every partial assignment and route order.
fn brute(s: &Scenario) -> [u32; 4] {
    fn visit(s: &Scenario, t: &Travel, j: usize, orders: &mut [Vec<usize>], best: &mut [u32; 4]) {
        if j == s.jobs.len() {
            if orders
                .iter()
                .enumerate()
                .all(|(e, route)| schedule(s, t, e, route).is_some())
            {
                *best = (*best).min(make_plan(s, t, orders).unwrap().metrics.key());
            }
            return;
        }
        visit(s, t, j + 1, orders, best);
        for e in 0..orders.len() {
            for pos in 0..=orders[e].len() {
                orders[e].insert(pos, j);
                // A later inserted stop can make a nonmetric route feasible.
                visit(s, t, j + 1, orders, best);
                orders[e].remove(pos);
            }
        }
    }
    let t = Travel::new(s).unwrap();
    let mut best = [u32::MAX; 4];
    visit(s, &t, 0, &mut vec![vec![]; s.engineers.len()], &mut best);
    best
}

#[test]
fn sat_matches_exhaustive_routes() {
    let mut seed = 12345u32;
    let mut rand = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed
    };
    for case in 0..120 {
        let mut s = tiny();
        if case % 6 == 0 {
            let mut extra = s.jobs[0].clone();
            extra.id = "extra".into();
            s.jobs.push(extra);
        }
        for e in &mut s.engineers {
            e.shift_start = 540 + rand() % 25;
            e.shift_end = 690 + rand() % 50;
            e.start.lat += (rand() % 30) as f64 / 10000.0;
            e.transport = if rand() % 2 == 0 {
                Transport::Car
            } else {
                Transport::Bicycle
            };
        }
        for j in &mut s.jobs {
            j.point = Point {
                lat: 55.75 + (rand() % 20) as f64 / 10000.0,
                lon: 37.62,
            };
            j.duration = 15 + rand() % 46;
            j.window_start = 550 + rand() % 120;
            j.window_end = j.window_start + rand() % 40;
            j.urgent = rand() % 3 == 0;
            j.transport = if rand() % 3 == 0 {
                Some(Transport::Car)
            } else {
                None
            };
        }
        if case % 4 == 0 {
            s.engineers[0].skills = vec![Skill::Emergency];
        }
        if case % 5 == 0 {
            s.engineers[1].already_used = true;
        }
        let expected = brute(&s);
        check_improvement(&s);
        let (p, b, stats) = sat::optimize(&s, 3.0).unwrap();
        assert!(stats.optimal, "case {case}: timeout");
        assert_eq!(p.metrics.key(), expected, "case {case}");
        assert!(p.metrics.key() <= b.metrics.key());
    }
}

fn check_improvement(s: &Scenario) {
    let t = Travel::new(s).unwrap();
    let mut p = greedy(s, &t, true, None).unwrap();
    let key = p.metrics.key();
    let assigned = |p: &Plan| {
        let mut ids: Vec<_> = p
            .routes
            .iter()
            .flat_map(|r| r.stops.iter().map(|stop| stop.job_id.clone()))
            .collect();
        ids.sort();
        ids
    };
    let jobs = assigned(&p);
    improve(
        s,
        &t,
        &mut p,
        std::time::Instant::now() + std::time::Duration::from_millis(50),
    )
    .unwrap();
    validate_plan(s, &p).unwrap();
    assert!(p.metrics.key() <= key);
    assert_eq!(assigned(&p), jobs);
}

#[test]
fn incumbent_polishing_preserves_objectives_and_daily_staff() {
    use std::time::{Duration, Instant};
    let mut s = tiny();
    s.engineers[1].start = s.engineers[0].start;
    for e in &mut s.engineers {
        e.shift_start = 540;
        e.shift_end = 1440;
    }
    for (j, step) in s.jobs.iter_mut().zip([1, 4, 2, 3]) {
        j.point = s.engineers[0].start;
        j.point.lat += f64::from(step) * 0.01;
        j.duration = 1;
        j.window_start = 540;
        j.window_end = 1400;
        j.transport = None;
    }
    let t = Travel::new(&s).unwrap();
    let mut p = make_plan(&s, &t, &[vec![3, 0, 1, 2], vec![]]).unwrap();
    p.routes.reverse(); // Valid plans need not arrive in engineer order.
    let original = serde_json::to_value(&p).unwrap();
    improve(&s, &t, &mut p, Instant::now()).unwrap();
    assert_eq!(serde_json::to_value(&p).unwrap(), original);
    let before = p.metrics.key();
    improve(&s, &t, &mut p, Instant::now() + Duration::from_secs(1)).unwrap();
    assert!(p.metrics.key() < before);
    assert_eq!(p.metrics.key(), brute(&s));
    validate_plan(&s, &p).unwrap();

    // A paid engineer counts even when idle. Moving the only future job to
    // that engineer reduces daily headcount, even if it increases kilometres.
    s.jobs.truncate(1);
    s.engineers[1].already_used = true;
    s.engineers[1].start.lat += 0.2;
    let t = Travel::new(&s).unwrap();
    let mut p = make_plan(&s, &t, &[vec![0], vec![]]).unwrap();
    let before = p.metrics.clone();
    assert_eq!(before.engineers, 2);
    improve(&s, &t, &mut p, Instant::now() + Duration::from_secs(1)).unwrap();
    assert_eq!(p.metrics.engineers, 1);
    assert!(p.metrics.distance_m > before.distance_m);
    assert_eq!(p.metrics.key(), brute(&s));
    validate_plan(&s, &p).unwrap();
}

#[test]
fn polished_exact_bounds_match_returned_incumbent() {
    let mut s = tiny();
    s.jobs = (0..20)
        .map(|i| {
            let mut j = s.jobs[0].clone();
            j.id = i.to_string();
            j.duration = 1;
            j.window_start = 540;
            j.window_end = 1000;
            j.point.lat += f64::from(i) * 0.001;
            j.transport = None;
            j
        })
        .collect();
    let (p, baseline, stats) = sat::optimize(&s, 0.05).unwrap();
    validate_plan(&s, &p).unwrap();
    assert!(p.metrics.key() <= baseline.metrics.key());
    assert_eq!(stats.scope, "global");
    for (i, stage) in stats.stages.iter().enumerate() {
        assert_eq!(stage.upper, p.metrics.key()[i]);
        assert!(stage.lower <= stage.upper);
        assert_eq!(stage.proven, stage.lower == stage.upper);
    }
    assert_eq!(
        stats.optimal,
        stats.stages.len() == 4 && stats.stages.iter().all(|s| s.proven)
    );
}

#[test]
fn boundaries_cycles_and_empty_inputs() {
    let mut s = tiny();
    s.engineers.truncate(1);
    for j in &mut s.jobs {
        j.point = s.engineers[0].start;
        j.duration = 60;
        j.window_start = 1320;
        j.window_end = 1380;
        j.transport = None;
        j.urgent = false;
    }
    s.engineers[0].shift_start = 1320;
    s.engineers[0].shift_end = 1440;
    let (p, _, stats) = sat::optimize(&s, 3.0).unwrap();
    assert!(stats.optimal);
    assert_eq!(p.metrics.unassigned, 2);
    assert_eq!(p.routes[0].stops.last().unwrap().end, 1440); // end may exceed window_end
    assert_eq!(p.metrics.distance_m, 0); // co-located jobs cannot form disconnected cycles
    s.engineers.clear();
    assert_eq!(sat::optimize(&s, 3.0).unwrap().0.metrics.unassigned, 4);
    s.jobs.clear();
    assert_eq!(
        sat::optimize(&s, 3.0).unwrap().0.metrics.key(),
        [0, 0, 0, 0]
    );
}

#[test]
fn import_all_supplied_csvs_and_validate_bad_inputs() {
    for file in std::fs::read_dir("dataset").unwrap() {
        let file = file.unwrap().path();
        if file.extension().is_none_or(|e| e != "csv") {
            continue;
        }
        if file.to_string_lossy().contains("Юго-восток Контрольное") {
            // This reference export has contradictory windows/statuses for one ID.
            assert!(import::load(&file)
                .unwrap_err()
                .contains("duplicate job: 305898293"));
            continue;
        }
        let s = import::load(&file).unwrap();
        let expected = if file.to_string_lossy().contains("Югоцентр") {
            56
        } else if file.to_string_lossy().contains("Юго-восток") {
            83
        } else {
            66
        };
        assert_eq!(s.jobs.len(), expected);
        assert!(s.notes.iter().any(|n| n.contains("ДЕМОНСТРАЦИЯ")));
        let t = Travel::new(&s).unwrap();
        validate_plan(&s, &greedy(&s, &t, false, None).unwrap()).unwrap();
    }
    let mut s = tiny();
    s.jobs[0].duration = 0;
    assert!(validate_input(&s).is_err());
    s.jobs[0].duration = 30;
    s.jobs[1].id = s.jobs[0].id.clone();
    assert!(validate_input(&s).is_err());
    assert!(import::parse("bad.json", "{}").is_err());
    assert!(import::parse("bad.csv", "Заявка;Начало\n1;nonsense").is_err());
    assert!(sat::optimize(&tiny(), f64::NAN).is_err());
    assert!(sat::optimize(&tiny(), 0.0).is_err());
}

#[test]
fn cancellation_freezes_dispatched_work_and_recounts_daily_staff() {
    let s = import::demo();
    let (previous, _, _) = sat::optimize(&s, 3.0).unwrap();
    let time = 720;
    let frozen: Vec<_> = previous
        .routes
        .iter()
        .flat_map(|r| &r.stops)
        .filter(|s| s.departure < time)
        .cloned()
        .collect();
    let job = previous
        .routes
        .iter()
        .flat_map(|r| &r.stops)
        .find(|s| s.departure >= time)
        .unwrap()
        .job_id
        .clone();
    let response = app::run(app::Request {
        scenario: s.clone(),
        seconds: 3.0,
        mode: app::SearchMode::Fast,
        previous: Some(previous.clone()),
        event: Some(app::Event::Cancel {
            time,
            job_id: job.clone(),
        }),
        last_event_time: 0,
    })
    .unwrap();
    assert!(!response.scenario.jobs.iter().any(|j| j.id == job));
    validate_plan(&response.scenario, &response.plan).unwrap();
    for old in &frozen {
        let new = response
            .plan
            .routes
            .iter()
            .flat_map(|r| &r.stops)
            .find(|s| s.job_id == old.job_id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(new).unwrap()
        );
    }
    assert!(app::run(app::Request {
        scenario: response.scenario,
        seconds: 1.0,
        mode: app::SearchMode::Fast,
        previous: Some(response.plan),
        event: None,
        last_event_time: time
    })
    .is_err());
    assert!(app::run(app::Request {
        scenario: s,
        seconds: 1.0,
        mode: app::SearchMode::Fast,
        previous: Some(previous),
        event: Some(app::Event::Cancel {
            time,
            job_id: frozen[0].job_id.clone()
        }),
        last_event_time: 0
    })
    .is_err());
}

#[test]
fn fast_route_pool_handles_all_datasets_and_keeps_proofs_scoped() {
    for file in crate::datasets().unwrap() {
        let s = import::load(&file).unwrap();
        let (p, baseline, stats) = crate::fast::optimize(&s, 1.0).unwrap();
        validate_plan(&s, &p).unwrap();
        assert!(p.metrics.key() <= baseline.metrics.key());
        // Workbook service times are longer than the old synthetic constants;
        // complete coverage is not assumed under these windows/resources.
        assert_eq!(stats.scope, "candidate_routes");
        assert!(!stats.optimal); // a restricted pool never proves global optimality
    }
    let mut s = tiny();
    s.jobs = (0..20)
        .map(|i| {
            let mut job = s.jobs[0].clone();
            job.id = i.to_string();
            job.point = s.engineers[0].start;
            job.duration = 1;
            job.window_start = 540;
            job.window_end = 1000;
            job.transport = None;
            job
        })
        .collect();
    let (p, _, stats) = crate::fast::optimize(&s, 1.0).unwrap();
    assert_eq!(p.metrics.key(), [0, 0, 1, 0]);
    assert!(stats.search_complete && !stats.optimal); // zero-distance routes must not be forbidden
    let (p, b, _) = crate::fast::optimize(&s, 0.01).unwrap();
    validate_plan(&s, &p).unwrap();
    assert!(p.metrics.key() <= b.metrics.key());
    s.engineers.clear();
    let (p, _, stats) = crate::fast::optimize(&s, 1.0).unwrap();
    assert_eq!(p.metrics.unassigned, 20);
    assert_eq!(p.metrics.engineers, 0);
    assert!(!stats.optimal);
}

fn street_fixture() -> Scenario {
    use crate::routing::{test_snapshot, Leg, Profile, Snapshot};
    let mut s = tiny();
    s.jobs.truncate(3);
    s.engineers.truncate(1);
    s.engineers[0].shift_start = 540;
    s.engineers[0].shift_end = 600;
    for (i, j) in s.jobs.iter_mut().enumerate() {
        j.duration = 10;
        j.transport = None;
        j.urgent = false;
        j.window_start = 541 + i as u32 * 11;
        j.window_end = j.window_start + 1;
    }
    let mut points: Vec<_> = s.jobs.iter().map(|j| j.point).collect();
    points.push(s.engineers[0].start);
    let leg = |n| {
        Some(Leg {
            minutes: n,
            metres: n * 10,
            shape: "??AA".into(),
        })
    };
    let legs = vec![
        vec![leg(0), leg(1), leg(100), None],
        vec![None, leg(0), leg(1), None],
        vec![None, None, leg(0), None],
        vec![leg(1), None, leg(100), leg(0)],
    ];
    s.routing = Some(test_snapshot(Snapshot {
        points,
        transit: None,
        profiles: vec![Profile {
            transport: Transport::Car,
            legs,
            lower: vec![],
            coverage: None,
        }],
    }));
    s
}

#[test]
fn full_sat_handles_directed_nonmetric_streets() {
    let s = street_fixture();
    let (p, _, stats) = sat::optimize(&s, 1.0).unwrap();
    assert!(stats.optimal);
    assert_eq!(p.metrics.key(), [0, 0, 1, 30]);
    assert_eq!(p.metrics.key(), brute(&s));
    // Base -> second job is unreachable, and first -> third is slow, yet the
    // route through intervening stops is feasible. Direct/pairwise pruning is unsound.
    let mut bad = p.clone();
    bad.routes[0].stops[0].distance_m += 1;
    assert!(validate_plan(&s, &bad).is_err());
    let mut bad = p.clone();
    bad.routes[0].stops[0].shape = "tampered".into();
    assert!(validate_plan(&s, &bad).is_err());
    let mut moved = s.clone();
    moved.jobs[0].point.lat += 0.1;
    assert!(Travel::new(&moved).is_err());
    for seed in 1..=60u32 {
        let mut r = s.clone();
        let tight = seed.is_multiple_of(2);
        for j in &mut r.jobs {
            j.window_start = 540;
            j.window_end = if tight { 545 } else { 590 };
            j.duration = if tight { 4 } else { 5 };
        }
        if seed.is_multiple_of(3) {
            let mut e = r.engineers[0].clone();
            e.id = "second".into();
            e.shift_start = 542;
            e.already_used = seed.is_multiple_of(6);
            r.engineers.push(e);
        }
        let mut snapshot =
            (*crate::routing::load(&r.routing.as_ref().unwrap().key).unwrap()).clone();
        for (i, row) in snapshot.profiles[0].legs.iter_mut().enumerate() {
            for (j, leg) in row.iter_mut().enumerate() {
                let k = (seed * 37 + i as u32 * 19 + j as u32 * 73) % 47;
                *leg = if i != j && k.is_multiple_of(7) {
                    None
                } else {
                    Some(crate::routing::Leg {
                        minutes: if i == j {
                            0
                        } else if tight {
                            1 + k % 5
                        } else {
                            k
                        },
                        metres: k * 10,
                        shape: "??AA".into(),
                    })
                };
            }
        }
        r.routing = Some(crate::routing::test_snapshot(snapshot));
        check_improvement(&r);
        let (p, _, st) = sat::optimize(&r, 1.0).unwrap();
        assert!(st.optimal);
        assert_eq!(p.metrics.key(), brute(&r), "street seed {seed}");
    }
}

#[test]
fn urgent_addition_is_transactional_and_preserves_city_history() {
    let s = street_fixture();
    let (previous, _, _) = sat::optimize(&s, 1.0).unwrap();
    let mut urgent = s.jobs[1].clone();
    urgent.id = "URG-1".into();
    urgent.duration = 1;
    urgent.window_start = 545;
    urgent.window_end = 560;
    urgent.urgent = false;
    let mut compact_previous = previous.clone();
    for r in &mut compact_previous.routes {
        for stop in &mut r.stops {
            stop.shape.clear();
        }
    }
    let request = |job: Job| app::Request {
        scenario: s.clone(),
        seconds: 1.0,
        mode: app::SearchMode::Exact,
        previous: Some(compact_previous.clone()),
        event: Some(app::Event::AddUrgent { time: 545, job }),
        last_event_time: 0,
    };
    assert!(app::run(request(s.jobs[1].clone())).is_err());
    let mut invalid = urgent.clone();
    invalid.duration = 0;
    assert!(app::run(request(invalid)).is_err());
    let mut invalid = urgent.clone();
    invalid.window_start = 544;
    assert!(app::run(request(invalid)).is_err());
    let response = app::run(request(urgent)).unwrap();
    assert_eq!(response.scenario.jobs.len(), 4);
    assert!(response.scenario.jobs[3].urgent);
    assert_eq!(response.plan.metrics.urgent_unassigned, 0);
    assert_eq!(
        serde_json::to_value(&response.plan.routes[0].stops[0]).unwrap(),
        serde_json::to_value(&previous.routes[0].stops[0]).unwrap()
    );
    validate_plan(&response.scenario, &response.plan).unwrap();
    let back = app::Request {
        scenario: response.scenario.clone(),
        seconds: 1.0,
        mode: app::SearchMode::Exact,
        previous: Some(response.plan.clone()),
        event: Some(app::Event::Cancel {
            time: 544,
            job_id: "URG-1".into(),
        }),
        last_event_time: 545,
    };
    assert!(app::run(back).is_err());
    let cancelled = app::run(app::Request {
        scenario: response.scenario,
        seconds: 1.0,
        mode: app::SearchMode::Exact,
        previous: Some(response.plan),
        event: Some(app::Event::Cancel {
            time: 545,
            job_id: "URG-1".into(),
        }),
        last_event_time: 545,
    })
    .unwrap();
    assert_eq!(cancelled.plan.metrics.key(), previous.metrics.key());
    assert_eq!(cancelled.scenario.jobs.len(), 3);
}

#[test]
fn timeout_keeps_a_valid_incumbent_without_false_optimality() {
    let s = import::load(std::path::Path::new(
        "dataset/Югоцентр Синтетические данные.csv.new.csv",
    ))
    .unwrap();
    let (plan, baseline, stats) = sat::optimize(&s, 0.01).unwrap();
    validate_plan(&s, &plan).unwrap();
    assert!(plan.metrics.key() <= baseline.metrics.key());
    assert!(!stats.optimal);
}
