import type { Point } from "../types";
import type { Transport, Skill } from "../worker";

export type JobId = string;
export type JobType = "local" | "connection" | "emergency" | "equipment";

export interface Job {
  id: JobId;
  address: string;
  point: Point;
  name?: string;

  /** Planned work duration, in minutes. */
  duration: number;

  windowStart: Date;
  windowEnd: Date;

  skill: Skill;
  transport: Transport | null;
  jobType: JobType;
  urgent: boolean;

  /** Added by geocoding in the existing application, when available. */
  geocodeMatch?: string | null;
}
