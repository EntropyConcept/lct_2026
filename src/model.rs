use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    Local,
    Connection,
    Emergency,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Car,
    Walk,
    Bicycle,
    Public,
}
impl Transport {
    pub fn speed(self) -> f64 {
        match self {
            Self::Car => 30.0,
            Self::Walk => 5.0,
            Self::Bicycle => 15.0,
            Self::Public => 20.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub address: String,
    pub point: Point,
    pub duration: u32,
    #[serde(default)]
    pub work_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geocode_match: Option<String>,
    pub window_start: u32,
    pub window_end: u32,
    pub skill: Skill,
    #[serde(default)]
    pub transport: Option<Transport>,
    #[serde(default)]
    pub urgent: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Engineer {
    pub id: String,
    pub name: String,
    pub start: Point,
    pub shift_start: u32,
    pub shift_end: u32,
    pub skills: Vec<Skill>,
    pub transport: Transport,
    // Replanning: these engineers already count toward the daily staff total.
    #[serde(default)]
    pub already_used: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub jobs: Vec<Job>,
    pub engineers: Vec<Engineer>,
    #[serde(default)]
    pub routing: Option<crate::routing::RoutingRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transit_date: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

pub fn validate_input(s: &Scenario) -> Result<()> {
    if s.routing.as_ref().is_some_and(|r| r.source.len() > 512) {
        return Err("Routing source label too long".into());
    }
    if let Some(date) = &s.transit_date {
        crate::transit::midnight(date)?;
    }
    if s.name.len() > 512 || s.notes.len() > 32 || s.notes.iter().any(|n| n.len() > 4096) {
        return Err("Scenario name/notes too long".into());
    }
    if s.jobs.len() > 100 || s.engineers.len() > 15 {
        return Err("Prototype limit: 100 jobs and 15 engineers".into());
    }
    let point_ok = |p: Point| {
        p.lat.is_finite() && p.lon.is_finite() && p.lat.abs() <= 90.0 && p.lon.abs() <= 180.0
    };
    let mut ids = HashSet::new();
    for j in &s.jobs {
        if j.id.trim().is_empty()
            || j.id.len() > 128
            || j.address.len() > 2048
            || j.work_type.as_ref().is_some_and(|w| w.len() > 128)
            || j.geocode_match.as_ref().is_some_and(|m| m.len() > 4096)
            || !ids.insert(&j.id)
            || !point_ok(j.point)
            || j.duration == 0
            || j.duration > 1440
            || j.window_start > j.window_end
            || j.window_end > 1440
        {
            return Err(format!(
                "Invalid/duplicate job: {} (times must be minutes within one day, duration > 0)",
                j.id
            ));
        }
    }
    ids.clear();
    for e in &s.engineers {
        if e.id.trim().is_empty()
            || e.id.len() > 128
            || e.name.len() > 512
            || !ids.insert(&e.id)
            || !point_ok(e.start)
            || e.shift_start > e.shift_end
            || e.shift_end > 1440
            || e.skills.is_empty()
            || e.skills.len() > 3
        {
            return Err(format!("Invalid/duplicate engineer: {}", e.id));
        }
    }
    Ok(())
}

pub fn compatible(e: &Engineer, j: &Job) -> bool {
    e.skills.contains(&j.skill) && j.transport.is_none_or(|t| t == e.transport)
}

// Haversine, rounded UP to metres. No geocoding or external routing in the hot path.
pub fn metres(a: Point, b: Point) -> u32 {
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + a.lat.to_radians().cos() * b.lat.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    (12_742_000.0 * h.clamp(0.0, 1.0).sqrt().asin()).ceil() as u32
}
pub fn minutes(distance: u32, transport: Transport) -> u32 {
    (distance as f64 * 60.0 / (transport.speed() * 1000.0)).ceil() as u32
}

pub struct Travel {
    pub distance: Vec<Vec<Vec<u32>>>,
    pub start_distance: Vec<Vec<u32>>,
    pub time: Vec<Vec<Vec<u32>>>,
    pub start_time: Vec<Vec<u32>>,
    pub time_lower: Vec<Vec<Vec<u32>>>,
    pub start_lower: Vec<Vec<u32>>,
    pub metric: bool,
    snapshot: Option<std::sync::Arc<crate::routing::Snapshot>>,
    job_points: Vec<usize>,
    base_points: Vec<usize>,
    profiles: Vec<usize>,
}

pub struct TravelLeg<'a> {
    pub metres: u32,
    pub minutes: u32,
    pub shape: &'a str,
    pub description: &'a str,
}
impl Travel {
    pub fn new(s: &Scenario) -> Result<Self> {
        if let Some(reference) = &s.routing {
            return Self::city(s, crate::routing::load(&reference.key)?);
        }
        let distance: Vec<Vec<u32>> = s
            .jobs
            .iter()
            .map(|a| s.jobs.iter().map(|b| metres(a.point, b.point)).collect())
            .collect();
        let start_distance: Vec<Vec<u32>> = s
            .engineers
            .iter()
            .map(|e| s.jobs.iter().map(|j| metres(e.start, j.point)).collect())
            .collect();
        let time: Vec<Vec<Vec<u32>>> = s
            .engineers
            .iter()
            .map(|e| {
                distance
                    .iter()
                    .map(|row| row.iter().map(|&d| minutes(d, e.transport)).collect())
                    .collect()
            })
            .collect();
        let start_time: Vec<Vec<u32>> = s
            .engineers
            .iter()
            .zip(&start_distance)
            .map(|(e, row)| row.iter().map(|&d| minutes(d, e.transport)).collect())
            .collect();
        Ok(Self {
            distance: vec![distance; s.engineers.len()],
            start_distance,
            time_lower: time.clone(),
            start_lower: start_time.clone(),
            time,
            start_time,
            metric: true,
            snapshot: None,
            job_points: vec![],
            base_points: vec![],
            profiles: vec![],
        })
    }
    fn city(s: &Scenario, snapshot: std::sync::Arc<crate::routing::Snapshot>) -> Result<Self> {
        use crate::routing::{point_index, UNREACHABLE};
        if s.engineers.iter().any(|e| e.transport == Transport::Public) {
            let timetable = snapshot
                .transit
                .as_ref()
                .ok_or("Prepare a timetable snapshot for public transport")?;
            if s.transit_date.as_deref() != Some(timetable.date.as_str()) {
                return Err("Transit service date changed: prepare city routing again".into());
            }
            let required = crate::transit_scope::required(s, &snapshot.points)?;
            for (from, row) in required.iter().enumerate() {
                for (to, needed) in row.iter().enumerate() {
                    let Some(needed) = needed else { continue };
                    let covered = timetable
                        .coverage
                        .get(from)
                        .and_then(|row| row.get(to))
                        .copied()
                        .flatten();
                    if !covered.is_some_and(|covered| covered.contains(*needed)) {
                        return Err(format!(
                            "Transit coverage is incomplete for point pair {from}->{to} (departures {}–{}): prepare city routing again",
                            needed.start, needed.end,
                        ));
                    }
                }
            }
        }
        for profile in &snapshot.profiles {
            if profile.transport == Transport::Public {
                continue;
            }
            if let Some(covered) = &profile.coverage {
                let required =
                    crate::transit_scope::required_for(s, &snapshot.points, profile.transport)?;
                for (from, row) in required.iter().enumerate() {
                    for (to, needed) in row.iter().enumerate() {
                        if needed.is_some() && !covered[from][to] {
                            return Err("Road coverage is incomplete after input changes: prepare city routing again".into());
                        }
                    }
                }
            }
        }
        let job_points: Vec<_> = s
            .jobs
            .iter()
            .map(|j| point_index(&snapshot.points, j.point))
            .collect::<Result<_>>()?;
        let base_points: Vec<_> = s
            .engineers
            .iter()
            .map(|e| point_index(&snapshot.points, e.start))
            .collect::<Result<_>>()?;
        let profiles: Vec<_> = s
            .engineers
            .iter()
            .map(|e| {
                snapshot
                    .profiles
                    .iter()
                    .position(|p| p.transport == e.transport)
                    .ok_or_else(|| "Transport changed: prepare city routing again".into())
            })
            .collect::<Result<_>>()?;
        let mut t = Self {
            distance: vec![],
            start_distance: vec![],
            time: vec![],
            start_time: vec![],
            time_lower: vec![],
            start_lower: vec![],
            metric: false,
            snapshot: None,
            job_points,
            base_points,
            profiles,
        };
        for (e, &profile) in t.profiles.iter().enumerate() {
            let p = &snapshot.profiles[profile];
            let base = t.base_points[e];
            t.start_distance.push(
                t.job_points
                    .iter()
                    .map(|&j| p.legs[base][j].as_ref().map_or(0, |l| l.metres))
                    .collect(),
            );
            t.start_time.push(
                t.job_points
                    .iter()
                    .map(|&j| p.legs[base][j].as_ref().map_or(UNREACHABLE, |l| l.minutes))
                    .collect(),
            );
            t.start_lower
                .push(t.job_points.iter().map(|&j| p.lower[base][j]).collect());
            t.distance.push(
                t.job_points
                    .iter()
                    .map(|&i| {
                        t.job_points
                            .iter()
                            .map(|&j| p.legs[i][j].as_ref().map_or(0, |l| l.metres))
                            .collect()
                    })
                    .collect(),
            );
            t.time.push(
                t.job_points
                    .iter()
                    .map(|&i| {
                        t.job_points
                            .iter()
                            .map(|&j| p.legs[i][j].as_ref().map_or(UNREACHABLE, |l| l.minutes))
                            .collect()
                    })
                    .collect(),
            );
            t.time_lower.push(
                t.job_points
                    .iter()
                    .map(|&i| t.job_points.iter().map(|&j| p.lower[i][j]).collect())
                    .collect(),
            );
        }
        t.snapshot = Some(snapshot);
        Ok(t)
    }
    pub fn leg(
        &self,
        e: usize,
        previous: Option<usize>,
        j: usize,
        ready: u32,
    ) -> Option<TravelLeg<'_>> {
        if let Some(snapshot) = &self.snapshot {
            let from = previous.map_or(self.base_points[e], |i| self.job_points[i]);
            let to = self.job_points[j];
            let profile = &snapshot.profiles[self.profiles[e]];
            if profile.transport == Transport::Public {
                if from == to {
                    return Some(TravelLeg {
                        metres: 0,
                        minutes: 0,
                        shape: "",
                        description: "",
                    });
                }
                let journey = snapshot.transit.as_ref()?.journey(from, to, ready)?;
                return Some(TravelLeg {
                    metres: journey.leg.metres,
                    minutes: journey.arrival.checked_sub(ready)?,
                    shape: &journey.leg.shape,
                    description: &journey.description,
                });
            }
            let leg = profile.legs[from][to].as_ref()?;
            return Some(TravelLeg {
                metres: leg.metres,
                minutes: leg.minutes,
                shape: &leg.shape,
                description: "",
            });
        }
        let (metres, minutes) = previous
            .map_or((self.start_distance[e][j], self.start_time[e][j]), |i| {
                (self.distance[e][i][j], self.time[e][i][j])
            });
        Some(TravelLeg {
            metres,
            minutes,
            shape: "",
            description: "",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stop {
    pub job_id: String,
    pub departure: u32,
    pub arrival: u32,
    pub start: u32,
    pub end: u32,
    pub distance_m: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shape: String,
    pub explanation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Route {
    pub engineer_id: String,
    pub stops: Vec<Stop>,
    pub distance_m: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unassigned {
    pub job_id: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub urgent_unassigned: u32,
    pub unassigned: u32,
    pub engineers: u32,
    pub distance_m: u32,
}
impl Metrics {
    pub fn key(&self) -> [u32; 4] {
        [
            self.urgent_unassigned,
            self.unassigned,
            self.engineers,
            self.distance_m,
        ]
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub routes: Vec<Route>,
    pub unassigned: Vec<Unassigned>,
    pub metrics: Metrics,
}

pub fn schedule(s: &Scenario, t: &Travel, e: usize, jobs: &[usize]) -> Option<Route> {
    route(s, t, e, jobs, true)
}
pub fn route_cost(s: &Scenario, t: &Travel, e: usize, jobs: &[usize]) -> Option<u32> {
    route(s, t, e, jobs, false).map(|r| r.distance_m)
}
fn route(s: &Scenario, t: &Travel, e: usize, jobs: &[usize], details: bool) -> Option<Route> {
    let engineer = &s.engineers[e];
    let mut route = Route {
        engineer_id: if details {
            engineer.id.clone()
        } else {
            String::new()
        },
        stops: vec![],
        distance_m: 0,
    };
    let mut end = engineer.shift_start;
    let mut prev: Option<usize> = None;
    for &j in jobs {
        let job = &s.jobs[j];
        if !compatible(engineer, job) {
            return None;
        }
        let leg = t.leg(e, prev, j, end)?;
        let (distance, travel) = (leg.metres, leg.minutes);
        let arrival = end + travel;
        let start = arrival.max(job.window_start);
        if start > job.window_end || start + job.duration > engineer.shift_end {
            return None;
        }
        if details {
            route.stops.push(Stop {
            job_id: job.id.clone(), departure: end, arrival, start, end: start + job.duration, distance_m: distance, shape: leg.shape.into(),
            explanation: format!("Навык и транспорт подходят. Переезд с ожиданием {} мин; начало в окне {}–{}, работа заканчивается до конца смены. {} Порядок выбран для совместного выполнения заявок с меньшим числом инженеров и пробегом; он не обязательно единственный.", travel, clock(job.window_start), clock(job.window_end), leg.description),
        });
        }
        route.distance_m += distance;
        end = start + job.duration;
        prev = Some(j);
    }
    Some(route)
}

pub fn clock(t: u32) -> String {
    format!("{:02}:{:02}", t / 60, t % 60)
}

pub fn unassigned_reason(s: &Scenario, t: &Travel, j: usize) -> String {
    let job = &s.jobs[j];
    if !s.engineers.iter().any(|e| e.skills.contains(&job.skill)) {
        "Нет инженера с нужным навыком".into()
    } else if !s.engineers.iter().any(|e| compatible(e, job)) {
        "Нет инженера с нужным сочетанием навыка и транспорта".into()
    } else if t.metric && !(0..s.engineers.len()).any(|e| schedule(s, t, e, &[j]).is_some()) {
        "Даже отдельный выезд не помещается в окно начала и смену с учётом переезда".into()
    } else {
        "Не включена в найденный план при конкуренции за время исполнителей. Это не доказательство невозможности: может существовать другой план".into()
    }
}
pub fn make_plan(s: &Scenario, t: &Travel, orders: &[Vec<usize>]) -> Result<Plan> {
    let mut routes = vec![];
    for (e, jobs) in orders.iter().enumerate() {
        routes.push(schedule(s, t, e, jobs).ok_or("Invalid route produced by planner")?);
    }
    let assigned: HashSet<_> = orders.iter().flatten().copied().collect();
    let unassigned = (0..s.jobs.len())
        .filter(|j| !assigned.contains(j))
        .map(|j| Unassigned {
            job_id: s.jobs[j].id.clone(),
            reason: unassigned_reason(s, t, j),
        })
        .collect();
    let mut plan = Plan {
        routes,
        unassigned,
        metrics: Metrics::default(),
    };
    refresh_metrics(s, &mut plan);
    validate_with_travel(s, &plan, t)?;
    Ok(plan)
}
pub fn refresh_metrics(s: &Scenario, p: &mut Plan) {
    p.metrics = Metrics {
        urgent_unassigned: p
            .unassigned
            .iter()
            .filter(|u| s.jobs.iter().any(|j| j.id == u.job_id && j.urgent))
            .count() as u32,
        unassigned: p.unassigned.len() as u32,
        engineers: s
            .engineers
            .iter()
            .filter(|e| {
                e.already_used
                    || p.routes
                        .iter()
                        .any(|r| r.engineer_id == e.id && !r.stops.is_empty())
            })
            .count() as u32,
        distance_m: p.routes.iter().map(|r| r.distance_m).sum(),
    };
}

// Independent replay: never trust a SAT model, client-supplied plan or heuristic.
pub fn validate_plan(s: &Scenario, p: &Plan) -> Result<()> {
    validate_with_travel(s, p, &Travel::new(s)?)
}
fn validate_with_travel(s: &Scenario, p: &Plan, t: &Travel) -> Result<()> {
    let mut jobs = HashSet::new();
    let mut engineers = HashSet::new();
    for r in &p.routes {
        let ei = s
            .engineers
            .iter()
            .position(|e| e.id == r.engineer_id)
            .ok_or("Unknown engineer in plan")?;
        let e = &s.engineers[ei];
        if !engineers.insert(&e.id) {
            return Err("Duplicate engineer route".into());
        }
        let mut previous: Option<usize> = None;
        let mut end = e.shift_start;
        let mut total = 0;
        for stop in &r.stops {
            let ji = s
                .jobs
                .iter()
                .position(|j| j.id == stop.job_id)
                .ok_or("Unknown job in plan")?;
            let j = &s.jobs[ji];
            let leg = t
                .leg(ei, previous, ji, stop.departure)
                .ok_or_else(|| format!("No available journey for {}", j.id))?;
            let (d, travel) = (leg.metres, leg.minutes);
            if stop.departure > 1440
                || stop.arrival > 1440
                || stop.start > 1440
                || stop.end > 1440
                || !jobs.insert(&j.id)
                || !compatible(e, j)
                || stop.departure < end
                || stop.arrival != stop.departure + travel
                || (!stop.shape.is_empty() && stop.shape != leg.shape)
                || stop.start < stop.arrival
                || stop.start < j.window_start
                || stop.start > j.window_end
                || stop.end != stop.start + j.duration
                || stop.end > e.shift_end
                || stop.distance_m != d
            {
                return Err(format!("Invalid schedule for {}", j.id));
            }
            total += d;
            previous = Some(ji);
            end = stop.end;
        }
        if total != r.distance_m {
            return Err("Invalid route distance".into());
        }
    }
    for u in &p.unassigned {
        let j = s
            .jobs
            .iter()
            .find(|j| j.id == u.job_id)
            .ok_or("Unknown unassigned job")?;
        if !jobs.insert(&j.id) {
            return Err("Duplicate assigned/unassigned job".into());
        }
    }
    if jobs.len() != s.jobs.len() {
        return Err("Plan omits jobs".into());
    }
    let mut check = p.clone();
    refresh_metrics(s, &mut check);
    if p.metrics.key() != check.metrics.key() {
        return Err("Incorrect plan metrics".into());
    }
    Ok(())
}

/// Improve an incumbent without changing its assigned job set or restricting SAT.
/// Every move replays complete affected routes: street matrices can be nonmetric.
pub fn improve(
    s: &Scenario,
    t: &Travel,
    best: &mut Plan,
    deadline: std::time::Instant,
) -> Result<()> {
    let ids: std::collections::HashMap<_, _> = s
        .jobs
        .iter()
        .enumerate()
        .map(|(j, job)| (job.id.as_str(), j))
        .collect();
    let mut orders = vec![vec![]; s.engineers.len()];
    for route in &best.routes {
        let e = s
            .engineers
            .iter()
            .position(|e| e.id == route.engineer_id)
            .ok_or("Unknown improvement engineer")?;
        orders[e] = route
            .stops
            .iter()
            .map(|stop| ids[stop.job_id.as_str()])
            .collect();
    }
    let mut changed = false;
    // ponytail: relocation/swap/reversal local optimum only; unrestricted SAT
    // remains responsible for global improvements and proofs.
    while std::time::Instant::now() < deadline {
        let costs: Vec<_> = orders
            .iter()
            .enumerate()
            .map(|(e, jobs)| {
                route_cost(s, t, e, jobs).ok_or_else(|| "Invalid improvement incumbent".to_owned())
            })
            .collect::<Result<_>>()?;
        let total: u32 = costs.iter().sum();
        let mut score = (0i32, total);
        let mut winner = None;
        'relocate: for source in 0..orders.len() {
            for i in 0..orders[source].len() {
                if std::time::Instant::now() >= deadline {
                    break 'relocate;
                }
                let j = orders[source].remove(i);
                let remainder = route_cost(s, t, source, &orders[source]);
                for target in 0..orders.len() {
                    if target != source && remainder.is_none() {
                        continue;
                    }
                    let staff = if source == target {
                        0
                    } else {
                        i32::from(orders[target].is_empty() && !s.engineers[target].already_used)
                            - i32::from(
                                orders[source].is_empty() && !s.engineers[source].already_used,
                            )
                    };
                    if staff > 0 {
                        continue;
                    }
                    for pos in 0..=orders[target].len() {
                        orders[target].insert(pos, j);
                        if let Some(after) = route_cost(s, t, target, &orders[target]) {
                            let distance = if target == source {
                                total - costs[source] + after
                            } else {
                                total - costs[source] - costs[target] + remainder.unwrap() + after
                            };
                            if (staff, distance) < score {
                                score = (staff, distance);
                                winner = Some(orders.clone());
                            }
                        }
                        orders[target].remove(pos);
                    }
                }
                orders[source].insert(i, j);
            }
        }
        'swap: for a in 0..orders.len() {
            for i in 0..orders[a].len() {
                if std::time::Instant::now() >= deadline {
                    break 'swap;
                }
                for b in a..orders.len() {
                    for j in if a == b {
                        i + 1..orders[b].len()
                    } else {
                        0..orders[b].len()
                    } {
                        let (x, y) = (orders[a][i], orders[b][j]);
                        orders[a][i] = y;
                        orders[b][j] = x;
                        if let Some(ca) = route_cost(s, t, a, &orders[a]) {
                            let distance = if a == b {
                                Some(total - costs[a] + ca)
                            } else {
                                route_cost(s, t, b, &orders[b])
                                    .map(|cb| total - costs[a] - costs[b] + ca + cb)
                            };
                            if let Some(d) = distance {
                                if (0, d) < score {
                                    score = (0, d);
                                    winner = Some(orders.clone());
                                }
                            }
                        }
                        orders[a][i] = x;
                        orders[b][j] = y;
                        if a == b && j > i + 1 {
                            orders[a][i..=j].reverse();
                            if let Some(ca) = route_cost(s, t, a, &orders[a]) {
                                let d = total - costs[a] + ca;
                                if (0, d) < score {
                                    score = (0, d);
                                    winner = Some(orders.clone());
                                }
                            }
                            orders[a][i..=j].reverse();
                        }
                    }
                }
            }
        }
        let Some(next) = winner else {
            break;
        };
        orders = next;
        changed = true;
    }
    if changed {
        let candidate = make_plan(s, t, &orders)?;
        if candidate.metrics.key() >= best.metrics.key() {
            return Err("Route improvement did not improve the lexicographic objective".into());
        }
        *best = candidate;
    }
    Ok(())
}

pub fn greedy(
    s: &Scenario,
    t: &Travel,
    insertion: bool,
    deadline: Option<std::time::Instant>,
) -> Result<Plan> {
    let mut jobs: Vec<_> = (0..s.jobs.len()).collect();
    if insertion {
        jobs.sort_by_key(|&j| {
            (
                !s.jobs[j].urgent,
                s.jobs[j].window_end,
                s.jobs[j].window_start,
            )
        });
    }
    greedy_order(s, t, jobs, insertion, 0, deadline)
}

// Deterministic multistart seeds; all trial routes still use the same exact feasibility check.
pub fn greedy_variant(
    s: &Scenario,
    t: &Travel,
    seed: u32,
    deadline: std::time::Instant,
) -> Result<Plan> {
    let mut jobs: Vec<_> = (0..s.jobs.len()).collect();
    jobs.sort_by_key(|&j| {
        let noise = mix(seed.wrapping_mul(101) ^ j as u32);
        (!s.jobs[j].urgent, s.jobs[j].window_end / 120, noise)
    });
    greedy_order(s, t, jobs, true, seed, Some(deadline))
}
fn mix(mut n: u32) -> u32 {
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846ca68b);
    n ^ (n >> 16)
}
fn greedy_order(
    s: &Scenario,
    t: &Travel,
    jobs: Vec<usize>,
    insertion: bool,
    seed: u32,
    deadline: Option<std::time::Instant>,
) -> Result<Plan> {
    let mut orders = vec![vec![]; s.engineers.len()];
    for j in jobs {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            break;
        }
        let mut best = None;
        for (e, order) in orders.iter_mut().enumerate() {
            let old_distance = route(s, t, e, order, false).unwrap().distance_m;
            let jitter = if seed == 0 {
                0
            } else {
                (mix(seed ^ ((e as u32 + 1) * 12347) ^ j as u32) % 1500) as i64
            };
            let positions = if insertion {
                0..=order.len()
            } else {
                order.len()..=order.len()
            };
            for pos in positions {
                order.insert(pos, j);
                if let Some(r) = route(s, t, e, order, false) {
                    let key = (
                        order.len() == 1 && !s.engineers[e].already_used,
                        r.distance_m as i64 - old_distance as i64 + jitter,
                        e,
                        pos,
                    );
                    if best.is_none_or(|b| key < b) {
                        best = Some(key);
                    }
                }
                order.remove(pos);
                if !insertion && best.is_some() {
                    break;
                }
            }
            if !insertion && best.is_some() {
                break;
            }
        }
        if let Some((_, _, e, pos)) = best {
            orders[e].insert(pos, j);
        }
    }
    make_plan(s, t, &orders)
}

#[cfg(test)]
mod transit_coverage_tests {
    use super::*;
    use crate::{
        routing::Snapshot,
        transit_scope::Coverage,
        transit_snapshot::{Journey, Timetable, TransitLeg},
    };
    use std::sync::Arc;

    fn scenario() -> Scenario {
        let mut s = crate::import::demo();
        s.jobs.truncate(2);
        s.engineers.truncate(1);
        s.routing = None;
        s.transit_date = Some("2026-09-27".into());
        let e = &mut s.engineers[0];
        e.start = Point {
            lat: 55.0,
            lon: 37.0,
        };
        e.transport = Transport::Public;
        e.skills = vec![Skill::Local];
        e.shift_start = 480;
        e.shift_end = 1080;
        for (i, j) in s.jobs.iter_mut().enumerate() {
            j.point = Point {
                lat: 55.01 + i as f64 / 100.0,
                lon: 37.0,
            };
            j.transport = None;
            j.skill = if i == 0 {
                Skill::Local
            } else {
                Skill::Emergency
            };
            j.window_start = 600;
            j.window_end = 700;
            j.duration = 30;
        }
        s
    }

    fn snapshot(s: &Scenario, legacy: bool) -> Arc<Snapshot> {
        let points = vec![s.engineers[0].start, s.jobs[0].point, s.jobs[1].point];
        let mut timetable = Timetable::new(s.transit_date.clone().unwrap(), "fixture".into());
        timetable.coverage = crate::transit_scope::required(s, &points).unwrap();
        timetable.journeys = timetable
            .coverage
            .iter()
            .map(|row| row.iter().map(|cell| cell.map(|_| vec![])).collect())
            .collect();
        timetable.journeys[0][1] = Some(vec![Journey {
            departure: 600,
            arrival: 610,
            leg: TransitLeg {
                metres: 100,
                minutes: 10,
                shape: "??AA".into(),
            },
            description: "fixture".into(),
        }]);
        if legacy {
            let mut stored = serde_json::to_value(&timetable).unwrap();
            stored.as_object_mut().unwrap().remove("coverage");
            timetable = serde_json::from_value(stored).unwrap();
        }
        timetable.validate(points.len()).unwrap();
        let mut profile = timetable.profile();
        profile.lower = vec![vec![0; points.len()]; points.len()];
        Arc::new(Snapshot {
            points,
            profiles: vec![profile],
            transit: Some(timetable),
        })
    }

    #[test]
    fn edited_windows_shifts_and_skills_require_preparation_not_false_unassignment() {
        let s = scenario();
        let snapshot = snapshot(&s, false);
        let travel = Travel::city(&s, Arc::clone(&snapshot)).unwrap();
        assert_eq!(
            schedule(&s, &travel, 0, &[0]).unwrap().stops[0].arrival,
            610
        );
        let mut later = s.clone();
        later.jobs[0].window_end += 1;
        let mut earlier = s.clone();
        earlier.engineers[0].shift_start -= 1;
        let mut skills = s.clone();
        skills.engineers[0].skills.push(Skill::Emergency);
        for changed in [later, earlier, skills] {
            let error = Travel::city(&changed, Arc::clone(&snapshot)).err().unwrap();
            assert!(error.contains("prepare city routing again"), "{error}");
        }
    }

    #[test]
    fn extending_shift_requires_coverage_when_service_end_was_the_limit() {
        let mut s = scenario();
        s.jobs[0].window_end = 1440;
        let snapshot = snapshot(&s, false);
        s.engineers[0].shift_end += 1;
        let error = Travel::city(&s, snapshot).err().unwrap();
        assert!(error.contains("prepare city routing again"), "{error}");
    }

    #[test]
    fn legacy_snapshot_replays_after_window_expansion_with_full_day_coverage() {
        let mut s = scenario();
        let snapshot = snapshot(&s, true);
        assert_eq!(
            snapshot.transit.as_ref().unwrap().coverage[0][1],
            Some(Coverage {
                start: 0,
                end: 1440
            })
        );
        s.jobs[0].window_end = 1000;
        s.engineers[0].shift_start = 0;
        let travel = Travel::city(&s, snapshot).unwrap();
        let route = schedule(&s, &travel, 0, &[0]).unwrap();
        assert_eq!(route.stops[0].arrival, 610);
        assert_eq!(route.stops[0].start, 610);
    }
}
