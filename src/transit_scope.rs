//! Conservative departure coverage derived from service windows, never journey pruning.
use crate::{
    model::{compatible, Point, Result, Scenario, Transport},
    routing::point_index,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub start: u32,
    pub end: u32,
}

impl Coverage {
    pub fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    // Include gaps for disjoint requests: union stores a single contiguous hull.
    // Already queried departures are never fetched again, including boundary minutes.
    pub fn missing(self, requested: Self) -> Vec<Self> {
        let mut missing = Vec::new();
        if requested.start < self.start {
            missing.push(Self {
                start: requested.start,
                end: self.start - 1,
            });
        }
        if requested.end > self.end {
            missing.push(Self {
                start: self.end + 1,
                end: requested.end,
            });
        }
        missing
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

pub(crate) fn required(s: &Scenario, points: &[Point]) -> Result<Vec<Vec<Option<Coverage>>>> {
    required_for(s, points, Transport::Public)
}

/// Every arc a valid schedule can use, before accounting for nonnegative travel.
/// Shared by roads and transit: no base-to-base or incompatible/time-reversed work.
pub(crate) fn required_for(
    s: &Scenario,
    points: &[Point],
    transport: Transport,
) -> Result<Vec<Vec<Option<Coverage>>>> {
    let mut required = vec![vec![None; points.len()]; points.len()];
    let job_points = s
        .jobs
        .iter()
        .map(|job| point_index(points, job.point))
        .collect::<Result<Vec<_>>>()?;
    let mut include = |from: usize, to: usize, start: u32, end: u32| {
        if from != to && start <= end {
            let interval = Coverage { start, end };
            let cell: &mut Option<Coverage> = &mut required[from][to];
            *cell = Some(cell.map_or(interval, |old| old.union(interval)));
        }
    };
    for engineer in s.engineers.iter().filter(|e| e.transport == transport) {
        let base = point_index(points, engineer.start)?;
        let feasible: Vec<_> = s
            .jobs
            .iter()
            .enumerate()
            .filter_map(|(index, job)| {
                if !compatible(engineer, job) {
                    return None;
                }
                let latest = job
                    .window_end
                    .min(engineer.shift_end.checked_sub(job.duration)?);
                let earliest = job.window_start.max(engineer.shift_start);
                (earliest <= latest).then_some((index, earliest + job.duration, latest))
            })
            .collect();
        for &(target, _, latest) in &feasible {
            include(base, job_points[target], engineer.shift_start, latest);
            for &(source, earliest_end, _) in &feasible {
                include(job_points[source], job_points[target], earliest_end, latest);
            }
        }
    }
    Ok(required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Engineer, Job, Skill};

    fn point(id: usize) -> Point {
        Point {
            lat: 55.0 + id as f64 / 100.0,
            lon: 37.0,
        }
    }

    fn job(id: usize, start: u32, end: u32, duration: u32) -> Job {
        Job {
            id: id.to_string(),
            address: String::new(),
            point: point(id),
            duration,
            work_type: None,
            geocode_match: None,
            window_start: start,
            window_end: end,
            skill: Skill::Local,
            transport: None,
            urgent: false,
        }
    }

    fn scenario(jobs: Vec<Job>) -> Scenario {
        Scenario {
            name: String::new(),
            jobs,
            engineers: vec![Engineer {
                id: "engineer".into(),
                name: String::new(),
                start: point(0),
                shift_start: 480,
                shift_end: 1080,
                skills: vec![Skill::Local],
                transport: Transport::Public,
                already_used: false,
            }],
            routing: None,
            transit_date: None,
            notes: vec![],
        }
    }

    #[test]
    fn reverse_time_transitions_are_absent_but_boundary_departures_remain() {
        let s = scenario(vec![job(1, 600, 600, 30), job(2, 630, 630, 30)]);
        let needed = required(&s, &[point(0), point(1), point(2)]).unwrap();
        assert_eq!(
            needed[1][2],
            Some(Coverage {
                start: 630,
                end: 630
            })
        );
        assert_eq!(needed[2][1], None);
        assert_eq!(
            needed[0][1],
            Some(Coverage {
                start: 480,
                end: 600
            })
        );
        assert_eq!(needed[1][1], None);
    }

    #[test]
    fn colocated_jobs_and_bases_aggregate_without_self_queries() {
        let mut s = scenario(vec![
            job(1, 600, 650, 20),
            job(1, 800, 850, 30),
            job(2, 700, 900, 30),
        ]);
        s.jobs[1].id = "later".into();
        s.engineers[0].start = point(1);
        let needed = required(&s, &[point(1), point(2)]).unwrap();
        assert_eq!(needed[0][0], None);
        assert_eq!(
            needed[0][1],
            Some(Coverage {
                start: 480,
                end: 900
            })
        );
        assert_eq!(
            needed[1][0],
            Some(Coverage {
                start: 730,
                end: 850
            })
        );
    }

    #[test]
    fn earlier_source_and_later_target_windows_expand_departure_bounds() {
        let mut s = scenario(vec![job(1, 600, 650, 30), job(2, 700, 750, 30)]);
        let points = [point(0), point(1), point(2)];
        let old = required(&s, &points).unwrap()[1][2].unwrap();
        assert_eq!(
            old,
            Coverage {
                start: 630,
                end: 750
            }
        );
        s.jobs[0].window_start = 550;
        s.jobs[1].window_end = 800;
        let expanded = required(&s, &points).unwrap()[1][2].unwrap();
        assert_eq!(
            old.missing(expanded),
            vec![
                Coverage {
                    start: 580,
                    end: 629
                },
                Coverage {
                    start: 751,
                    end: 800
                },
            ]
        );
        assert_eq!(
            old.union(expanded),
            Coverage {
                start: 580,
                end: 800
            }
        );
    }

    #[test]
    fn only_compatible_public_engineers_contribute_their_shift_bounds() {
        let mut s = scenario(vec![
            job(1, 0, 1440, 60),
            job(2, 0, 1440, 60),
            job(3, 0, 1440, 60),
        ]);
        s.jobs[1].skill = Skill::Emergency;
        s.jobs[2].transport = Some(Transport::Car);
        let mut late = s.engineers[0].clone();
        late.id = "late".into();
        late.shift_start = 900;
        late.shift_end = 1200;
        late.skills = vec![Skill::Emergency];
        s.engineers.push(late);
        let mut car = s.engineers[0].clone();
        car.id = "car".into();
        car.transport = Transport::Car;
        car.shift_start = 0;
        s.engineers.push(car);
        let needed = required(&s, &[point(0), point(1), point(2), point(3)]).unwrap();
        assert_eq!(
            needed[0][1],
            Some(Coverage {
                start: 480,
                end: 1020
            })
        );
        assert_eq!(
            needed[0][2],
            Some(Coverage {
                start: 900,
                end: 1140
            })
        );
        assert_eq!(needed[0][3], None);
        assert_eq!(needed[1][2], None);
        assert_eq!(needed[2][1], None);
    }

    #[test]
    fn impossible_service_windows_do_not_create_queries() {
        let mut s = scenario(vec![
            job(1, 0, 479, 1),
            job(2, 1080, 1440, 1),
            job(3, 0, 1440, 1440),
        ]);
        let points = [point(0), point(1), point(2), point(3)];
        assert!(required(&s, &points)
            .unwrap()
            .iter()
            .flatten()
            .all(Option::is_none));
        s.engineers[0].shift_start = 0;
        s.engineers[0].shift_end = 1440;
        let needed = required(&s, &points).unwrap();
        assert_eq!(needed[0][3], Some(Coverage { start: 0, end: 0 }));
        assert_eq!(needed[3][1], None);
    }

    #[test]
    fn expansion_queries_only_new_edges_and_fills_disjoint_gaps() {
        let old = Coverage {
            start: 480,
            end: 900,
        };
        assert!(old.contains(Coverage {
            start: 480,
            end: 900
        }));
        assert!(old
            .missing(Coverage {
                start: 500,
                end: 800
            })
            .is_empty());
        let expanded = Coverage {
            start: 0,
            end: 1440,
        };
        assert_eq!(
            old.missing(expanded),
            vec![
                Coverage { start: 0, end: 479 },
                Coverage {
                    start: 901,
                    end: 1440
                }
            ]
        );
        assert_eq!(old.union(expanded), expanded);
        assert_eq!(
            old.missing(Coverage {
                start: 100,
                end: 200
            }),
            vec![Coverage {
                start: 100,
                end: 479
            }]
        );
        assert_eq!(
            old.missing(Coverage {
                start: 1000,
                end: 1100
            }),
            vec![Coverage {
                start: 901,
                end: 1100
            }]
        );
        assert!(expanded.missing(old).is_empty());
    }
}
