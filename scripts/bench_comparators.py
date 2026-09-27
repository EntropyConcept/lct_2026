"""Benchmark-only OR-Tools comparators; scenario Job.duration is total service.

Call with an absolute monotonic deadline, after charging matrix/seed preparation
against the same outer budget. Native solver calls and final replay can slightly
overshoot wall limits; no incomplete model is ever solved or reported as proven.
"""
import time

from ortools.constraint_solver import pywrapcp, routing_enums_pb2
from ortools.sat.python import cp_model


class _Deadline(Exception):
    pass


def _check(deadline):
    if time.monotonic() >= deadline:
        raise _Deadline


def replay(data, orders):
    """Validate complete orders and return (lex objective, earliest starts)."""
    n, m = data["n"], data["m"]
    jobs, engineers = data["scenario"]["jobs"], data["scenario"]["engineers"]
    if len(orders) != m:
        raise ValueError("Wrong number of engineer routes")
    seen, starts, distance, staff = set(), {}, 0, 0
    for e, order in enumerate(orders):
        engineer = engineers[e]
        staff += bool(order) or engineer.get("already_used", False)
        end, previous = engineer["shift_start"], n
        for j in order:
            if not isinstance(j, int) or not 0 <= j < n or j in seen:
                raise ValueError("Unknown or duplicate job")
            if not data["eligible"][e][j]:
                raise ValueError("Incompatible job")
            job = jobs[j]
            start = max(end + data["minutes"][e][previous][j], job["window_start"])
            end = start + job["duration"]
            if start > job["window_end"] or end > engineer["shift_end"]:
                raise ValueError("Infeasible route timing")
            starts[j] = start
            distance += data["metres"][e][previous][j]
            previous = j
            seen.add(j)
    urgent = sum(bool(job.get("urgent", False)) for j, job in enumerate(jobs) if j not in seen)
    return (urgent, n - len(seen), staff, distance), starts


def _scalar(data, key):
    w = data["weights"]
    return key[0] * w["urgent"] + key[1] * w["unassigned"] + key[2] * w["staff"] + key[3]


def _result(data, seed, status, candidate=None, optimal=False, **details):
    seed_key, _ = replay(data, seed)
    orders, key = seed, seed_key
    if candidate is not None:
        candidate_key, _ = replay(data, candidate)
        if optimal and seed_key < candidate_key:
            raise AssertionError("CP-SAT OPTIMAL contradicts the validated seed")
        if candidate_key < key:
            orders, key = candidate, candidate_key
    return {"orders": [list(row) for row in orders], "status": status,
            "optimal": optimal, "incumbent_origin": "greedy_seed" if orders is seed else "solver",
            "objective": list(key),
            "scalar_objective": _scalar(data, key), **details}


def solve_routing(data, deadline):
    """Native single-thread RoutingModel; NEVER claims a global optimum."""
    seed = [list(row) for row in data["seed_orders"]]
    replay(data, seed)
    n, m = data["n"], data["m"]
    if m == 0:
        return _result(data, seed, "NO_VEHICLES")
    jobs, engineers = data["scenario"]["jobs"], data["scenario"]["engineers"]
    try:
        _check(deadline)
        sink = n + m
        manager = pywrapcp.RoutingIndexManager(n + m + 1, m, list(range(n, n + m)), [sink] * m)
        routing = pywrapcp.RoutingModel(manager)
        evaluators = []
        # Retain closures throughout search; each vehicle has its own directed matrix.
        callbacks = []
        for e in range(m):
            _check(deadline)

            def travel(a, b, e=e):
                i, j = manager.IndexToNode(a), manager.IndexToNode(b)
                service = jobs[i]["duration"] if i < n else 0
                return service + (0 if j == sink else data["minutes"][e][i if i < n else n][j])

            def distance(a, b, e=e):
                i, j = manager.IndexToNode(a), manager.IndexToNode(b)
                return 0 if j == sink else data["metres"][e][i if i < n else n][j]

            callbacks.extend([travel, distance])
            evaluators.append(routing.RegisterTransitCallback(travel))
            routing.SetArcCostEvaluatorOfVehicle(routing.RegisterTransitCallback(distance), e)
            routing.SetFixedCostOfVehicle(0 if engineers[e].get("already_used", False)
                                          else data["weights"]["staff"], e)
        horizon = max([0] + [j["window_end"] for j in jobs] + [e["shift_end"] for e in engineers])
        routing.AddDimensionWithVehicleTransits(evaluators, horizon, horizon, False, "Time")
        dimension = routing.GetDimensionOrDie("Time")
        for e, engineer in enumerate(engineers):
            _check(deadline)
            dimension.CumulVar(routing.Start(e)).SetValue(engineer["shift_start"])
            dimension.CumulVar(routing.End(e)).SetRange(engineer["shift_start"], engineer["shift_end"])
        for j, job in enumerate(jobs):
            _check(deadline)
            index = manager.NodeToIndex(j)
            dimension.CumulVar(index).SetRange(job["window_start"], job["window_end"])
            penalty = data["weights"]["unassigned"] + bool(job.get("urgent", False)) * data["weights"]["urgent"]
            routing.AddDisjunction([index], penalty)
            allowed = [e for e in range(m) if data["eligible"][e][j]]
            if allowed:
                # -1 is the vehicle of an omitted job, and must stay in the domain.
                routing.VehicleVar(index).SetValues([-1] + allowed)
            else:
                # SetAllowedVehiclesForIndex([]) would mean unrestricted, not none.
                routing.ActiveVar(index).SetValue(0)
        params = pywrapcp.DefaultRoutingSearchParameters()
        params.first_solution_strategy = routing_enums_pb2.FirstSolutionStrategy.PARALLEL_CHEAPEST_INSERTION
        params.local_search_metaheuristic = routing_enums_pb2.LocalSearchMetaheuristic.GUIDED_LOCAL_SEARCH
        params.sat_parameters.num_search_workers = 1
        params.sat_parameters.random_seed = 1
        params.fallback_to_cp_sat_size_threshold = 0  # Keep this comparator native Routing search.
        _check(deadline)
        params.time_limit.FromNanoseconds(max(1, int((deadline - time.monotonic()) * 1e9)))
        routing.CloseModelWithParameters(params)
        _check(deadline)
        assignment = routing.ReadAssignmentFromRoutes(seed, True)
        _check(deadline)
        params.time_limit.FromNanoseconds(max(1, int((deadline - time.monotonic()) * 1e9)))
        answer = (routing.SolveFromAssignmentWithParameters(assignment, params) if assignment is not None
                  else routing.SolveWithParameters(params))
        status = f"ROUTING_{routing.status()}"
        if answer is None:
            return _result(data, seed, status)
        orders = [[] for _ in range(m)]
        for e in range(m):
            index = answer.Value(routing.NextVar(routing.Start(e)))
            while not routing.IsEnd(index):
                if len(orders[e]) >= n:
                    raise AssertionError("Routing produced a cycle")
                orders[e].append(manager.IndexToNode(index))
                index = answer.Value(routing.NextVar(index))
        key, _ = replay(data, orders)
        paid = sum(e.get("already_used", False) for e in engineers)
        if answer.ObjectiveValue() + paid * data["weights"]["staff"] != _scalar(data, key):
            raise AssertionError("Routing objective disagrees with route replay")
        return _result(data, seed, status, orders)
    except _Deadline:
        return _result(data, seed, "BUILD_OR_WARM_START_TIMEOUT")


def solve_cpsat(data, deadline):
    """Complete per-engineer circuits, global service-start variables, seed 1."""
    seed = [list(row) for row in data["seed_orders"]]
    _, seed_starts = replay(data, seed)
    n, m = data["n"], data["m"]
    jobs, engineers = data["scenario"]["jobs"], data["scenario"]["engineers"]
    w = data["weights"]
    try:
        _check(deadline)
        model = cp_model.CpModel()
        starts, missed, assignments, all_arcs = [], [], [], []
        terms = []
        for j, job in enumerate(jobs):
            _check(deadline)
            start = model.new_int_var(job["window_start"], job["window_end"], f"t{j}")
            omit = model.new_bool_var(f"omit{j}")
            starts.append(start)
            missed.append(omit)
            model.add_hint(start, seed_starts.get(j, job["window_start"]))
            model.add_hint(omit, int(j not in seed_starts))
            terms.append((w["unassigned"] + bool(job.get("urgent", False)) * w["urgent"]) * omit)
        for e, engineer in enumerate(engineers):
            _check(deadline)
            used = model.new_bool_var(f"used{e}")
            model.add_hint(used, int(bool(seed[e])))
            terms.append(w["staff"] if engineer.get("already_used", False) else w["staff"] * used)
            row, arcs, arc_vars = [], [(n, n, used.Not())], {}
            seed_jobs = set(seed[e])
            seed_arcs = set(zip([n] + seed[e], seed[e] + [n])) if seed[e] else set()
            for j, job in enumerate(jobs):
                _check(deadline)
                assigned = model.new_bool_var(f"x{e}_{j}")
                row.append(assigned)
                model.add_hint(assigned, int(j in seed_jobs))
                arcs.append((j, j, assigned.Not()))
                model.add(assigned <= used)
                if not data["eligible"][e][j]:
                    model.add(assigned == 0)
                # No direct-base lower bound: a different first job can repair it.
                model.add(starts[j] >= engineer["shift_start"]).only_enforce_if(assigned)
                model.add(starts[j] + job["duration"] <= engineer["shift_end"]).only_enforce_if(assigned)
            model.add(sum(row) >= used)
            earliest = [max(job['window_start'], engineer['shift_start']) for job in jobs]
            latest = [min(job['window_end'], engineer['shift_end'] - job['duration']) for job in jobs]
            possible = [data['eligible'][e][j] and earliest[j] <= latest[j] for j in range(n)]
            for i in range(n + 1):
                _check(deadline)
                for j in range(n + 1):
                    _check(deadline)
                    if i == j or (i < n and not possible[i]) or (j < n and not possible[j]):
                        continue
                    if j < n:
                        lower = engineer['shift_start'] if i == n else earliest[i] + jobs[i]['duration']
                        if lower + data['minutes'][e][i][j] > latest[j]:
                            continue  # This ARC is impossible; assignment via another predecessor remains legal.
                    arc = model.new_bool_var(f"a{e}_{i}_{j}")
                    arcs.append((i, j, arc))
                    arc_vars[i, j] = arc
                    model.add_hint(arc, int((i, j) in seed_arcs))
                    if j == n:
                        # Virtual sink: no travel home, but final service must fit.
                        model.add(starts[i] + jobs[i]["duration"] <= engineer["shift_end"]).only_enforce_if(arc)
                    else:
                        departure = engineer["shift_start"] if i == n else starts[i] + jobs[i]["duration"]
                        model.add(starts[j] >= departure + data["minutes"][e][i][j]).only_enforce_if(arc)
                        terms.append(data["metres"][e][i][j] * arc)
            model.add_circuit(arcs)
            assignments.append(row)
            all_arcs.append(arc_vars)
        for j in range(n):
            _check(deadline)
            model.add(sum(assignments[e][j] for e in range(m)) + missed[j] == 1)
        model.minimize(sum(terms))
        _check(deadline)
        solver = cp_model.CpSolver()
        solver.parameters.num_search_workers = 1
        solver.parameters.random_seed = 1
        solver.parameters.relative_gap_limit = 0
        solver.parameters.absolute_gap_limit = 0
        _check(deadline)
        solver.parameters.max_time_in_seconds = deadline - time.monotonic()
        if solver.parameters.max_time_in_seconds <= 0:
            raise _Deadline
        status = solver.solve(model)
        name = solver.status_name(status)
        if status in (cp_model.INFEASIBLE, cp_model.MODEL_INVALID):
            raise AssertionError(f"CP-SAT {name} despite validated feasible seed: {solver.solution_info()}")
        if status not in (cp_model.FEASIBLE, cp_model.OPTIMAL):
            return _result(data, seed, name)
        orders = [[] for _ in range(m)]
        for e, arcs in enumerate(all_arcs):
            successors = {i: j for (i, j), arc in arcs.items() if solver.boolean_value(arc)}
            at = successors.get(n, n)
            while at != n:
                if len(orders[e]) >= n:
                    raise AssertionError("CP-SAT produced a disconnected cycle")
                orders[e].append(at)
                at = successors[at]
        key, _ = replay(data, orders)
        assigned_jobs = {j for row in orders for j in row}
        for j in range(n):
            if solver.boolean_value(missed[j]) != (j not in assigned_jobs):
                raise AssertionError("CP-SAT circuit decode omits an assigned job")
        # Evaluate the integer expression, not the rounded floating objective API.
        if solver.value(sum(terms)) != _scalar(data, key):
            raise AssertionError("CP-SAT objective disagrees with route replay")
        return _result(data, seed, name, orders, status == cp_model.OPTIMAL,
                       scalar_lower_bound=solver.best_objective_bound)
    except _Deadline:
        return _result(data, seed, "BUILD_TIMEOUT")
