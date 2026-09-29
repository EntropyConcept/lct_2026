import type { Job, JobId } from "./job";
import type { Engineer, EngineerId } from "./worker";

export interface Point {
  lat: number;
  lon: number;
}

export interface Scenario {
  name: string;
  jobs: Job[];
  engineers: Engineer[];

  routing?: unknown;
  transit_date?: string;
  notes?: string[];
}

export interface RouteStop {
  jobId: JobId;

  /** Planned arrival/start/end times */
  arrival: Date;
  start: Date;
  end: Date;

  /** Departure time is used by the event/cancellation logic. */
  departure: Date;

  /** Route geometry returned by the road-routing backend. */
  shape?: string;

  // TODO: on backend that is ment to be turned into object with fields
  //       each one represents reasons for choosing engineer for job
  /** Human-readable reason for this assignment. */
  explanation: unknown;
}

export interface Route {
  engineerId: EngineerId;
  stops: RouteStop[];
  distance: number;
}

export interface UnassignedJob {
  jobId: JobId;
  reason: string;
}

export interface PlanMetrics {
  /** Number of jobs that could not be assigned. */
  unassigned: number;

  /** Number of urgent jobs that could not be assigned. */
  urgentUnassigned: number;

  /** Number of engineers used by this plan. */
  engineers: number;

  /** Total route distance in metres. */
  distance: number;
}

export interface Plan {
  routes: Route[];
  unassigned: UnassignedJob[];
  metrics: PlanMetrics;
}

export interface SolverStage {
  criterion: string;
  lower: number;
  upper: number;
  proven: boolean;
}

export interface PlanResult {
  plan: Plan;
  baseline: Plan;
  stats: SolverStats;
  changes: string[];

  lastEventTime: Date | null;
  scenario?: Scenario;
}

export interface SolverStats {
  elapsed_ms: number;
  optimal: boolean;
  scope: "candidate_routes" | string;
  search_complete: boolean;
  candidate_routes?: number;

  status: string;
  generation_ms: number;
  variables: number;
  clauses: number;
  sat_calls: number;
  encoding_ms: number;

  stages: SolverStage[];
}

export type AssignedJob = Job & ({
  assigned: boolean,
  // assigned: true
  stop?: RouteStop,
  engineer?: Engineer,
  // assigned: false
  reason?: string;
});

export type AssignedEngineer = Engineer & ({
  assigned: boolean,
  // assigned: true
  stops?: RouteStop[],
  // assigned: false
  // no stops
});