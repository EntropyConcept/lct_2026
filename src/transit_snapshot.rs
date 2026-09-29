//! Lossless frozen timetables with one validated allocation per distinct geometry.
use crate::{
    model::{Result, Transport},
    routing::{self, Leg, Profile},
    transit_scope::Coverage,
};
use serde::{
    de::Error as _, ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const MAX_GEOMETRY: usize = 32_000_000;
const MAX_SHAPE: usize = 200_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitLeg {
    pub metres: u32,
    pub minutes: u32,
    pub shape: Arc<str>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journey {
    pub departure: u32,
    pub arrival: u32,
    pub leg: TransitLeg,
    pub description: String,
}

#[derive(Clone, Debug, Default)]
struct ShapePool {
    shapes: HashSet<Arc<str>>,
    // Every address belongs to an Arc retained in shapes, so it cannot be reused.
    addresses: HashSet<usize>,
    bytes: usize,
}

impl ShapePool {
    fn intern(&mut self, shape: &mut Arc<str>, validated: Option<&Self>) -> Result<()> {
        if self.addresses.contains(&(shape.as_ptr() as usize)) {
            return Ok(());
        }
        if let Some(shared) = self.shapes.get(shape.as_ref()) {
            *shape = Arc::clone(shared);
            return Ok(());
        }
        if shape.len() > MAX_SHAPE {
            return Err("Invalid transit journey in snapshot".into());
        }
        if self.bytes + shape.len() > MAX_GEOMETRY {
            return Err("Transit geometry exceeds 32 MB; reduce the planning area".into());
        }
        if let Some(shared) = validated.and_then(|pool| pool.shapes.get(shape.as_ref())) {
            *shape = Arc::clone(shared);
        } else if routing::decode_shape(shape)?.len() < 2 {
            return Err("Invalid transit journey in snapshot".into());
        }
        self.bytes += shape.len();
        self.addresses.insert(shape.as_ptr() as usize);
        self.shapes.insert(Arc::clone(shape));
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Timetable {
    pub date: String,
    pub provider: String,
    // None = not queried; Some([]) = no transit journey within the covered departures.
    pub journeys: Vec<Vec<Option<Vec<Journey>>>>,
    pub coverage: CoverageMatrix,
    pub suffix: Vec<Vec<Vec<usize>>>,
    pool: ShapePool,
}

type CoverageMatrix = Vec<Vec<Option<Coverage>>>;

impl Timetable {
    pub fn new(date: String, provider: String) -> Self {
        Self {
            date,
            provider,
            journeys: vec![],
            coverage: vec![],
            suffix: vec![],
            pool: ShapePool::default(),
        }
    }

    pub fn geometry_bytes(&self) -> usize {
        self.pool.bytes
    }

    pub(crate) fn intern(&mut self, mut journeys: Vec<Journey>) -> Result<Vec<Journey>> {
        if journeys.len() > 10_000 {
            return Err("Too many transit journeys for one point pair".into());
        }
        for journey in &mut journeys {
            self.pool.intern(&mut journey.leg.shape, None)?;
        }
        Ok(journeys)
    }

    fn full_day_coverage(&self) -> CoverageMatrix {
        self.journeys
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| {
                        cell.as_ref().map(|_| Coverage {
                            start: 0,
                            end: 1440,
                        })
                    })
                    .collect()
            })
            .collect()
    }

    pub fn validate(&mut self, n: usize) -> Result<()> {
        crate::transit::midnight(&self.date)?;
        if n > 130
            || self.provider.len() > 2048
            || self.journeys.len() != n
            || self.journeys.iter().any(|row| row.len() != n)
        {
            return Err("Invalid transit snapshot dimensions/provider".into());
        }
        if self.coverage.is_empty() {
            self.coverage = self.full_day_coverage();
        }
        if self.coverage.len() != n || self.coverage.iter().any(|row| row.len() != n) {
            return Err("Invalid transit coverage dimensions".into());
        }
        let mut pool = ShapePool::default();
        self.suffix = vec![vec![vec![]; n]; n];
        for (i, row) in self.journeys.iter_mut().enumerate() {
            for (k, cell) in row.iter_mut().enumerate() {
                let coverage = self.coverage[i][k];
                if cell.is_some() != coverage.is_some()
                    || coverage.is_some_and(|range| range.start > range.end || range.end > 1440)
                {
                    return Err("Invalid transit coverage in snapshot".into());
                }
                let (Some(journeys), Some(coverage)) = (cell, coverage) else {
                    continue;
                };
                if journeys.len() > 10_000 {
                    return Err("Too many transit journeys for one point pair".into());
                }
                let mut suffix = vec![0; journeys.len()];
                let mut best = journeys.len().saturating_sub(1);
                let mut best_key = (u32::MAX, u32::MAX, u32::MAX);
                for index in (0..journeys.len()).rev() {
                    let previous = index.checked_sub(1).map(|p| journeys[p].departure);
                    let journey = &mut journeys[index];
                    if journey.departure > journey.arrival
                        || journey.departure < coverage.start
                        || journey.departure > coverage.end
                        || journey.arrival > 1440
                        || journey.leg.minutes != journey.arrival - journey.departure
                        || journey.leg.metres > 20_000_000
                        || journey.description.len() > 8192
                        || previous.is_some_and(|departure| departure > journey.departure)
                    {
                        return Err("Invalid transit journey in snapshot".into());
                    }
                    pool.intern(&mut journey.leg.shape, Some(&self.pool))?;
                    let key = (journey.arrival, journey.leg.metres, journey.departure);
                    if key <= best_key {
                        best = index;
                        best_key = key;
                    }
                    suffix[index] = best;
                }
                self.suffix[i][k] = suffix;
            }
        }
        self.pool = pool;
        Ok(())
    }

    pub fn journey(&self, from: usize, to: usize, ready: u32) -> Option<&Journey> {
        let journeys = self.journeys[from][to].as_ref()?;
        let first = journeys.partition_point(|journey| journey.departure < ready);
        self.suffix[from][to]
            .get(first)
            .map(|&best| &journeys[best])
    }

    pub(crate) fn profile(&self) -> Profile {
        let legs = self
            .journeys
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .map(|(j, cell)| {
                        if i == j {
                            return Some(Leg {
                                minutes: 0,
                                metres: 0,
                                shape: String::new(),
                            });
                        }
                        let journeys = cell.as_ref()?;
                        // Independent optimistic minima, never used to schedule an actual transit leg.
                        Some(Leg {
                            minutes: journeys.iter().map(|j| j.leg.minutes).min()?,
                            metres: journeys.iter().map(|j| j.leg.metres).min()?,
                            shape: String::new(),
                        })
                    })
                    .collect()
            })
            .collect();
        Profile {
            transport: Transport::Public,
            legs,
            lower: vec![],
            coverage: None,
        }
    }
}

#[derive(Serialize)]
struct CompactLeg {
    metres: u32,
    minutes: u32,
    shape: usize,
}

#[derive(Serialize)]
struct CompactJourney<'a> {
    departure: u32,
    arrival: u32,
    leg: CompactLeg,
    description: &'a str,
}

// Stream the existing nested vectors without copying journeys or geometry strings.
struct CompactJourneys<'a>(&'a [Vec<Option<Vec<Journey>>>], &'a HashMap<usize, usize>);
struct CompactRow<'a>(&'a [Option<Vec<Journey>>], &'a HashMap<usize, usize>);
struct CompactCell<'a>(&'a [Journey], &'a HashMap<usize, usize>);

impl Serialize for CompactJourneys<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|row| CompactRow(row, self.1)))
    }
}

impl Serialize for CompactRow<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(
            self.0
                .iter()
                .map(|cell| cell.as_ref().map(|journeys| CompactCell(journeys, self.1))),
        )
    }
}

impl Serialize for CompactCell<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|journey| CompactJourney {
            departure: journey.departure,
            arrival: journey.arrival,
            leg: CompactLeg {
                metres: journey.leg.metres,
                minutes: journey.leg.minutes,
                shape: self.1[&(journey.leg.shape.as_ptr() as usize)],
            },
            description: &journey.description,
        }))
    }
}

impl Serialize for Timetable {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut shapes = vec![];
        let mut by_content = HashMap::new();
        let mut by_address = HashMap::new();
        for journey in self.journeys.iter().flatten().flatten().flatten() {
            let shape = journey.leg.shape.as_ref();
            let address = shape.as_ptr() as usize;
            by_address.entry(address).or_insert_with(|| {
                *by_content.entry(shape).or_insert_with(|| {
                    let index = shapes.len();
                    shapes.push(shape);
                    index
                })
            });
        }
        let mut wire = serializer.serialize_struct("Timetable", 5)?;
        wire.serialize_field("date", &self.date)?;
        wire.serialize_field("provider", &self.provider)?;
        wire.serialize_field("shapes", &shapes)?;
        wire.serialize_field("journeys", &CompactJourneys(&self.journeys, &by_address))?;
        if self.coverage.is_empty() {
            wire.serialize_field("coverage", &self.full_day_coverage())?;
        } else {
            wire.serialize_field("coverage", &self.coverage)?;
        }
        wire.end()
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredShape {
    Index(usize),
    Inline(Arc<str>),
}

#[derive(Deserialize)]
struct StoredLeg {
    metres: u32,
    minutes: u32,
    shape: StoredShape,
}

#[derive(Deserialize)]
struct StoredJourney {
    departure: u32,
    arrival: u32,
    leg: StoredLeg,
    description: String,
}

#[derive(Deserialize)]
struct StoredTimetable {
    date: String,
    provider: String,
    #[serde(default)]
    shapes: Option<Vec<Arc<str>>>,
    journeys: Vec<Vec<Option<Vec<StoredJourney>>>>,
    #[serde(default, deserialize_with = "stored_coverage")]
    coverage: Option<CoverageMatrix>,
}

fn stored_coverage<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<CoverageMatrix>, D::Error> {
    CoverageMatrix::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for Timetable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let stored = StoredTimetable::deserialize(deserializer)?;
        let mut timetable = Self::new(stored.date, stored.provider);
        let mut shapes = stored.shapes;
        if let Some(shapes) = &mut shapes {
            for shape in shapes {
                timetable
                    .pool
                    .intern(shape, None)
                    .map_err(D::Error::custom)?;
            }
        }
        for row in stored.journeys {
            let mut journeys_row = Vec::with_capacity(row.len());
            for cell in row {
                let Some(cell) = cell else {
                    journeys_row.push(None);
                    continue;
                };
                let mut journeys = Vec::with_capacity(cell.len());
                for stored in cell {
                    let mut shape = match (stored.leg.shape, &shapes) {
                        (StoredShape::Index(index), Some(shapes)) => {
                            Arc::clone(shapes.get(index).ok_or_else(|| {
                                D::Error::custom("Invalid transit geometry index")
                            })?)
                        }
                        (StoredShape::Inline(shape), None) => shape,
                        _ => return Err(D::Error::custom("Invalid transit geometry reference")),
                    };
                    timetable
                        .pool
                        .intern(&mut shape, None)
                        .map_err(D::Error::custom)?;
                    journeys.push(Journey {
                        departure: stored.departure,
                        arrival: stored.arrival,
                        leg: TransitLeg {
                            metres: stored.leg.metres,
                            minutes: stored.leg.minutes,
                            shape,
                        },
                        description: stored.description,
                    });
                }
                journeys_row.push(Some(journeys));
            }
            timetable.journeys.push(journeys_row);
        }
        if let Some(coverage) = stored.coverage {
            if coverage.len() != timetable.journeys.len() {
                return Err(D::Error::custom("Invalid transit coverage dimensions"));
            }
            timetable.coverage = coverage;
        }
        timetable
            .validate(timetable.journeys.len())
            .map_err(D::Error::custom)?;
        Ok(timetable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn journey(departure: u32, arrival: u32, metres: u32, shape: &str, label: &str) -> Journey {
        Journey {
            departure,
            arrival,
            leg: TransitLeg {
                metres,
                minutes: arrival - departure,
                shape: shape.into(),
            },
            description: label.into(),
        }
    }

    fn timetable(journeys: Vec<Journey>) -> Timetable {
        let mut timetable = Timetable::new("2026-09-27".into(), "http://localhost".into());
        timetable.journeys = vec![vec![None, Some(journeys)], vec![Some(vec![]), None]];
        timetable
    }

    fn observed(journey: Option<&Journey>) -> Option<(u32, u32, u32, u32, &str, &str)> {
        journey.map(|journey| {
            (
                journey.departure,
                journey.arrival,
                journey.leg.metres,
                journey.leg.minutes,
                journey.leg.shape.as_ref(),
                journey.description.as_str(),
            )
        })
    }

    #[test]
    fn repeated_geometry_exceeding_32_mb_roundtrips_once_without_losing_departures() {
        let shape = "??".repeat(10_000);
        let journeys = (0..2000)
            .map(|i| journey(i / 2, i / 2 + 1, i + 1, &shape, &format!("BUS {i}")))
            .collect::<Vec<_>>();
        assert!(
            journeys
                .iter()
                .map(|journey| journey.leg.shape.len())
                .sum::<usize>()
                > MAX_GEOMETRY
        );
        let mut original = timetable(vec![]);
        let journeys = original.intern(journeys).unwrap();
        original.journeys[0][1] = Some(journeys);
        original.validate(2).unwrap();
        assert_eq!(original.geometry_bytes(), shape.len());
        let bytes = serde_json::to_vec(&original).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wire["shapes"], json!([shape]));
        assert_eq!(wire["journeys"][0][1][1999]["leg"]["shape"], json!(0));
        let restored: Timetable = serde_json::from_slice(&bytes).unwrap();
        assert!(restored.journeys[0][1]
            .as_ref()
            .unwrap()
            .iter()
            .map(|journey| observed(Some(journey)))
            .eq(original.journeys[0][1]
                .as_ref()
                .unwrap()
                .iter()
                .map(|journey| observed(Some(journey)))));
        assert!(restored.journeys[0][0].is_none());
        assert!(restored.journeys[1][0].as_ref().unwrap().is_empty());
        assert_eq!(restored.geometry_bytes(), shape.len());
        for ready in 0..=1440 {
            assert_eq!(
                observed(restored.journey(0, 1, ready)),
                observed(original.journey(0, 1, ready))
            );
        }
        let restored = restored.journeys[0][1].as_ref().unwrap();
        assert!(restored
            .iter()
            .all(|journey| Arc::ptr_eq(&journey.leg.shape, &restored[0].leg.shape)));
    }

    #[test]
    fn legacy_inline_snapshot_keeps_history_and_all_departure_tie_breaks() {
        let journeys = vec![
            journey(0, 100, 10, "??AA", "slow"),
            journey(5, 20, 30, "??CC", "overtaking"),
            journey(5, 20, 20, "??AA", "distance first"),
            journey(5, 20, 20, "??CC", "same key, later input"),
            journey(6, 20, 20, "??EE", "same cost, later departure"),
            journey(100, 110, 5, "??AA", "late service · frozen"),
            journey(1440, 1440, 0, "??AA", "midnight"),
        ];
        let mut legacy = timetable(journeys);
        legacy.journeys[1][0] = Some(vec![journey(10, 15, 2, "??AA", "return")]);
        let old = json!({
            "date": legacy.date,
            "provider": legacy.provider,
            "journeys": legacy.journeys,
        });
        let loaded: Timetable = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(
            loaded.coverage,
            vec![
                vec![
                    None,
                    Some(Coverage {
                        start: 0,
                        end: 1440
                    })
                ],
                vec![
                    Some(Coverage {
                        start: 0,
                        end: 1440
                    }),
                    None
                ],
            ]
        );
        assert_eq!(
            serde_json::to_value(&loaded.journeys).unwrap(),
            old["journeys"]
        );
        assert!(Arc::ptr_eq(
            &loaded.journeys[0][1].as_ref().unwrap()[0].leg.shape,
            &loaded.journeys[1][0].as_ref().unwrap()[0].leg.shape,
        ));
        let roundtrip: Timetable =
            serde_json::from_slice(&serde_json::to_vec(&loaded).unwrap()).unwrap();
        for ready in 0..=1441 {
            let expected = loaded.journeys[0][1]
                .as_ref()
                .unwrap()
                .iter()
                .enumerate()
                .filter(|(_, journey)| journey.departure >= ready)
                .min_by_key(|(index, journey)| {
                    (
                        journey.arrival,
                        journey.leg.metres,
                        journey.departure,
                        *index,
                    )
                })
                .map(|(_, journey)| journey);
            assert_eq!(observed(loaded.journey(0, 1, ready)), observed(expected));
            assert_eq!(observed(roundtrip.journey(0, 1, ready)), observed(expected));
        }
    }

    #[test]
    fn partial_coverage_roundtrip_preserves_departures_and_later_arrivals() {
        let mut original = timetable(vec![
            journey(100, 500, 30, "??AA", "slow"),
            journey(150, 210, 20, "??CC", "overtaking"),
            journey(200, 1440, 10, "??AA", "last departure"),
        ]);
        original.coverage = vec![
            vec![
                None,
                Some(Coverage {
                    start: 100,
                    end: 200,
                }),
            ],
            vec![
                Some(Coverage {
                    start: 300,
                    end: 300,
                }),
                None,
            ],
        ];
        // Serialization must retain bounded coverage even before explicit validation.
        let restored: Timetable =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(restored.coverage, original.coverage);
        assert_eq!(
            serde_json::to_value(&restored.journeys).unwrap(),
            serde_json::to_value(&original.journeys).unwrap()
        );
        original.validate(2).unwrap();
        for ready in 0..=1441 {
            assert_eq!(
                observed(restored.journey(0, 1, ready)),
                observed(original.journey(0, 1, ready))
            );
        }
        assert_eq!(
            restored.journey(0, 1, 100).unwrap().description,
            "overtaking"
        );
        assert_eq!(restored.journey(0, 1, 200).unwrap().arrival, 1440);
        assert!(restored.journey(1, 0, 300).is_none());
    }

    #[test]
    fn legacy_compact_coverage_and_unvalidated_serialization_remain_full_day() {
        let original = timetable(vec![journey(1440, 1440, 0, "??AA", "midnight")]);
        let mut wire = serde_json::to_value(&original).unwrap();
        let restored: Timetable = serde_json::from_value(wire.clone()).unwrap();
        wire.as_object_mut().unwrap().remove("coverage");
        let legacy: Timetable = serde_json::from_value(wire).unwrap();
        let expected = vec![
            vec![
                None,
                Some(Coverage {
                    start: 0,
                    end: 1440,
                }),
            ],
            vec![
                Some(Coverage {
                    start: 0,
                    end: 1440,
                }),
                None,
            ],
        ];
        assert_eq!(restored.coverage, expected);
        assert_eq!(legacy.coverage, expected);
        assert_eq!(
            observed(restored.journey(0, 1, 1440)),
            observed(legacy.journey(0, 1, 1440))
        );
    }

    #[test]
    fn snapshots_reject_malformed_coverage_and_uncovered_departures() {
        let mut valid = timetable(vec![journey(100, 200, 1, "??AA", "BUS")]);
        valid.coverage = vec![
            vec![
                None,
                Some(Coverage {
                    start: 100,
                    end: 100,
                }),
            ],
            vec![
                Some(Coverage {
                    start: 300,
                    end: 400,
                }),
                None,
            ],
        ];
        let wire = serde_json::to_value(&valid).unwrap();
        for coverage in [
            json!(null),
            json!([]),
            json!([[null, {"start": 100, "end": 100}]]),
            json!([[null], [{"start": 300, "end": 400}, null]]),
            json!([[null, null], [{"start": 300, "end": 400}, null]]),
            json!([[{"start": 0, "end": 0}, {"start": 100, "end": 100}],
                [{"start": 300, "end": 400}, null]]),
        ] {
            let mut invalid = wire.clone();
            invalid["coverage"] = coverage;
            assert!(serde_json::from_value::<Timetable>(invalid).is_err());
        }
        for range in [
            json!({"start": 101, "end": 100}),
            json!({"start": 0, "end": 1441}),
            json!({"start": 101, "end": 200}),
            json!({"start": 0, "end": 99}),
        ] {
            let mut invalid = wire.clone();
            invalid["coverage"][0][1] = range;
            assert!(serde_json::from_value::<Timetable>(invalid).is_err());
        }
        valid.coverage[1][0] = None;
        assert!(valid.validate(2).is_err());
    }

    #[test]
    fn compact_snapshot_rejects_malformed_geometry_references() {
        let mut valid = timetable(vec![journey(0, 1, 1, "??AA", "BUS")]);
        valid.validate(2).unwrap();
        let wire = serde_json::to_value(&valid).unwrap();
        for reference in [json!(1), json!(-1), json!(0.5), json!(null), json!("??AA")] {
            let mut invalid = wire.clone();
            invalid["journeys"][0][1][0]["leg"]["shape"] = reference;
            assert!(serde_json::from_value::<Timetable>(invalid).is_err());
        }
        let mut missing_dictionary = wire.clone();
        missing_dictionary.as_object_mut().unwrap().remove("shapes");
        assert!(serde_json::from_value::<Timetable>(missing_dictionary).is_err());
        let mut invalid_geometry = wire;
        invalid_geometry["shapes"][0] = json!("_");
        assert!(serde_json::from_value::<Timetable>(invalid_geometry).is_err());
    }

    #[test]
    fn geometry_limits_still_bound_unique_content_and_each_shape() {
        let shape = "??".repeat(MAX_SHAPE / 2);
        let mut timetable = timetable(vec![]);
        for index in 0..MAX_GEOMETRY / MAX_SHAPE {
            let prefix = index as u32;
            let mut unique = shape.clone().into_bytes();
            unique[0] = 63 + (prefix % 32) as u8;
            unique[1] = 63 + (prefix / 32) as u8;
            let unique = String::from_utf8(unique).unwrap();
            timetable
                .intern(vec![journey(0, 1, 1, &unique, "BUS")])
                .unwrap();
        }
        assert_eq!(timetable.geometry_bytes(), MAX_GEOMETRY);
        assert!(timetable
            .intern(vec![journey(0, 1, 1, "??AA", "extra")])
            .is_err());
        let mut timetable = Timetable::new("2026-09-27".into(), "http://localhost".into());
        assert!(timetable
            .intern(vec![journey(0, 1, 1, &(shape + "??"), "oversize")])
            .is_err());
    }
}
