use crate::{model::*, sat::Stats};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

fn budget() -> f64 {
    5.0
}
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Fast,
    Exact,
}
fn optimize(s: &Scenario, seconds: f64, mode: SearchMode) -> Result<(Plan, Plan, Stats)> {
    match mode {
        SearchMode::Fast => crate::fast::optimize(s, seconds),
        SearchMode::Exact => crate::sat::optimize(s, seconds),
    }
}
#[derive(Deserialize)]
pub struct Request {
    pub scenario: Scenario,
    #[serde(default = "budget")]
    pub seconds: f64,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub previous: Option<Plan>,
    #[serde(default)]
    pub event: Option<Event>,
    #[serde(default)]
    pub last_event_time: u32,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Cancel { time: u32, job_id: String },
    AddUrgent { time: u32, job: Job },
}
#[derive(Serialize)]
pub struct Response {
    pub scenario: Scenario,
    pub plan: Plan,
    pub baseline: Plan,
    pub stats: Stats,
    pub changes: Vec<String>,
    pub last_event_time: u32,
}

pub fn run(req: Request) -> Result<Response> {
    validate_input(&req.scenario)?;
    if !req.seconds.is_finite() || !(0.01..=300.0).contains(&req.seconds) {
        return Err("seconds must be between 0.01 and 300".into());
    }
    let Some(event) = req.event else {
        if req.previous.is_some() || req.last_event_time != 0 {
            return Err("Для нового плана с начала дня откройте исходный набор заново; текущее состояние требует события".into());
        }
        let (plan, baseline, stats) = optimize(&req.scenario, req.seconds, req.mode)?;
        return Ok(Response {
            scenario: req.scenario,
            plan,
            baseline,
            stats,
            changes: vec![],
            last_event_time: 0,
        });
    };
    let time = match &event {
        Event::Cancel { time, .. } | Event::AddUrgent { time, .. } => *time,
    };
    if time > 1440 || time < req.last_event_time {
        return Err("Event time must be within this day and not precede the previous event".into());
    }
    let mut old = req
        .previous
        .ok_or("Replanning requires the previous plan")?;
    validate_plan(&req.scenario, &old)?;
    // Geometry may be omitted from submitted history. Recover it from the SAME
    // snapshot, never a fresh network query, before freezing published stops.
    if req.scenario.routing.is_some() {
        let t = Travel::new(&req.scenario)?;
        for route in &mut old.routes {
            let e = req
                .scenario
                .engineers
                .iter()
                .position(|e| e.id == route.engineer_id)
                .unwrap();
            let mut previous = None;
            for stop in &mut route.stops {
                let j = req
                    .scenario
                    .jobs
                    .iter()
                    .position(|j| j.id == stop.job_id)
                    .unwrap();
                stop.shape = t
                    .leg(e, previous, j, stop.departure)
                    .ok_or("Published journey is unavailable")?
                    .shape
                    .into();
                previous = Some(j);
            }
        }
    }
    let mut scenario = req.scenario;
    let change = match event {
        Event::Cancel { job_id, .. } => {
            if !scenario.jobs.iter().any(|j| j.id == job_id) {
                return Err("Unknown cancellation job ID".into());
            }
            // Departure commits the whole visit, including travel, waiting and service.
            if old
                .routes
                .iter()
                .flat_map(|r| &r.stops)
                .any(|s| s.job_id == job_id && s.departure < time)
            {
                return Err("Нельзя отменить уже начатый выезд. Выберите заявку, к которой инженер ещё не отправился".into());
            }
            scenario.jobs.retain(|j| j.id != job_id);
            format!("заявка {job_id} отменена")
        }
        Event::AddUrgent { mut job, .. } => {
            if job.window_start < time {
                return Err("Urgent job window must not start before its arrival event".into());
            }
            job.urgent = true;
            let id = job.id.clone();
            scenario.jobs.push(job);
            validate_input(&scenario)?;
            if scenario.routing.is_some() {
                scenario = crate::routing::prepare(scenario, true)?;
            }
            format!("добавлена срочная заявка {id}")
        }
    };
    let mut remaining = scenario.clone();
    let mut frozen = HashSet::new();
    let mut prefixes = vec![];
    for e in &mut remaining.engineers {
        let stops: Vec<_> = old
            .routes
            .iter()
            .find(|r| r.engineer_id == e.id)
            .map(|r| {
                r.stops
                    .iter()
                    .take_while(|s| s.departure < time)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for stop in &stops {
            frozen.insert(stop.job_id.clone());
        }
        e.shift_start = time.max(e.shift_start).min(e.shift_end);
        if let Some(last) = stops.last() {
            e.start = scenario
                .jobs
                .iter()
                .find(|j| j.id == last.job_id)
                .unwrap()
                .point;
            e.shift_start = e.shift_start.max(last.end);
            e.already_used = true;
        }
        prefixes.push(Route {
            engineer_id: e.id.clone(),
            distance_m: stops.iter().map(|s| s.distance_m).sum(),
            stops,
        });
    }
    remaining.jobs.retain(|j| !frozen.contains(&j.id));
    let (mut plan, mut baseline, mut stats) = optimize(&remaining, req.seconds, req.mode)?;
    let prefix_distance: u32 = prefixes.iter().map(|r| r.distance_m).sum();
    for p in [&mut plan, &mut baseline] {
        for route in &mut p.routes {
            let prefix = prefixes
                .iter()
                .find(|r| r.engineer_id == route.engineer_id)
                .unwrap();
            let mut stops = prefix.stops.clone();
            stops.append(&mut route.stops);
            route.stops = stops;
            route.distance_m += prefix.distance_m;
        }
        refresh_metrics(&scenario, p);
        validate_plan(&scenario, p)?;
    }
    for stage in &mut stats.stages {
        if stage.criterion == "distance_m" {
            stage.lower += prefix_distance;
            stage.upper += prefix_distance;
        }
    }
    let mut changes = vec![format!(
        "{}: {}; выполненные и начатые выезды сохранены",
        clock(time),
        change
    )];
    for j in &scenario.jobs {
        let locate = |p: &Plan| {
            p.routes.iter().find_map(|r| {
                r.stops
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.job_id == j.id)
                    .map(|(i, s)| (r.engineer_id.clone(), i + 1, s.start))
            })
        };
        let before = locate(&old);
        let after = locate(&plan);
        if before != after {
            let label = |p: Option<(String, usize, u32)>| {
                p.map(|(e, i, t)| format!("{e}, №{i}, {}", clock(t)))
                    .unwrap_or("не назначена".into())
            };
            changes.push(format!("{}: {} → {}", j.id, label(before), label(after)));
        }
    }
    Ok(Response {
        scenario,
        plan,
        baseline,
        stats,
        changes,
        last_event_time: time,
    })
}
