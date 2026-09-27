use crate::model::*;
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

type Lit = i32;
const TRUE: Lit = 1;
const FALSE: Lit = -1;

struct Deadline(Instant);
impl cadical::Callbacks for Deadline {
    fn terminate(&mut self) -> bool {
        Instant::now() >= self.0
    }
}
pub(crate) struct Cnf {
    solver: cadical::Solver<Deadline>,
    pub(crate) vars: Lit,
    pub(crate) clauses: usize,
    deadline: Instant,
}
impl Cnf {
    pub(crate) fn new(deadline: Instant) -> Self {
        let mut solver = cadical::Solver::with_config("sat").expect("CaDiCaL SAT configuration");
        solver.set_callbacks(Some(Deadline(deadline)));
        let mut c = Self {
            solver,
            vars: 1,
            clauses: 0,
            deadline,
        };
        c.clause(&[TRUE]);
        c
    }
    pub(crate) fn var(&mut self) -> Lit {
        self.vars += 1;
        self.vars
    }
    pub(crate) fn clause(&mut self, xs: &[Lit]) {
        self.solver.add_clause(xs.iter().copied());
        self.clauses += 1;
    }
    fn and(&mut self, a: Lit, b: Lit) -> Lit {
        if a == FALSE || b == FALSE || a == -b {
            return FALSE;
        }
        if a == TRUE || a == b {
            return b;
        }
        if b == TRUE {
            return a;
        }
        let q = self.var();
        self.clause(&[-q, a]);
        self.clause(&[-q, b]);
        self.clause(&[q, -a, -b]);
        q
    }
    fn or(&mut self, a: Lit, b: Lit) -> Lit {
        -self.and(-a, -b)
    }
    fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        if a == FALSE {
            return b;
        }
        if b == FALSE {
            return a;
        }
        if a == TRUE {
            return -b;
        }
        if b == TRUE {
            return -a;
        }
        if a == b {
            return FALSE;
        }
        if a == -b {
            return TRUE;
        }
        let q = self.var();
        self.clause(&[-a, -b, -q]);
        self.clause(&[a, b, -q]);
        self.clause(&[a, -b, q]);
        self.clause(&[-a, b, q]);
        q
    }
    // Sinz-style linear at-most-one; tiny lists use cheaper pairwise clauses.
    fn amo(&mut self, xs: &[Lit]) {
        if xs.len() <= 4 {
            for i in 0..xs.len() {
                for j in 0..i {
                    self.clause(&[-xs[i], -xs[j]]);
                }
            }
        } else {
            let mut prefix = xs[0];
            for &x in &xs[1..] {
                self.clause(&[-prefix, -x]);
                prefix = self.or(prefix, x);
            }
        }
    }
    pub(crate) fn one_if(&mut self, xs: &[Lit], active: Lit) {
        let mut clause = vec![-active];
        clause.extend(xs);
        self.clause(&clause);
        for &x in xs {
            self.clause(&[-x, active]);
        }
        self.amo(xs);
    }
    fn constant(n: u64, width: usize) -> Vec<Lit> {
        (0..width)
            .map(|b| if (n >> b) & 1 == 1 { TRUE } else { FALSE })
            .collect()
    }
    // Full-width addition; the carry is retained (never modular arithmetic).
    fn add(&mut self, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
        let mut carry = FALSE;
        let mut out = vec![];
        for i in 0..a.len().max(b.len()) {
            let a = *a.get(i).unwrap_or(&FALSE);
            let b = *b.get(i).unwrap_or(&FALSE);
            let x = self.xor(a, b);
            out.push(self.xor(x, carry));
            let ab = self.and(a, b);
            let xc = self.and(x, carry);
            carry = self.or(ab, xc);
        }
        out.push(carry);
        while out.len() > 1 && out.last() == Some(&FALSE) {
            out.pop();
        }
        out
    }
    fn leq(&mut self, a: &[Lit], b: &[Lit]) -> Lit {
        let mut le = TRUE;
        // Low-to-high: each more significant unequal bit overrides lower bits.
        for i in 0..a.len().max(b.len()) {
            let a = *a.get(i).unwrap_or(&FALSE);
            let b = *b.get(i).unwrap_or(&FALSE);
            let less = self.and(-a, b);
            let equal = -self.xor(a, b);
            let rest = self.and(equal, le);
            le = self.or(less, rest);
        }
        le
    }
    pub(crate) fn bound(&mut self, bits: &[Lit], n: u64) -> Lit {
        let width = bits.len().max(64 - n.leading_zeros() as usize).max(1);
        self.leq(bits, &Self::constant(n, width))
    }
    pub(crate) fn sum(&mut self, terms: &[(Lit, u32)]) -> Option<Vec<Lit>> {
        let mut level: Vec<Vec<Lit>> = terms
            .iter()
            .filter(|(_, w)| *w > 0)
            .map(|&(x, w)| {
                (0..32 - w.leading_zeros())
                    .map(|i| if (w >> i) & 1 == 1 { x } else { FALSE })
                    .collect()
            })
            .collect();
        if level.is_empty() {
            return Some(vec![FALSE]);
        }
        while level.len() > 1 {
            let mut next = Vec::with_capacity(level.len().div_ceil(2));
            for pair in level.chunks(2) {
                if Instant::now() >= self.deadline {
                    return None;
                }
                next.push(if pair.len() == 1 {
                    pair[0].clone()
                } else {
                    self.add(&pair[0], &pair[1])
                });
            }
            level = next;
        }
        level.pop()
    }
    // Unary thresholds for small cardinality objectives propagate staffing bounds
    // directly, unlike a binary adder whose carries hide them from unit propagation.
    fn count(&mut self, literals: &[Lit], limit: usize) -> Vec<Lit> {
        let mut previous: Vec<Lit> = vec![];
        for &x in literals {
            let mut next = Vec::with_capacity((previous.len() + 1).min(limit));
            for k in 0..(previous.len() + 1).min(limit) {
                let carry = if k == 0 {
                    x
                } else {
                    self.and(x, previous[k - 1])
                };
                next.push(self.or(*previous.get(k).unwrap_or(&FALSE), carry));
            }
            previous = next;
        }
        previous
    }
    pub(crate) fn solve_with(
        &mut self,
        assumptions: impl IntoIterator<Item = Lit>,
        conflicts: i32,
    ) -> Option<bool> {
        self.solver
            .set_limit("conflicts", conflicts)
            .expect("CaDiCaL conflict limit");
        self.solver.solve_with(assumptions)
    }
    pub(crate) fn value(&self, x: Lit) -> bool {
        self.solver.value(x.abs()).unwrap_or(false) == (x > 0)
    }
}

// Order encoding: bits[k] means start >= lo+k+1. Monotone thresholds let
// unit propagation move entire time bounds, rather than guessing binary digits.
struct Time {
    lo: u32,
    hi: u32,
    bits: Vec<Lit>,
}
impl Time {
    fn lower(&self, bound: u32) -> Lit {
        if bound <= self.lo {
            TRUE
        } else if bound > self.hi {
            FALSE
        } else {
            self.bits[(bound - self.lo - 1) as usize]
        }
    }
    fn upper(&self, bound: u32) -> Lit {
        if bound >= self.hi {
            TRUE
        } else if bound < self.lo {
            FALSE
        } else {
            -self.bits[(bound - self.lo) as usize]
        }
    }
    fn value(&self, c: &Cnf) -> u32 {
        self.lo + self.bits.iter().filter(|&&lit| c.value(lit)).count() as u32
    }
}
fn precedence(c: &mut Cnf, a: &Time, b: &Time, delta: u32) -> Lit {
    if a.lo + delta > b.hi {
        return FALSE;
    }
    if a.hi + delta <= b.lo {
        return TRUE;
    }
    let before = c.var();
    let first = b.lower(a.lo + delta);
    let last = a.upper(b.hi - delta);
    c.clause(&[-before, first]);
    c.clause(&[-before, last]);
    for time in (a.lo + 1).max(b.lo.saturating_sub(delta) + 1)..=a.hi.min(b.hi - delta) {
        let premise = a.lower(time);
        let consequent = b.lower(time + delta);
        c.clause(&[-before, -premise, consequent]);
    }
    before
}

struct Arc {
    e: usize,
    from: Option<usize>,
    to: Option<usize>,
    lit: Lit,
    distance: u32,
}
struct Encoding {
    cnf: Cnf,
    arcs: Vec<Arc>,
    routed: bool,
    assignment: Vec<Vec<Lit>>,
    unassigned: Vec<Lit>,
    used: Vec<Lit>,
    times: Vec<Time>,
    precedences: HashMap<(usize, usize, u32), Lit>,
}
impl Encoding {
    fn build(s: &Scenario, t: &Travel, deadline: Instant) -> Option<Self> {
        let n = s.jobs.len();
        let m = s.engineers.len();
        let mut c = Cnf::new(deadline);
        let unassigned: Vec<_> = (0..n).map(|_| c.var()).collect();
        let used: Vec<_> = (0..m).map(|_| c.var()).collect();
        let mut assignment = vec![vec![FALSE; n]; m];
        let mut earliest = vec![vec![0; n]; m];
        let mut latest = earliest.clone();
        let mut times = vec![];
        for (j, job) in s.jobs.iter().enumerate() {
            let mut lo = u32::MAX;
            let mut hi = 0;
            for (e, engineer) in s.engineers.iter().enumerate() {
                if !compatible(engineer, job) || job.duration > engineer.shift_end {
                    continue;
                }
                let a = job
                    .window_start
                    .max(engineer.shift_start + t.start_lower[e][j]);
                let b = job.window_end.min(engineer.shift_end - job.duration);
                if a > b {
                    continue;
                }
                assignment[e][j] = c.var();
                earliest[e][j] = a;
                latest[e][j] = b;
                lo = lo.min(a);
                hi = hi.max(b);
            }
            if lo == u32::MAX {
                lo = job.window_start;
                hi = lo; // No compatible engineer: this unassigned job needs no time variables.
            }
            let span = hi - lo;
            let bits: Vec<_> = (0..span).map(|_| c.var()).collect();
            for pair in bits.windows(2) {
                c.clause(&[-pair[1], pair[0]]);
            }
            times.push(Time { lo, hi, bits });
            let mut choices = vec![unassigned[j]];
            choices.extend((0..m).map(|e| assignment[e][j]).filter(|&x| x != FALSE));
            c.one_if(&choices, TRUE);
        }
        let mut intervals = vec![];
        if !t.metric {
            let mut starts: Vec<_> = s.jobs.iter().map(|j| j.window_start).collect();
            starts.sort_unstable();
            starts.dedup();
            for a in starts {
                if Instant::now() >= deadline {
                    return None;
                }
                let best = s
                    .jobs
                    .iter()
                    .map(|j| j.window_end + j.duration)
                    .filter(|&b| b > a)
                    .map(|b| {
                        let work: u32 = s
                            .jobs
                            .iter()
                            .map(|j| mandatory_work(j.window_start, j.window_end, j.duration, a, b))
                            .sum();
                        (b, work)
                    })
                    .max_by(|&(b, w), &(d, v)| {
                        (u64::from(w) * u64::from(d - a)).cmp(&(u64::from(v) * u64::from(b - a)))
                    });
                if let Some((b, w)) = best {
                    if w > 0 {
                        intervals.push((a, b, w));
                    }
                }
            }
        }
        // These are optional propagation cuts, not a coarser time grid.
        if intervals.len() > 8 {
            intervals.sort_by(|&(a, b, w), &(c, d, v)| {
                (u64::from(v) * u64::from(b - a)).cmp(&(u64::from(w) * u64::from(d - c)))
            });
            intervals.truncate(8);
            intervals.sort_unstable_by_key(|&(a, _, _)| a);
        }
        let mut precedences = HashMap::new();
        let mut cut_variables = 0;
        for e in 0..m {
            if Instant::now() >= deadline {
                return None;
            }
            let mut active = vec![-used[e]];
            for j in 0..n {
                if Instant::now() >= deadline {
                    return None;
                }
                let x = assignment[e][j];
                if x == FALSE {
                    continue;
                }
                active.push(x);
                c.clause(&[-x, used[e]]);
                let lo = times[j].lower(earliest[e][j]);
                let hi = times[j].upper(latest[e][j]);
                c.clause(&[-x, lo]);
                c.clause(&[-x, hi]);
                for i in 0..j {
                    if assignment[e][i] == FALSE {
                        continue;
                    }
                    let ij = s.jobs[i].duration + t.time_lower[e][i][j];
                    let ji = s.jobs[j].duration + t.time_lower[e][j][i];
                    // Disjoint windows often make the entire disjunction tautological.
                    if latest[e][i] + ij <= earliest[e][j] || latest[e][j] + ji <= earliest[e][i] {
                        continue;
                    }
                    let forward = if earliest[e][i] + ij > latest[e][j] {
                        FALSE
                    } else {
                        *precedences
                            .entry((i, j, ij))
                            .or_insert_with(|| precedence(&mut c, &times[i], &times[j], ij))
                    };
                    let backward = if earliest[e][j] + ji > latest[e][i] {
                        FALSE
                    } else {
                        *precedences
                            .entry((j, i, ji))
                            .or_insert_with(|| precedence(&mut c, &times[j], &times[i], ji))
                    };
                    c.clause(&[-assignment[e][i], -x, forward, backward]);
                }
            }
            c.clause(&active);
            for &(a, b, _) in &intervals {
                if cut_variables >= 100_000 {
                    break;
                } // Optional cuts must not dominate model size.
                if Instant::now() >= deadline {
                    return None;
                }
                let terms: Vec<_> = (0..n)
                    .filter(|&j| assignment[e][j] != FALSE)
                    .map(|j| {
                        (
                            assignment[e][j],
                            mandatory_work(earliest[e][j], latest[e][j], s.jobs[j].duration, a, b),
                        )
                    })
                    .filter(|&(_, w)| w > 0)
                    .collect();
                let capacity = s.engineers[e]
                    .shift_end
                    .min(b)
                    .saturating_sub(s.engineers[e].shift_start.max(a));
                let mut weights: Vec<_> = terms.iter().map(|&(_, w)| w).collect();
                weights.sort_unstable();
                weights.dedup();
                // Each selected visit contributes >= w minutes in this interval.
                // Hence at most floor(capacity / w) such visits can share an engineer.
                for w in weights {
                    if cut_variables >= 100_000 {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return None;
                    }
                    let literals: Vec<_> = terms
                        .iter()
                        .filter(|&&(_, v)| v >= w)
                        .map(|&(x, _)| x)
                        .collect();
                    let limit = (capacity / w) as usize;
                    if literals.len() > limit {
                        let before = c.vars;
                        let counts = c.count(&literals, limit + 1);
                        cut_variables += c.vars - before;
                        c.clause(&[-counts[limit]]);
                    }
                }
            }
        }
        let encoding = Self {
            cnf: c,
            arcs: vec![],
            routed: false,
            assignment,
            unassigned,
            used,
            times,
            precedences,
        };
        Some(encoding)
    }

    // Route arcs are unnecessary for coverage or staff: under metric travel,
    // pairwise non-overlap is equivalent to a feasible time-sorted route.
    // Nonmetric streets use replay/refinement until this handoff. Add ALL
    // feasible successor arcs when distance becomes the objective.
    fn add_routes(&mut self, s: &Scenario, t: &Travel) -> bool {
        if self.routed {
            return true;
        }
        let n = s.jobs.len();
        for (e, engineer) in s.engineers.iter().enumerate() {
            let mut incoming = vec![vec![]; n];
            let mut outgoing = incoming.clone();
            let mut first = vec![];
            let mut last = vec![];
            for (j, &x) in self.assignment[e].iter().enumerate() {
                if x == FALSE {
                    continue;
                }
                let start = self.cnf.var();
                let arrival = self.times[j].lower(engineer.shift_start + t.start_time[e][j]);
                self.cnf.clause(&[-start, arrival]);
                let end = self.cnf.var();
                first.push(start);
                last.push(end);
                incoming[j].push(start);
                outgoing[j].push(end);
                self.arcs.push(Arc {
                    e,
                    from: None,
                    to: Some(j),
                    lit: start,
                    distance: t.start_distance[e][j],
                });
                self.arcs.push(Arc {
                    e,
                    from: Some(j),
                    to: None,
                    lit: end,
                    distance: 0,
                });
            }
            for (i, out) in outgoing.iter_mut().enumerate() {
                if Instant::now() >= self.cnf.deadline {
                    return false;
                }
                if self.assignment[e][i] == FALSE {
                    continue;
                }
                for (j, incoming) in incoming.iter_mut().enumerate() {
                    if i == j || self.assignment[e][j] == FALSE {
                        continue;
                    }
                    let delta = s.jobs[i].duration + t.time[e][i][j];
                    let early = self.times[i]
                        .lo
                        .max(engineer.shift_start + t.start_lower[e][i]);
                    let late = self.times[j]
                        .hi
                        .min(engineer.shift_end - s.jobs[j].duration);
                    if early + delta > late {
                        continue;
                    }
                    let before = *self.precedences.entry((i, j, delta)).or_insert_with(|| {
                        precedence(&mut self.cnf, &self.times[i], &self.times[j], delta)
                    });
                    if before == FALSE {
                        continue;
                    }
                    let z = self.cnf.var();
                    self.cnf.clause(&[-z, before]);
                    out.push(z);
                    incoming.push(z);
                    self.arcs.push(Arc {
                        e,
                        from: Some(i),
                        to: Some(j),
                        lit: z,
                        distance: t.distance[e][i][j],
                    });
                }
            }
            self.cnf.one_if(&first, self.used[e]);
            self.cnf.one_if(&last, self.used[e]);
            for j in 0..n {
                if self.assignment[e][j] == FALSE {
                    continue;
                }
                self.cnf.one_if(&incoming[j], self.assignment[e][j]);
                self.cnf.one_if(&outgoing[j], self.assignment[e][j]);
            }
        }
        self.routed = true;
        true
    }
    fn distance_terms(&mut self) -> Vec<(Lit, u32)> {
        // At most one incoming arc per job globally. Equal-cost copies across
        // engineers therefore share one objective literal, not m weighted addends.
        let mut keys = HashMap::new();
        let mut groups: Vec<(Vec<Lit>, u32)> = vec![];
        for a in &self.arcs {
            if a.distance == 0 {
                continue;
            }
            let key = (a.from, a.to, a.distance);
            let index = *keys.entry(key).or_insert_with(|| {
                groups.push((vec![], a.distance));
                groups.len() - 1
            });
            groups[index].0.push(a.lit);
        }
        groups
            .into_iter()
            .map(|(lits, distance)| {
                if lits.len() == 1 {
                    return (lits[0], distance);
                }
                let any = self.cnf.var();
                let mut clause = vec![-any];
                clause.extend(&lits);
                self.cnf.clause(&clause);
                for lit in lits {
                    self.cnf.clause(&[-lit, any]);
                }
                (any, distance)
            })
            .collect()
    }
    fn ordered_assignments(&self) -> Vec<Vec<usize>> {
        self.assignment
            .iter()
            .map(|row| {
                let mut jobs: Vec<_> = row
                    .iter()
                    .enumerate()
                    .filter(|(_, x)| self.cnf.value(**x))
                    .map(|(j, _)| j)
                    .collect();
                jobs.sort_by_key(|&j| self.times[j].value(&self.cnf));
                jobs
            })
            .collect()
    }

    // The shortest-path timing closure is a RELAXATION, not a street schedule.
    // Cut only a failed engineer's exact job set AND order. A different set may
    // repair a nonmetric route through a new intermediate stop and must stay legal.
    fn refine_streets(&mut self, s: &Scenario, t: &Travel) -> bool {
        if self.routed || t.metric {
            return false;
        }
        // Snapshot every row before adding clauses invalidates the solver model.
        let orders = self.ordered_assignments();
        let mut refined = false;
        for (e, jobs) in orders.iter().enumerate() {
            if route_cost(s, t, e, jobs).is_some() {
                continue;
            }
            let mut clause: Vec<_> = self.assignment[e]
                .iter()
                .enumerate()
                .filter(|(_, x)| **x != FALSE)
                .map(|(j, &x)| if jobs.contains(&j) { -x } else { x })
                .collect();
            for pair in jobs.windows(2) {
                let (i, j) = (pair[1], pair[0]);
                let reversed = *self.precedences.entry((i, j, 1)).or_insert_with(|| {
                    precedence(&mut self.cnf, &self.times[i], &self.times[j], 1)
                });
                clause.push(reversed);
            }
            self.cnf.clause(&clause);
            refined = true;
        }
        refined
    }

    fn decode(&self, s: &Scenario, t: &Travel) -> Result<Plan> {
        if !self.routed {
            return make_plan(s, t, &self.ordered_assignments());
        }
        let mut next = vec![vec![None; s.jobs.len() + 1]; s.engineers.len()];
        for a in &self.arcs {
            if self.cnf.value(a.lit) {
                next[a.e][a.from.unwrap_or(s.jobs.len())] = a.to;
            }
        }
        let mut orders = vec![vec![]; s.engineers.len()];
        for e in 0..s.engineers.len() {
            let mut at = next[e][s.jobs.len()];
            while let Some(j) = at {
                if orders[e].len() >= s.jobs.len() {
                    return Err("SAT route contains a cycle".into());
                }
                orders[e].push(j);
                at = next[e][j];
            }
        }
        let p = make_plan(s, t, &orders)?;
        for (j, job) in s.jobs.iter().enumerate() {
            if self.cnf.value(self.unassigned[j]) != p.unassigned.iter().any(|u| u.job_id == job.id)
            {
                return Err("SAT model contains a disconnected route".into());
            }
        }
        Ok(p)
    }
    fn warm_start(&mut self, s: &Scenario, p: &Plan) -> Option<bool> {
        let mut assumptions = vec![];
        let mut selected_arcs = std::collections::HashSet::new();
        let jobs: HashMap<_, _> = s
            .jobs
            .iter()
            .enumerate()
            .map(|(j, job)| (job.id.as_str(), j))
            .collect();
        for (e, r) in p.routes.iter().enumerate() {
            for (j, &x) in self.assignment[e].iter().enumerate() {
                if x != FALSE {
                    assumptions.push(if r.stops.iter().any(|stop| stop.job_id == s.jobs[j].id) {
                        x
                    } else {
                        -x
                    });
                }
            }
            let mut from = None;
            for stop in &r.stops {
                let to = Some(jobs[stop.job_id.as_str()]);
                selected_arcs.insert((e, from, to));
                from = to;
            }
            if from.is_some() {
                selected_arcs.insert((e, from, None));
            }
        }
        for (j, job) in s.jobs.iter().enumerate() {
            if let Some(stop) = p
                .routes
                .iter()
                .flat_map(|r| &r.stops)
                .find(|stop| stop.job_id == job.id)
            {
                for (b, &lit) in self.times[j].bits.iter().enumerate() {
                    assumptions.push(if stop.start > self.times[j].lo + b as u32 {
                        lit
                    } else {
                        -lit
                    });
                }
            } else {
                assumptions.push(self.unassigned[j]);
            }
        }
        for a in &self.arcs {
            if Instant::now() >= self.cnf.deadline {
                return None;
            }
            let selected = selected_arcs.contains(&(a.e, a.from, a.to));
            assumptions.push(if selected { a.lit } else { -a.lit });
        }
        self.cnf.solver.solve_with(assumptions)
    }
}

#[derive(Serialize)]
pub struct Stage {
    pub criterion: String,
    pub lower: u32,
    pub upper: u32,
    pub proven: bool,
}
#[derive(Serialize, Default)]
pub struct Stats {
    /// Bounds/proofs concern this search space, not necessarily all possible routes.
    pub scope: &'static str,
    pub search_complete: bool,
    pub candidate_routes: usize,
    pub generation_ms: u128,
    pub elapsed_ms: u128,
    pub encoding_ms: u128,
    pub variables: i32,
    pub clauses: usize,
    pub sat_calls: usize,
    pub optimal: bool,
    pub stages: Vec<Stage>,
    pub status: String,
}

// Cheap incumbent improvement only. The SAT search still admits every assignment.
fn compact(s: &Scenario, t: &Travel, best: &mut Plan, deadline: Instant) -> Result<()> {
    let ids: HashMap<_, _> = s
        .jobs
        .iter()
        .enumerate()
        .map(|(j, job)| (job.id.as_str(), j))
        .collect();
    loop {
        let orders: Vec<Vec<usize>> = best
            .routes
            .iter()
            .map(|r| r.stops.iter().map(|s| ids[s.job_id.as_str()]).collect())
            .collect();
        let mut sources: Vec<_> = (0..orders.len())
            .filter(|&e| !orders[e].is_empty() && !s.engineers[e].already_used)
            .collect();
        sources.sort_by_key(|&e| orders[e].len());
        let mut improved = false;
        for source in sources {
            if Instant::now() >= deadline {
                return Ok(());
            }
            let mut trial = orders.clone();
            let mut jobs = std::mem::take(&mut trial[source]);
            jobs.sort_by_key(|&j| (s.jobs[j].window_end, s.jobs[j].window_start));
            let mut complete = true;
            for j in jobs {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                let mut insertion = None;
                for (e, order) in trial.iter_mut().enumerate() {
                    if e == source || (order.is_empty() && !s.engineers[e].already_used) {
                        continue;
                    }
                    let old = route_cost(s, t, e, order).unwrap();
                    for pos in 0..=order.len() {
                        order.insert(pos, j);
                        if let Some(distance) = route_cost(s, t, e, order) {
                            let option = (distance as i64 - old as i64, e, pos);
                            if insertion.is_none_or(|old| option < old) {
                                insertion = Some(option);
                            }
                        }
                        order.remove(pos);
                    }
                }
                if let Some((_, e, pos)) = insertion {
                    trial[e].insert(pos, j);
                } else {
                    complete = false;
                    break;
                }
            }
            if complete {
                *best = make_plan(s, t, &trial)?;
                improved = true;
                break;
            }
        }
        if !improved {
            return Ok(());
        }
    }
}

// Distance moves free travel time; whole-route elimination can then reduce staff.
// Neither operation changes the feasible space of the unrestricted SAT model.
fn polish(s: &Scenario, t: &Travel, best: &mut Plan, deadline: Instant) -> Result<()> {
    loop {
        let staff = best.metrics.engineers;
        improve(s, t, best, deadline)?;
        compact(s, t, best, deadline)?;
        if best.metrics.engineers == staff || Instant::now() >= deadline {
            return Ok(());
        }
    }
}

fn mandatory_work(earliest: u32, latest: u32, duration: u32, a: u32, b: u32) -> u32 {
    let overlap = |start: u32| (start + duration).min(b).saturating_sub(start.max(a));
    overlap(earliest).min(overlap(latest))
}

// Energetic relaxation: every served job contributes its minimum service overlap
// with [a,b]. Even the best choice of omitted jobs and available shifts needs K staff.
fn staff_lower_bound(s: &Scenario, omitted: u32, deadline: Instant) -> u32 {
    let paid = s.engineers.iter().filter(|e| e.already_used).count() as u32;
    let mut lower = paid;
    let mut boundaries = vec![0, 1440];
    for j in &s.jobs {
        boundaries.push(j.window_start);
        boundaries.push(j.window_end + j.duration);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    for (index, &a) in boundaries.iter().enumerate() {
        if Instant::now() >= deadline {
            break;
        }
        for &b in &boundaries[index + 1..] {
            let mut work: Vec<_> = s
                .jobs
                .iter()
                .map(|j| mandatory_work(j.window_start, j.window_end, j.duration, a, b))
                .collect();
            work.sort_unstable();
            let work: u32 = work.iter().take(s.jobs.len() - omitted as usize).sum();
            let capacity = |e: &Engineer| e.shift_end.min(b).saturating_sub(e.shift_start.max(a));
            let mut covered: u32 = s
                .engineers
                .iter()
                .filter(|e| e.already_used)
                .map(capacity)
                .sum();
            let mut extra: Vec<_> = s
                .engineers
                .iter()
                .filter(|e| !e.already_used)
                .map(capacity)
                .collect();
            extra.sort_unstable_by(|a, b| b.cmp(a));
            let mut count = paid;
            for cap in extra {
                if covered >= work {
                    break;
                }
                covered += cap;
                count += 1;
            }
            lower = lower.max(count);
        }
    }
    lower
}

pub fn optimize(s: &Scenario, seconds: f64) -> Result<(Plan, Plan, Stats)> {
    validate_input(s)?;
    if !seconds.is_finite() || !(0.01..=300.0).contains(&seconds) {
        return Err("seconds must be between 0.01 and 300".into());
    }
    let started = Instant::now();
    let finish = started + Duration::from_secs_f64(seconds);
    // Do not spend the entire budget proving staff while returning unpolished routes.
    let reserve = if s.jobs.len() > 16 {
        (seconds * 0.1).min(0.1)
    } else {
        0.0
    };
    let deadline = finish - Duration::from_secs_f64(reserve);
    let t = Travel::new(s)?;
    let baseline = greedy(s, &t, false, None)?;
    let mut best = greedy(s, &t, true, Some(deadline))?;
    if baseline.metrics.key() < best.metrics.key() {
        best = baseline.clone();
    }
    let mut stats = Stats {
        scope: "global",
        ..Stats::default()
    };
    // Better upper bounds seed the unrestricted solver; no route is removed.
    if s.jobs.len() > 16 {
        let seed_deadline =
            deadline.min(started + Duration::from_secs_f64((seconds * 0.1).min(0.025)));
        for seed in 1..=24 {
            if Instant::now() >= seed_deadline {
                break;
            }
            let candidate = greedy_variant(s, &t, seed, seed_deadline)?;
            if candidate.metrics.key() < best.metrics.key() {
                best = candidate;
            }
        }
        compact(s, &t, &mut best, seed_deadline)?;
    }
    stats.generation_ms = started.elapsed().as_millis();
    let encoding_started = Instant::now();
    let Some(mut enc) = Encoding::build(s, &t, deadline) else {
        stats.encoding_ms = encoding_started.elapsed().as_millis();
        polish(s, &t, &mut best, finish)?;
        stats.elapsed_ms = started.elapsed().as_millis();
        stats.status =
            "Time budget reached during encoding; validated heuristic incumbent returned".into();
        return Ok((best, baseline, stats));
    };
    stats.encoding_ms = encoding_started.elapsed().as_millis();
    stats.sat_calls += 1;
    if let Some(false) = enc.warm_start(s, &best) {
        return Err("SAT encoding rejected a validated incumbent".into());
    }
    let already_used = s.engineers.iter().filter(|e| e.already_used).count() as u32;
    for (index, name) in ["urgent_unassigned", "unassigned", "engineers", "distance_m"]
        .iter()
        .enumerate()
    {
        let offset = if index == 2 { already_used } else { 0 };
        let mut lower = offset;
        let mut upper = best.metrics.key()[index];
        if index == 2 {
            lower = staff_lower_bound(s, best.metrics.unassigned, deadline);
        }
        if Instant::now() >= deadline {
            stats.stages.push(Stage {
                criterion: name.to_string(),
                lower,
                upper,
                proven: lower == upper,
            });
            break;
        }
        if index == 3 {
            if upper == 0 {
                stats.stages.push(Stage {
                    criterion: name.to_string(),
                    lower: 0,
                    upper: 0,
                    proven: true,
                });
                continue;
            }
            let build_started = Instant::now();
            let complete = enc.add_routes(s, &t);
            stats.encoding_ms += build_started.elapsed().as_millis();
            if !complete {
                stats.stages.push(Stage {
                    criterion: name.to_string(),
                    lower,
                    upper,
                    proven: false,
                });
                break;
            }
            stats.sat_calls += 1;
            if let Some(false) = enc.warm_start(s, &best) {
                return Err("Distance encoding rejected a validated incumbent".into());
            }
        }
        let terms: Vec<_> = match index {
            0 => enc
                .unassigned
                .iter()
                .enumerate()
                .filter(|(j, _)| s.jobs[*j].urgent)
                .map(|(_, &x)| (x, 1))
                .collect(),
            1 => enc.unassigned.iter().map(|&x| (x, 1)).collect(),
            2 => enc
                .used
                .iter()
                .enumerate()
                .filter(|(e, _)| !s.engineers[*e].already_used)
                .map(|(_, &x)| (x, 1))
                .collect(),
            _ => enc.distance_terms(),
        };
        // Zero count: unit clauses beat building an entire counting circuit.
        if upper == offset {
            for &(lit, _) in &terms {
                enc.cnf.clause(&[-lit]);
            }
            stats.stages.push(Stage {
                criterion: name.to_string(),
                lower,
                upper,
                proven: true,
            });
            continue;
        }
        let thresholds = if index < 3 {
            enc.cnf.count(
                &terms.iter().map(|&(lit, _)| lit).collect::<Vec<_>>(),
                (upper - offset + 1) as usize,
            )
        } else {
            vec![]
        };
        let Some(bits) = (if index < 3 {
            Some(vec![])
        } else {
            enc.cnf.sum(&terms)
        }) else {
            stats.stages.push(Stage {
                criterion: name.to_string(),
                lower,
                upper,
                proven: false,
            });
            break;
        };
        while lower < upper && Instant::now() < deadline {
            // Small improvements are easier to find for a weighted distance objective.
            let bound = if index >= 2 {
                upper - 1
            } else {
                lower + (upper - lower) / 2
            };
            let assumption = if index < 3 {
                -thresholds[(bound - offset) as usize]
            } else {
                enc.cnf.bound(&bits, (bound - offset) as u64)
            };
            stats.sat_calls += 1;
            match enc.cnf.solver.solve_with([assumption]) {
                Some(true) => {
                    if enc.refine_streets(s, &t) {
                        continue;
                    }
                    let mut candidate = enc.decode(s, &t)?;
                    if s.jobs.len() > 16 {
                        polish(
                            s,
                            &t,
                            &mut candidate,
                            deadline.min(
                                Instant::now()
                                    + Duration::from_secs_f64((seconds * 0.025).min(0.01)),
                            ),
                        )?;
                    }
                    let actual = candidate.metrics.key()[index];
                    if actual > bound {
                        return Err("SAT objective disagrees with decoded plan".into());
                    }
                    if candidate.metrics.key() < best.metrics.key() {
                        best = candidate;
                    }
                    upper = actual;
                }
                Some(false) => lower = bound + 1,
                None => break,
            }
        }
        stats.stages.push(Stage {
            criterion: name.to_string(),
            lower,
            upper,
            proven: lower == upper,
        });
        if lower != upper {
            break;
        }
        let fixed = if index < 3 {
            thresholds
                .get((upper - offset) as usize)
                .map_or(TRUE, |&lit| -lit)
        } else {
            enc.cnf.bound(&bits, (upper - offset) as u64)
        };
        enc.cnf.clause(&[fixed]);
    }
    if stats.stages.len() != 4 || stats.stages.iter().any(|stage| !stage.proven) {
        polish(s, &t, &mut best, finish)?;
        for (index, stage) in stats.stages.iter_mut().enumerate() {
            stage.upper = best.metrics.key()[index];
            if stage.upper < stage.lower {
                return Err("Improved incumbent contradicts a proven lower bound".into());
            }
            stage.proven = stage.lower == stage.upper;
        }
    }
    stats.variables = enc.cnf.vars;
    stats.clauses = enc.cnf.clauses;
    stats.optimal = stats.stages.len() == 4 && stats.stages.iter().all(|s| s.proven);
    stats.search_complete = stats.optimal;
    stats.elapsed_ms = started.elapsed().as_millis();
    stats.status = if stats.optimal {
        "Lexicographic optimum proven for the integer travel model"
    } else {
        "Time budget reached; best validated plan returned, optimality not proven"
    }
    .into();
    validate_plan(s, &best)?;
    Ok((best, baseline, stats))
}

#[cfg(test)]
mod circuit_tests {
    use super::*;
    fn time(c: &mut Cnf, lo: u32, hi: u32) -> Time {
        let bits: Vec<_> = (lo..hi).map(|_| c.var()).collect();
        for pair in bits.windows(2) {
            c.clause(&[-pair[1], pair[0]]);
        }
        Time { lo, hi, bits }
    }
    #[test]
    fn street_refinement_keeps_alternate_orders_and_inserted_stops() {
        use crate::routing::{test_snapshot, Leg, Profile, Snapshot};
        let mut s = crate::import::demo();
        s.jobs.truncate(3);
        s.engineers.truncate(1);
        s.engineers[0].skills = vec![Skill::Local, Skill::Connection, Skill::Emergency];
        s.engineers[0].transport = Transport::Car;
        s.engineers[0].shift_start = 540;
        s.engineers[0].shift_end = 600;
        for j in &mut s.jobs {
            j.duration = 1;
            j.window_start = 540;
            j.window_end = 550;
            j.transport = None;
        }
        let mut points: Vec<_> = s.jobs.iter().map(|j| j.point).collect();
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
        legs[3][1] = leg(1);
        legs[3][2] = leg(20);
        legs[0][1] = leg(20);
        legs[1][0] = leg(1);
        legs[0][2] = leg(1);
        legs[2][1] = leg(1);
        s.routing = Some(test_snapshot(Snapshot {
            points,
            profiles: vec![Profile {
                transport: Transport::Car,
                legs,
                lower: vec![],
            }],
        }));
        let t = Travel::new(&s).unwrap();
        let mut enc = Encoding::build(&s, &t, Instant::now() + Duration::from_secs(3)).unwrap();
        assert!(!enc.routed);
        let pins = |enc: &Encoding, starts: [Option<u32>; 3]| -> Vec<Lit> {
            starts
                .iter()
                .enumerate()
                .flat_map(|(j, start)| match start {
                    Some(start) => vec![
                        enc.assignment[0][j],
                        enc.times[j].lower(*start),
                        enc.times[j].upper(*start),
                    ],
                    None => vec![-enc.assignment[0][j]],
                })
                .collect()
        };
        let bad = pins(&enc, [Some(541), Some(544), None]);
        assert_eq!(enc.cnf.solver.solve_with(bad.clone()), Some(true));
        assert!(enc.decode(&s, &t).is_err()); // lower-time closure is not a real route
        assert!(enc.refine_streets(&s, &t));
        assert_eq!(enc.cnf.solver.solve_with(bad), Some(false));
        for starts in [
            [Some(543), Some(541), None],
            [Some(541), Some(545), Some(543)],
        ] {
            let assumptions = pins(&enc, starts);
            assert_eq!(enc.cnf.solver.solve_with(assumptions), Some(true));
            assert!(!enc.refine_streets(&s, &t));
            validate_plan(&s, &enc.decode(&s, &t).unwrap()).unwrap();
        }
        let p = enc.decode(&s, &t).unwrap();
        assert!(enc.add_routes(&s, &t));
        assert_eq!(enc.warm_start(&s, &p), Some(true));
    }

    #[test]
    fn mandatory_interval_work_is_a_safe_exact_minimum() {
        for earliest in 0..6 {
            for latest in earliest..6 {
                for duration in 1..5 {
                    for a in 0..10 {
                        for b in a + 1..11 {
                            let actual = (earliest..=latest)
                                .map(|start: u32| {
                                    (start + duration).min(b).saturating_sub(start.max(a))
                                })
                                .min()
                                .unwrap();
                            assert_eq!(mandatory_work(earliest, latest, duration, a, b), actual);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn order_times_and_truncated_counts_are_exact() {
        for a in 0..4 {
            for b in 0..4 {
                for delta in 0..5 {
                    let mut c = Cnf::new(Instant::now() + Duration::from_secs(10));
                    let left = time(&mut c, a, a + 3);
                    let right = time(&mut c, b, b + 3);
                    let before = precedence(&mut c, &left, &right, delta);
                    for x in a..=a + 3 {
                        for y in b..=b + 3 {
                            let pins: Vec<_> = [(&left, x), (&right, y)]
                                .into_iter()
                                .flat_map(|(t, v)| {
                                    t.bits.iter().enumerate().map(move |(i, &lit)| {
                                        if v > t.lo + i as u32 {
                                            lit
                                        } else {
                                            -lit
                                        }
                                    })
                                })
                                .collect();
                            assert_eq!(
                                c.solver.solve_with(pins.into_iter().chain([before])),
                                Some(x + delta <= y)
                            );
                            if x + delta <= y {
                                assert_eq!(left.value(&c), x);
                                assert_eq!(right.value(&c), y);
                            }
                        }
                    }
                }
            }
        }
        let mut c = Cnf::new(Instant::now() + Duration::from_secs(10));
        let xs: Vec<_> = (0..6).map(|_| c.var()).collect();
        let count = c.count(&xs, 4);
        for mask in 0u32..64 {
            let pins: Vec<_> = xs
                .iter()
                .enumerate()
                .map(|(i, &x)| if mask & (1 << i) != 0 { x } else { -x })
                .collect();
            for (k, &threshold) in count.iter().enumerate() {
                assert_eq!(
                    c.solver
                        .solve_with(pins.iter().copied().chain([-threshold])),
                    Some(mask.count_ones() <= k as u32)
                );
            }
        }
    }

    #[test]
    fn deferred_routes_preserve_feasible_schedules_and_energy_bounds() {
        let mut s = crate::import::demo();
        s.jobs.truncate(5);
        s.engineers.truncate(3);
        let t = Travel::new(&s).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut enc = Encoding::build(&s, &t, deadline).unwrap();
        let p = greedy(&s, &t, true, None).unwrap();
        assert!(!enc.routed && enc.arcs.is_empty());
        assert_eq!(enc.warm_start(&s, &p), Some(true));
        assert_eq!(enc.decode(&s, &t).unwrap().metrics.key(), p.metrics.key());
        assert!(staff_lower_bound(&s, p.metrics.unassigned, deadline) <= p.metrics.engineers);
        assert!(enc.add_routes(&s, &t));
        let terms = enc.distance_terms();
        let total = enc.cnf.sum(&terms).unwrap();
        assert_eq!(enc.warm_start(&s, &p), Some(true));
        let actual: u64 = total
            .iter()
            .enumerate()
            .map(|(i, &lit)| if enc.cnf.value(lit) { 1 << i } else { 0 })
            .sum();
        assert_eq!(actual, p.metrics.distance_m as u64);
        assert_eq!(enc.decode(&s, &t).unwrap().metrics.key(), p.metrics.key());
    }
    #[test]
    fn full_dataset_base_models_stay_small() {
        for file in crate::datasets().unwrap() {
            let s = crate::import::load(&file).unwrap();
            let t = Travel::new(&s).unwrap();
            let mut enc = Encoding::build(&s, &t, Instant::now() + Duration::from_secs(5)).unwrap();
            assert!(
                enc.cnf.vars < 30_000,
                "full time/assignment model grew unexpectedly"
            );
            assert!(enc.arcs.is_empty());
            let incumbent = greedy(&s, &t, true, None).unwrap();
            assert_eq!(enc.warm_start(&s, &incumbent), Some(true));
            validate_plan(&s, &enc.decode(&s, &t).unwrap()).unwrap();
        }
    }
    #[test]
    fn arithmetic_and_assumption_bounds_are_exact() {
        let mut c = Cnf::new(Instant::now() + Duration::from_secs(10));
        let a: Vec<_> = (0..4).map(|_| c.var()).collect();
        let b: Vec<_> = (0..4).map(|_| c.var()).collect();
        let sum = c.add(&a, &b);
        let leq = c.leq(&a, &b);
        let fits = c.bound(&sum, 15);
        for x in 0..16 {
            for y in 0..16 {
                let assumptions: Vec<_> = a
                    .iter()
                    .enumerate()
                    .map(|(i, &l)| if (x >> i) & 1 == 1 { l } else { -l })
                    .chain(
                        b.iter()
                            .enumerate()
                            .map(|(i, &l)| if (y >> i) & 1 == 1 { l } else { -l }),
                    )
                    .collect();
                assert_eq!(c.solver.solve_with(assumptions.clone()), Some(true));
                let actual: u32 = sum
                    .iter()
                    .enumerate()
                    .map(|(i, &l)| if c.value(l) { 1 << i } else { 0 })
                    .sum();
                assert_eq!(actual, x + y);
                assert_eq!(c.value(leq), x <= y);
                assert_eq!(c.value(fits), x + y <= 15);
                // An UNSAT trial must not poison later, looser queries.
                assert_eq!(
                    c.solver.solve_with(assumptions.into_iter().chain([fits])),
                    Some(x + y <= 15)
                );
            }
        }
        let weighted = c.sum(&[(TRUE, u32::MAX), (TRUE, u32::MAX)]).unwrap();
        let exact = c.bound(&weighted, 2 * u32::MAX as u64);
        let too_small = c.bound(&weighted, 2 * u32::MAX as u64 - 1);
        assert_eq!(c.solver.solve_with([exact]), Some(true));
        assert_eq!(c.solver.solve_with([too_small]), Some(false));
    }
}
