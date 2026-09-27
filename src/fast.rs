//! Route-column SAT: schedule candidate routes once, then solve a much smaller
//! exact-cover problem. Global proofs require exhaustive timetable enumeration.
use crate::{
    model::*,
    sat::{Cnf, Stage, Stats},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

struct Candidate {
    engineer: usize,
    jobs: Vec<usize>,
    distance: u32,
}
struct Pool {
    routes: Vec<Candidate>,
    index: HashMap<(usize, u128), usize>,
    stored_jobs: usize,
}
impl Pool {
    fn add(&mut self, s: &Scenario, t: &Travel, e: usize, jobs: &[usize]) {
        if jobs.is_empty() {
            return;
        }
        let Some(distance) = route_cost(s, t, e, jobs) else {
            return;
        };
        let mask = jobs.iter().fold(0, |mask, &j| mask | 1u128 << j);
        self.add_feasible(e, jobs, mask, distance);
    }
    fn add_feasible(&mut self, e: usize, jobs: &[usize], mask: u128, distance: u32) {
        if let Some(&index) = self.index.get(&(e, mask)) {
            // Only completed columns are dominated, never prefixes during enumeration.
            if distance < self.routes[index].distance {
                self.routes[index] = Candidate {
                    engineer: e,
                    jobs: jobs.to_vec(),
                    distance,
                };
            }
        } else {
            self.index.insert((e, mask), self.routes.len());
            self.stored_jobs += jobs.len();
            self.routes.push(Candidate {
                engineer: e,
                jobs: jobs.to_vec(),
                distance,
            });
        }
    }
    fn add_plan(
        &mut self,
        s: &Scenario,
        t: &Travel,
        p: &Plan,
        ids: &HashMap<&str, usize>,
        subroutes: bool,
    ) {
        for (e, route) in p.routes.iter().enumerate() {
            let jobs: Vec<_> = route.stops.iter().map(|s| ids[s.job_id.as_str()]).collect();
            // A useful job sequence is not tied to the engineer that generated it.
            // Recheck every alternative with its own skills, shifts and timetable.
            for target in 0..s.engineers.len() {
                self.add(s, t, target, &jobs);
            }
            if subroutes {
                for removed in 0..jobs.len() {
                    let mut subset = jobs.clone();
                    subset.remove(removed);
                    self.add(s, t, e, &subset);
                }
            }
        }
    }
    fn decode(&self, c: &Cnf, choices: &[i32], s: &Scenario, t: &Travel) -> Result<Plan> {
        let mut orders = vec![vec![]; s.engineers.len()];
        for (r, &lit) in self.routes.iter().zip(choices) {
            if c.value(lit) {
                orders[r.engineer] = r.jobs.clone();
            }
        }
        make_plan(s, t, &orders)
    }
}

// Bound both columns and their total incidence count, including the solver's
// per-job clauses. DFS itself retains only one route of at most 100 jobs.
const MAX_TIMETABLE_ROUTES: usize = 16_384;
const MAX_TIMETABLE_STOPS: usize = 131_072;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Enumeration {
    NotRequested,
    Complete,
    TimeLimit,
    MemoryLimit,
}

#[derive(Clone, Copy)]
struct EnumerationBudget {
    deadline: Instant,
    routes: usize,
    stops: usize,
}

fn enumerate_routes(
    s: &Scenario,
    t: &Travel,
    pool: &mut Pool,
    e: usize,
    jobs: &mut Vec<usize>,
    mask: u128,
    budget: EnumerationBudget,
) -> Enumeration {
    for j in 0..s.jobs.len() {
        let bit = 1u128 << j;
        if mask & bit != 0 || !compatible(&s.engineers[e], &s.jobs[j]) {
            continue;
        }
        if Instant::now() >= budget.deadline {
            return Enumeration::TimeLimit;
        }
        jobs.push(j);
        if let Some(distance) = route_cost(s, t, e, jobs) {
            let next = mask | bit;
            if !pool.index.contains_key(&(e, next))
                && (pool.routes.len() >= budget.routes
                    || pool.stored_jobs + jobs.len() > budget.stops)
            {
                jobs.pop();
                return Enumeration::MemoryLimit;
            }
            pool.add_feasible(e, jobs, next, distance);
            // Even a longer ordering of the same set may have a different endpoint
            // or ready time and be the only prefix that catches the next departure.
            let result = enumerate_routes(s, t, pool, e, jobs, next, budget);
            if result != Enumeration::Complete {
                jobs.pop();
                return result;
            }
        }
        // An infeasible prefix cannot become feasible by appending more visits.
        jobs.pop();
    }
    Enumeration::Complete
}

pub(crate) fn uses_timetable(s: &Scenario) -> bool {
    s.routing.is_some() && s.engineers.iter().any(|e| e.transport == Transport::Public)
}

pub fn optimize(s: &Scenario, seconds: f64) -> Result<(Plan, Plan, Stats)> {
    if uses_timetable(s) {
        return optimize_timetable(s, seconds, false);
    }
    // Small instances are cheap enough to solve globally with the full encoding.
    if s.jobs.len() <= 16 {
        return crate::sat::optimize(s, seconds);
    }
    optimize_pool(s, seconds, None)
}

/// Timetable routes use actual ready times, never the static SAT travel bounds.
/// Exhaustive requests spend their remaining search budget on proofs; incomplete
/// enumeration still returns a validated incumbent with candidate-only bounds.
pub fn optimize_timetable(
    s: &Scenario,
    seconds: f64,
    exhaustive: bool,
) -> Result<(Plan, Plan, Stats)> {
    optimize_pool(s, seconds, Some(exhaustive))
}

fn optimize_pool(
    s: &Scenario,
    seconds: f64,
    timetable: Option<bool>,
) -> Result<(Plan, Plan, Stats)> {
    validate_input(s)?;
    if !seconds.is_finite() || !(0.01..=300.0).contains(&seconds) {
        return Err("seconds must be between 0.01 and 300".into());
    }
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(seconds);
    let t = Travel::new(s)?;
    let baseline = greedy(s, &t, false, None)?;
    let heuristic_deadline = if timetable.is_some() {
        deadline.min(started + Duration::from_secs_f64(seconds * 0.15))
    } else {
        deadline
    };
    let mut best = greedy(s, &t, true, Some(heuristic_deadline))?;
    if baseline.metrics.key() < best.metrics.key() {
        best = baseline.clone();
    }
    let mut stats = Stats {
        scope: "candidate_routes",
        ..Stats::default()
    };
    let ids: HashMap<_, _> = s
        .jobs
        .iter()
        .enumerate()
        .map(|(j, job)| (job.id.as_str(), j))
        .collect();
    let mut pool = Pool {
        routes: vec![],
        index: HashMap::new(),
        stored_jobs: 0,
    };
    pool.add_plan(s, &t, &baseline, &ids, true);
    pool.add_plan(s, &t, &best, &ids, true);
    if s.jobs.len() > 8 {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64();
        let seed_seconds = (seconds * 0.35).min(remaining * 0.5);
        let seed = if timetable.is_some() {
            crate::mixed_seed::road_only(s, seed_seconds)?
        } else if seed_seconds >= 0.01 {
            Some(crate::sat::optimize(s, seed_seconds)?.0)
        } else {
            None
        };
        if let Some(candidate) = seed {
            pool.add_plan(s, &t, &candidate, &ids, true);
            if candidate.metrics.key() < best.metrics.key() {
                best = candidate;
            }
        }
    }
    if s.jobs.len() > 8 {
        let repair_deadline =
            deadline.min(Instant::now() + Duration::from_secs_f64(seconds * 0.08));
        crate::sat::polish(s, &t, &mut best, repair_deadline)?;
        pool.add_plan(s, &t, &best, &ids, true);
    }
    for e in 0..s.engineers.len() {
        for j in 0..s.jobs.len() {
            pool.add(s, &t, e, &[j]);
        }
    }
    let mut enumeration = Enumeration::NotRequested;
    // Search diverse complete plans before exhaustive enumeration can consume its
    // budget on the first engineer. SAT then recombines and reassigns these routes.
    if timetable.is_none() || s.jobs.len() > 8 {
        let generation_deadline = deadline.min(started + Duration::from_secs_f64(seconds * 0.6));
        for seed in 1..=256 {
            if Instant::now() >= generation_deadline
                || pool.routes.len() >= MAX_TIMETABLE_ROUTES
                || pool.stored_jobs >= MAX_TIMETABLE_STOPS
            {
                break;
            }
            let mut candidate = greedy_variant(s, &t, seed, generation_deadline)?;
            pool.add_plan(s, &t, &candidate, &ids, false);
            if candidate.metrics.key()[..3] <= best.metrics.key()[..3] {
                let repair_deadline = generation_deadline
                    .min(Instant::now() + Duration::from_secs_f64((seconds * 0.005).min(0.02)));
                crate::sat::polish(s, &t, &mut candidate, repair_deadline)?;
                pool.add_plan(s, &t, &candidate, &ids, false);
            }
            if candidate.metrics.key() < best.metrics.key() {
                best = candidate;
            }
        }
    }
    if timetable.is_some_and(|exhaustive| exhaustive || s.jobs.len() <= 8) {
        // Leave time for encoding and exact cover, even when route generation is
        // factorial. Both public and road engineers are enumerated in mixed fleets.
        let fraction = if timetable == Some(true) { 0.65 } else { 0.35 };
        let generation_deadline = started + Duration::from_secs_f64(seconds * fraction);
        enumeration = Enumeration::Complete;
        let mut jobs = Vec::with_capacity(s.jobs.len());
        for e in 0..s.engineers.len() {
            let remaining = s.engineers.len() - e;
            let time_left = generation_deadline.saturating_duration_since(Instant::now());
            if time_left.is_zero() {
                enumeration = Enumeration::TimeLimit;
                break;
            }
            // One engineer's factorial subtree must not starve the rest of the fleet.
            let budget = EnumerationBudget {
                deadline: Instant::now() + time_left / remaining as u32,
                routes: pool.routes.len()
                    + MAX_TIMETABLE_ROUTES.saturating_sub(pool.routes.len()) / remaining,
                stops: pool.stored_jobs
                    + MAX_TIMETABLE_STOPS.saturating_sub(pool.stored_jobs) / remaining,
            };
            let status = enumerate_routes(s, &t, &mut pool, e, &mut jobs, 0, budget);
            if status != Enumeration::Complete {
                enumeration = status;
            }
        }
    }
    stats.generation_ms = started.elapsed().as_millis();
    stats.candidate_routes = pool.routes.len();
    let encoding_started = Instant::now();
    let mut c = Cnf::new(deadline);
    let choices: Vec<_> = pool.routes.iter().map(|_| c.var()).collect();
    let unassigned: Vec<_> = s.jobs.iter().map(|_| c.var()).collect();
    let used: Vec<_> = s.engineers.iter().map(|_| c.var()).collect();
    let mut per_job = vec![vec![]; s.jobs.len()];
    let mut per_engineer = vec![vec![]; s.engineers.len()];
    for (r, &lit) in pool.routes.iter().zip(&choices) {
        for &j in &r.jobs {
            per_job[j].push(lit);
        }
        per_engineer[r.engineer].push(lit);
    }
    for (j, literals) in per_job.iter_mut().enumerate() {
        literals.push(unassigned[j]);
        c.one_if(literals, 1);
    }
    for (e, literals) in per_engineer.iter().enumerate() {
        c.one_if(literals, used[e]);
    }
    stats.encoding_ms = encoding_started.elapsed().as_millis();
    // Set initial phases using a complete feasible plan already represented in the pool.
    let mut assumptions = vec![];
    for (e, r) in best.routes.iter().enumerate() {
        let mask = r
            .stops
            .iter()
            .fold(0, |mask, stop| mask | 1u128 << ids[stop.job_id.as_str()]);
        if mask == 0 {
            assumptions.push(-used[e]);
        } else {
            assumptions.push(choices[pool.index[&(e, mask)]]);
        }
    }
    if Instant::now() < deadline {
        stats.sat_calls += 1;
        match c.solve_with(assumptions, -1) {
            Some(false) => return Err("Route pool rejected its feasible incumbent".into()),
            Some(true) => {
                let candidate = pool.decode(&c, &choices, s, &t)?;
                if candidate.metrics.key() < best.metrics.key() {
                    best = candidate;
                }
            }
            None => {}
        }
    }
    let already_used = s.engineers.iter().filter(|e| e.already_used).count() as u32;
    for (index, criterion) in ["urgent_unassigned", "unassigned", "engineers", "distance_m"]
        .iter()
        .enumerate()
    {
        let offset = if index == 2 { already_used } else { 0 };
        let mut lower = offset;
        let mut upper = best.metrics.key()[index];
        let terms: Vec<_> = match index {
            0 => unassigned
                .iter()
                .zip(&s.jobs)
                .filter(|(_, j)| j.urgent)
                .map(|(&x, _)| (x, 1))
                .collect(),
            1 => unassigned.iter().map(|&x| (x, 1)).collect(),
            2 => used
                .iter()
                .zip(&s.engineers)
                .filter(|(_, e)| !e.already_used)
                .map(|(&x, _)| (x, 1))
                .collect(),
            _ => choices
                .iter()
                .zip(&pool.routes)
                .filter(|(_, r)| r.distance > 0)
                .map(|(&x, r)| (x, r.distance))
                .collect(),
        };
        if upper == offset {
            for &(lit, _) in &terms {
                c.clause(&[-lit]);
            }
        } else if let Some(bits) = c.sum(&terms) {
            while lower < upper && Instant::now() < deadline {
                let bound = upper - 1;
                let limit = c.bound(&bits, (bound - offset) as u64);
                stats.sat_calls += 1;
                // The caller's deadline bounds proof effort, not a fixed conflict cap.
                match c.solve_with([limit], -1) {
                    Some(true) => {
                        let candidate = pool.decode(&c, &choices, s, &t)?;
                        let actual = candidate.metrics.key()[index];
                        if actual > bound {
                            return Err("Route-pool objective mismatch".into());
                        }
                        upper = actual;
                        if candidate.metrics.key() < best.metrics.key() {
                            best = candidate;
                        }
                    }
                    Some(false) => lower = bound + 1,
                    None => break,
                }
            }
            if lower == upper {
                let fixed = c.bound(&bits, (upper - offset) as u64);
                c.clause(&[fixed]);
            }
        }
        stats.stages.push(Stage {
            criterion: criterion.to_string(),
            lower,
            upper,
            proven: lower == upper,
        });
        if lower != upper {
            break;
        }
    }
    stats.search_complete = stats.stages.len() == 4 && stats.stages.iter().all(|s| s.proven);
    stats.optimal = enumeration == Enumeration::Complete && stats.search_complete;
    if stats.optimal {
        stats.scope = "global";
    }
    stats.status = if timetable.is_some() {
        let coverage = match enumeration {
            Enumeration::Complete => "All feasible ordered routes enumerated",
            Enumeration::TimeLimit => "Route enumeration incomplete: generation time limit reached",
            Enumeration::MemoryLimit => {
                "Route enumeration incomplete: route-pool memory cap reached"
            }
            Enumeration::NotRequested => "Bounded route pool; exhaustive enumeration not requested",
        };
        let proof = if stats.optimal {
            "global lexicographic timetable optimum proven"
        } else if stats.search_complete {
            "candidate-route optimum proven only; no global proof or global lower bounds"
        } else {
            "search effort/time limit reached; candidate-only bounds, no global proof"
        };
        format!("{coverage}; {proof}; best validated plan returned")
    } else if stats.search_complete {
        "Candidate-route optimum proven; global optimality is NOT claimed".into()
    } else {
        "Fast search stopped at its effort/time limit; best validated plan returned, no global proof"
            .into()
    };
    stats.variables = c.vars;
    stats.clauses = c.clauses;
    validate_plan(s, &best)?;
    stats.elapsed_ms = started.elapsed().as_millis();
    Ok((best, baseline, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{test_snapshot, Leg, Profile, Snapshot};

    #[test]
    fn exhaustive_routes_extend_nonminimal_orderings() {
        let mut s = crate::import::demo();
        s.jobs.truncate(3);
        s.engineers.truncate(1);
        s.engineers[0].skills = vec![Skill::Local, Skill::Connection, Skill::Emergency];
        s.engineers[0].transport = Transport::Car;
        s.engineers[0].shift_start = 540;
        s.engineers[0].shift_end = 550;
        for job in &mut s.jobs {
            job.duration = 1;
            job.window_start = 540;
            job.window_end = 550;
            job.transport = None;
        }
        let mut points: Vec<_> = s.jobs.iter().map(|job| job.point).collect();
        points.push(s.engineers[0].start);
        let leg = |minutes| {
            Some(Leg {
                minutes,
                metres: minutes * 10,
                shape: "??AA".into(),
            })
        };
        let mut legs = vec![vec![None; 4]; 4];
        for (i, row) in legs.iter_mut().enumerate() {
            row[i] = leg(0);
        }
        legs[3][0] = leg(1);
        legs[3][1] = leg(2);
        legs[0][1] = leg(1);
        legs[1][0] = leg(1);
        legs[0][2] = leg(1);
        legs[1][2] = leg(20);
        s.routing = Some(test_snapshot(Snapshot {
            points,
            profiles: vec![Profile {
                transport: Transport::Car,
                legs,
                lower: vec![],
            }],
            transit: None,
        }));
        // [0, 1] is cheaper than [1, 0], but only the latter can serve job 2.
        // A job-set-dominated prefix must still be extended by the exact search.
        let (plan, baseline, stats) = optimize_timetable(&s, 2.0, true).unwrap();
        validate_plan(&s, &plan).unwrap();
        assert_eq!(baseline.metrics.unassigned, 1);
        assert_eq!(plan.metrics.key(), [0, 0, 1, 40]);
        assert_eq!(
            plan.routes[0]
                .stops
                .iter()
                .map(|stop| stop.job_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                s.jobs[1].id.as_str(),
                s.jobs[0].id.as_str(),
                s.jobs[2].id.as_str()
            ]
        );
        assert!(stats.optimal && stats.search_complete);
        assert_eq!(stats.scope, "global");
    }
}
