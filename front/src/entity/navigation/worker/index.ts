import type { Point } from "../types";

export type EngineerId = string;
export type Skill = "local" | "connection" | "emergency";
export type Transport = "car" | "walk" | "bicycle" | "public";


export interface Engineer {
  id: EngineerId;
  name: string;
  start: Point;

  shiftStart: Date;
  shiftEnd: Date;

  skills: Skill[];

  transport: Transport;
}
