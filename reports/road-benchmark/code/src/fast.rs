//! Route-column SAT: schedule candidate routes once, then solve a much smaller
//! exact-cover problem. Proofs apply ONLY to the generated route pool.
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
}
impl Pool {
    fn add(&mut self, s: &Scenario, t: &Travel, e: usize, jobs: Vec<usize>) {
        if jobs.is_empty() {
            return;
        }
        let Some(distance) = route_cost(s, t, e, &jobs) else {
            return;
        };
        let mask = jobs.iter().fold(0, |mask, &j| mask | 1u128 << j);
        let candidate = Candidate {
            engineer: e,
            jobs,
            distance,
        };
        if let Some(&index) = self.index.get(&(e, mask)) {
            // Same engineer + same job set: the shorter feasible ordering dominates.
            if distance < self.routes[index].distance {
                self.routes[index] = candidate;
            }
        } else {
            self.index.insert((e, mask), self.routes.len());
            self.routes.push(candidate);
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
            self.add(s, t, e, jobs.clone());
            if subroutes {
                for removed in 0..jobs.len() {
                    let mut subset = jobs.clone();
                    subset.remove(removed);
                    self.add(s, t, e, subset);
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

pub fn optimize(s: &Scenario, seconds: f64) -> Result<(Plan, Plan, Stats)> {
    // Small instances are cheap enough to solve globally with the full encoding.
    if s.jobs.len() <= 16 {
        return crate::sat::optimize(s, seconds);
    }
    validate_input(s)?;
    if !seconds.is_finite() || !(0.01..=300.0).contains(&seconds) {
        return Err("seconds must be between 0.01 and 300".into());
    }
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(seconds);
    let t = Travel::new(s)?;
    let baseline = greedy(s, &t, false, None)?;
    let mut best = greedy(s, &t, true, Some(deadline))?;
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
    };
    pool.add_plan(s, &t, &baseline, &ids, true);
    pool.add_plan(s, &t, &best, &ids, true);
    for e in 0..s.engineers.len() {
        for j in 0..s.jobs.len() {
            pool.add(s, &t, e, vec![j]);
        }
    }
    // ponytail: a bounded candidate pool, not exhaustive route enumeration. Use
    // full SAT mode when proofs over every possible route are required.
    let generation_deadline =
        deadline.min(started + Duration::from_secs_f64((seconds * 0.25).min(0.05)));
    for seed in 1..=24 {
        if Instant::now() >= generation_deadline {
            break;
        }
        let candidate = greedy_variant(s, &t, seed, generation_deadline)?;
        pool.add_plan(s, &t, &candidate, &ids, false);
        if candidate.metrics.key() < best.metrics.key() {
            best = candidate;
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
                // Responsiveness: stop a difficult proof instead of burning the
                // entire wall budget; UNKNOWN is never treated as UNSAT.
                match c.solve_with([limit], 500) {
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
    stats.status = if stats.search_complete {
        "Candidate-route optimum proven; global optimality is NOT claimed"
    } else {
        "Fast search stopped at its effort/time limit; best validated plan returned, no global proof"
    }.into();
    stats.variables = c.vars;
    stats.clauses = c.clauses;
    validate_plan(s, &best)?;
    stats.elapsed_ms = started.elapsed().as_millis();
    Ok((best, baseline, stats))
}
