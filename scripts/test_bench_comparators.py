"""Run with benchmark environment: python scripts/test_bench_comparators.py
Independent exhaustive tiny-route oracle; no testing dependency required.
"""
import copy
import itertools
import random
import time
from unittest.mock import patch

import bench_comparators as c


def job(start=0, end=20, duration=2, urgent=False, skill="local", transport=None):
    return dict(window_start=start, window_end=end, duration=duration,
                urgent=urgent, skill=skill, transport=transport)


def engineer(start=0, end=20, paid=False, skills=("local",), transport="car"):
    return dict(shift_start=start, shift_end=end, already_used=paid,
                skills=list(skills), transport=transport)


def data(jobs, engineers, times=None, distances=None):
    n, m = len(jobs), len(engineers)
    if times is None:
        times = [[[abs(i - j) for j in range(n)] for i in range(n + 1)] for _ in engineers]
    if distances is None:
        distances = [[[t * 10 if t < 100000 else 0 for t in row] for row in matrix] for matrix in times]
    max_distance = max([0] + [d for matrix in distances for row in matrix for d in row])
    staff = n * max_distance + 1
    unassigned = (m + 1) * staff
    urgent = (n + 1) * unassigned
    eligible = [[j["skill"] in e["skills"] and (j["transport"] is None or j["transport"] == e["transport"])
                 for j in jobs] for e in engineers]
    return dict(scenario=dict(jobs=jobs, engineers=engineers), n=n, m=m,
                minutes=times, metres=distances, eligible=eligible,
                weights=dict(staff=staff, unassigned=unassigned, urgent=urgent),
                seed_orders=[[] for _ in engineers])


def independent_score(d, orders):
    """Replay using scenario compatibility, not comparator helpers/eligible table."""
    s, assigned, metres = d["scenario"], [], 0
    for e, route in enumerate(orders):
        engineer = s["engineers"][e]
        clock, previous = engineer["shift_start"], d["n"]
        for j in route:
            if j in assigned or not 0 <= j < d["n"]:
                return None
            job = s["jobs"][j]
            if job["skill"] not in engineer["skills"] or job["transport"] not in (None, engineer["transport"]):
                return None
            clock += d["minutes"][e][previous][j]
            clock = max(clock, job["window_start"])
            if clock > job["window_end"]:
                return None
            clock += job["duration"]
            if clock > engineer["shift_end"]:
                return None
            metres += d["metres"][e][previous][j]
            previous = j
            assigned.append(j)
    missing = set(range(d["n"])) - set(assigned)
    return (sum(s["jobs"][j]["urgent"] for j in missing), len(missing),
            sum(e["already_used"] or bool(route) for e, route in zip(s["engineers"], orders)), metres)


def exhaustive(d):
    """All engineer/omission assignments and all per-engineer permutations."""
    best, winner = None, None
    for assignment in itertools.product(range(-1, d["m"]), repeat=d["n"]):
        groups = [[j for j, owner in enumerate(assignment) if owner == e] for e in range(d["m"])]
        for orders in itertools.product(*(itertools.permutations(group) for group in groups)):
            key = independent_score(d, orders)
            if key is not None and (best is None or key < best):
                best, winner = key, [list(row) for row in orders]
    return best, winner


def greedy_seed(d):
    orders = [[] for _ in range(d["m"])]
    jobs = d["scenario"]["jobs"]
    for j in sorted(range(d["n"]), key=lambda j: (not jobs[j]["urgent"], jobs[j]["window_end"], jobs[j]["window_start"])):
        choices = []
        for e in range(d["m"]):
            for pos in range(len(orders[e]) + 1):
                trial = copy.deepcopy(orders)
                trial[e].insert(pos, j)
                score = independent_score(d, trial)
                if score is not None:
                    choices.append((score, e, pos, trial))
        if choices:
            orders = min(choices)[-1]
    return orders


def cases():
    yield "metric", data([job(), job(), job(urgent=True)], [engineer(), engineer(paid=True)])
    # Job 1 cannot be reached directly, but job 0 repairs its base arc.
    yield "nonmetric_unreachable_base_repaired", data([job(end=5), job(end=5)], [engineer()],
                                                     [[[0, 1], [100000, 0], [1, 100000]]])
    yield "nonmetric_late_base_repaired", data([job(end=5), job(end=5)], [engineer()],
                                              [[[0, 1], [15, 0], [1, 19]]])
    yield "unreachable_between_jobs", data([job(), job()], [engineer()],
                                           [[[0, 100000], [100000, 0], [1, 1]]])
    yield "last_service_exceeds_shift", data([job(start=9, end=9, duration=2)], [engineer(end=10)], [[[0], [0]]])
    yield "last_service_exact_shift", data([job(start=9, end=9, duration=2)], [engineer(end=11)], [[[0], [0]]])
    yield "fixed_source_departure", data([job(start=102, end=102)], [engineer(start=100, end=120)], [[[0], [3]]])
    yield "source_travel_distance", data([job()], [engineer(), engineer()], [[[0], [1]], [[0], [1]]],
                                         [[[0], [100]], [[0], [2]]])
    yield "waiting_start_window_not_finish", data([job(start=10, end=10, duration=2)], [engineer(end=12)], [[[0], [1]]])
    yield "no_eligible_vehicle", data([job(skill="emergency", urgent=True)], [engineer()])
    yield "optional_restricted_vehicle", data([job(end=0, transport="walk")],
                                              [engineer(transport="walk"), engineer()], [[[0], [1]], [[0], [0]]])
    yield "different_skills_modes_bases", data([job(skill="emergency", transport="walk"), job()],
                                               [engineer(), engineer(skills=("emergency",), transport="walk")],
                                               [[[0, 3], [4, 0], [9, 1]], [[0, 1], [9, 0], [2, 9]]])
    yield "urgent_before_coverage", data([job(duration=10, urgent=True), job(duration=4), job(duration=4)],
                                         [engineer(end=10)], [[[0] * 3 for _ in range(4)]])
    yield "staff_before_metres_and_paid_constant", data([job()], [engineer(paid=True), engineer()],
                                                        [[[0], [1]], [[0], [1]]], [[[0], [1000]], [[0], [1]]])
    yield "coverage_before_staff", data([job(start=1, end=1), job(start=1, end=1)],
                                        [engineer(), engineer()], [[[0, 0], [0, 0], [0, 0]]] * 2)
    yield "empty_jobs_paid_staff", data([], [engineer(paid=True), engineer()])
    yield "empty_engineers", data([job(urgent=True)], [])
    rng = random.Random(1)
    for k in range(12):
        n, m = 4, 2
        jobs = [job(start=rng.randrange(4), end=rng.randrange(4, 12), duration=rng.randrange(1, 5),
                    urgent=bool(rng.randrange(2)), skill=rng.choice(["local", "emergency"])) for _ in range(n)]
        engineers = [engineer(start=e, end=rng.randrange(8, 16), paid=bool(rng.randrange(2)),
                              skills=("local", "emergency") if e else ("local",)) for e in range(m)]
        times = [[[0 if i == j else rng.choice([0, 1, 2, 5, 100000]) for j in range(n)] for i in range(n + 1)] for _ in range(m)]
        distances = [[[0 if t == 100000 else rng.randrange(20) for t in row] for row in mat] for mat in times]
        yield f"random_directed_{k}", data(jobs, engineers, times, distances)


def main():
    count, routing_gaps = 0, 0
    for name, d in cases():
        d["seed_orders"] = greedy_seed(d)
        optimum, best = exhaustive(d)
        seed_key = independent_score(d, d["seed_orders"])
        for solve in (c.solve_cpsat, c.solve_routing):
            result = solve(d, time.monotonic() + (1.0 if solve == c.solve_cpsat else 0.06))
            score = independent_score(d, result["orders"])
            assert score is not None and tuple(result["objective"]) == score, (name, result)
            assert optimum <= score <= seed_key, (name, optimum, score, seed_key)
            if solve == c.solve_cpsat:
                assert result["optimal"] and score == optimum, (name, optimum, result)
                assert abs(result["scalar_lower_bound"] - result["scalar_objective"]) < 0.5
            else:
                assert not result["optimal"]
                routing_gaps += score != optimum
        count += 1

    d = data([job(), job(), job()], [engineer(), engineer(paid=True)])
    d["seed_orders"] = greedy_seed(d)
    for solve in (c.solve_cpsat, c.solve_routing):
        expired = solve(d, time.monotonic() - 1)
        assert expired["orders"] == d["seed_orders"] and not expired["optimal"]
        calls = 0

        def interrupt(deadline):
            nonlocal calls
            calls += 1
            if calls == 6:
                raise c._Deadline

        with patch.object(c, "_check", interrupt):
            partial = solve(d, time.monotonic() + 1)
        assert calls == 6 and partial["orders"] == d["seed_orders"] and not partial["optimal"]
        bad = copy.deepcopy(d)
        bad["seed_orders"][0] = [0, 0]
        try:
            solve(bad, time.monotonic() - 1)
        except ValueError:
            pass
        else:
            raise AssertionError("Invalid seed was accepted")

    # Explicitly exercise UNKNOWN fallback and reject any model/seed proof conflict.
    with patch.object(c.cp_model.CpSolver, "solve", return_value=c.cp_model.UNKNOWN):
        result = c.solve_cpsat(d, time.monotonic() + 1)
        assert result["orders"] == d["seed_orders"] and not result["optimal"]
    try:
        c._result(d, d["seed_orders"], "OPTIMAL", [[], []], optimal=True)
    except AssertionError:
        pass
    else:
        raise AssertionError("Contradictory optimum accepted")
    print(f"PASS: {count} exhaustive metric/directed cases; CP-SAT globally optimal in all; "
          f"Routing valid/no worse than seed ({routing_gaps} heuristic gaps). "
          "Expired/build-timeout/UNKNOWN fallback, invalid seed, and proof-conflict checks passed.")


if __name__ == "__main__":
    main()
