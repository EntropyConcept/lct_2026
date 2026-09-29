import type { AssignedEngineer, AssignedJob, PlanResult, Scenario } from "./types";

export function getAssignedEngineers(scenario:Scenario, result: PlanResult): AssignedEngineer[] {
    return scenario.engineers.map((engineer) => {
        const stops = result.plan.routes.find(route => route.engineerId === engineer.id)?.stops;
        if (!stops?.length) {
            return {
                ...engineer,
                assigned: false,
            }
        }
        return {
            ...engineer,
            assigned: true,
            stops,
        }
    });
}

export function getAssignedJobs(scenario:Scenario, result: PlanResult): AssignedJob[] {
    return scenario.jobs.map((job) => {
        const route = result.plan.routes.find(route => route.stops.find(stop => stop.jobId === job.id));
        const stop = route?.stops.find(stop => stop.jobId === job.id);
        const engineer = scenario.engineers.find(engineer => route?.engineerId === engineer.id);
        const unassigned = result.plan.unassigned.find(unassignedJob => unassignedJob.jobId === job.id);

        if (!route || !stop || !engineer || unassigned) {
            return {
                ...job,
                assigned: false,
                reason: unassigned?.reason ?? 'Не удалось подобрать исполнителя',
            }
        }
        return {
            ...job,
            assigned: true,
            stop,
            engineer,
        }
    });
}
