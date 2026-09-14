#include "performance_capture.h"
#include <array>
#include <cassert>
#include <chrono>
#include <cstdint>
#include <string>
#include <vector>

using Clock = std::chrono::steady_clock;
struct Sample { int scope; unsigned long turn; uint64_t ns; };
struct Scope { Clock::time_point start; unsigned long turn; bool active = false; uint64_t total_ns = 0; };
struct Profile {
    bool active = true, draw_breakdown = false;
    std::array<Scope, PerfScopeCount> scopes;
    std::vector<Sample> samples;
    Clock::time_point last_present;
    std::string error;
};
Profile current;
Profile& profile() { return current; }
unsigned long get_gameturn() { return 40; }
void fail(Profile& p, const char* error) { p.error = error; p.active = false; }
#include "timing.inc"

int main()
{
    performance_begin(PerfPresentation);
    const auto anchor = current.last_present;
    performance_begin(PerfReplay);
    current.scopes[PerfReplay].start -= std::chrono::milliseconds(2);
    current.scopes[PerfPresentation].start -= std::chrono::milliseconds(4);
    performance_end(PerfReplay);
    performance_begin(PerfPresentWait);
    performance_end(PerfPresentWait);
    performance_end(PerfPresentation);
    assert(current.error.empty());
    assert(current.samples.size() == 3);
    assert(current.samples[0].scope == PerfPresentWait);
    assert(current.samples[1].scope == PerfReplay);
    assert(current.samples[2].scope == PerfPresentation);
    assert(current.samples[1].ns >= 2000000);
    assert(current.samples[2].ns >= 2000000);
    assert(current.samples[2].ns < 4000000);
    assert(current.last_present == anchor);
    performance_begin(PerfPresentation);
    performance_begin(PerfPresentWait);
    performance_end(PerfPresentWait);
    performance_end(PerfPresentation);
    assert(current.samples[3].scope == PerfScopeCount);
    assert(current.samples[5].scope == PerfReplay && current.samples[5].ns == 0);
    assert(std::string(PerformanceScopeNames[PerfReplay]) == "replay");
    assert(std::string(PerformanceScopeNames[PerfPresentation]) == "presentation");
    assert(std::string(PerformanceScopeNames[PerfScopeCount]) == "frame_interval");

    current = {};
    performance_begin(PerfReplay);
    assert(!current.error.empty());
    current = {};
    performance_begin(PerfPresentation);
    performance_begin(PerfReplay);
    performance_begin(PerfPresentWait);
    assert(!current.error.empty());
    current = {};
    performance_begin(PerfPresentation);
    performance_begin(PerfReplay);
    performance_end(PerfPresentation);
    assert(!current.error.empty());
    current = {};
    current.draw_breakdown = true;
    performance_begin(PerfDraw);
    performance_begin(PerfDrawRaster);
    performance_end(PerfDrawRaster);
    performance_end(PerfDraw);
    assert(current.error.empty());
    assert(current.samples.size() == 5);
    assert(current.samples[0].scope == PerfDrawScene && current.samples[0].ns == 0);
    assert(current.samples[4].scope == PerfDraw);
}
