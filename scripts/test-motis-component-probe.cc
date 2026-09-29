#include <cstdlib>
#include <iostream>
#include <utility>
#include <vector>

#include "osr/routing/component_probe.h"

void require(bool ok, char const* message) {
  if (!ok) {
    std::cerr << message << '\n';
    std::exit(1);
  }
}

int main() {
  // State includes arrival direction, not just the physical intersection.
  using node = std::pair<int, int>;
  auto probe = [](std::vector<node> const& seeds,
                  std::vector<std::pair<node, node>> const& edges,
                  int target, std::size_t limit, unsigned& expanded) {
    return osr::exhausted_component_misses<node>(
        [&](auto const& add) {
          for (auto n : seeds) add(n);
        },
        [&](node n, auto const& add) {
          ++expanded;
          for (auto [from, to] : edges) if (from == n) add(to);
        },
        [&](node n) { return n.first == target; }, limit);
  };
  unsigned expanded = 0;
  require(probe({{0, 0}}, {{{0, 0}, {1, 0}}, {{1, 0}, {0, 0}}}, 9, 256, expanded),
          "An exhausted directed cycle must prove the miss");
  require(expanded == 2, "Cycles must not expand repeatedly");
  expanded = 0;
  require(!probe({{0, 0}}, {{{0, 0}, {0, 1}}, {{0, 1}, {9, 0}}}, 9, 256, expanded),
          "Different arrival states at one intersection must remain distinct");
  require(expanded == 2, "The second arrival state must be expanded");
  expanded = 0;
  require(!probe({{9, 0}}, {}, 9, 256, expanded), "A target seed is inconclusive");
  require(expanded == 0, "A target seed must not expand");
  expanded = 0;
  require(!probe({{0, 0}, {2, 0}}, {{{2, 0}, {9, 0}}}, 9, 256, expanded),
          "Both endpoint seeds must be considered");
  expanded = 0;
  require(!probe({{0, 0}}, {{{0, 0}, {1, 0}}, {{1, 0}, {9, 0}}}, 9, 1, expanded),
          "A work-limit hit must never mean unreachable");
  require(expanded == 1, "Work must stay bounded");
  expanded = 0;
  require(!osr::exhausted_component_misses<int>(
              [](auto const& add) { add(0); },
              [&](int n, auto const& add) { ++expanded; add(n + 1); },
              [](int) { return false; }),
          "An unbounded chain must remain inconclusive");
  require(expanded == 256, "The production cap must bound even an infinite graph");
  std::cout << "PASS: directed cycles, arrival states, multiple seeds, targets, work cap\n";
}
