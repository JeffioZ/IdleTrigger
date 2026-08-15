package app

import (
	"errors"
	"sync/atomic"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/displaytopology"
)

func TestMergeThemeTransitionRequestsKeepsLatestTargetAndEnvironmentRisk(t *testing.T) {
	config := testThemeCoordinatorConfig()
	now := time.Now()
	existing := mergeThemeTransitionRequests(nil, themeTransitionRequest{
		source: themeSourceDisplay, displayChanged: true,
	}, now, config)
	merged := mergeThemeTransitionRequests(&existing, themeTransitionRequest{
		source: themeSourceBattery, target: theme.ModeDark, hasTarget: true,
	}, now.Add(time.Millisecond), config)
	if !merged.hasTarget || merged.target != theme.ModeDark || merged.source != themeSourceBattery || !merged.displayChanged {
		t.Fatalf("merged request lost target or display risk: %+v", merged)
	}

	merged = mergeThemeTransitionRequests(&merged, themeTransitionRequest{
		source: themeSourceSchedule, target: theme.ModeLight, hasTarget: true,
	}, now.Add(2*time.Millisecond), config)
	if merged.target != theme.ModeLight || merged.source != themeSourceSchedule {
		t.Fatalf("latest automatic target did not win: %+v", merged)
	}
}

func TestMergeThemeTransitionRequestsManualRepairReplacesAutomaticAndUpgradesManualSwitch(t *testing.T) {
	config := testThemeCoordinatorConfig()
	now := time.Now()
	automatic := mergeThemeTransitionRequests(nil, themeTransitionRequest{
		source: themeSourceBattery, target: theme.ModeDark, hasTarget: true,
	}, now, config)
	manualRepair := mergeThemeTransitionRequests(&automatic, themeTransitionRequest{
		source: themeSourceManualRepair, manual: true, fullRepair: true,
	}, now.Add(time.Millisecond), config)
	if !manualRepair.manual || !manualRepair.fullRepair || manualRepair.hasTarget {
		t.Fatalf("manual repair did not replace stale automatic target: %+v", manualRepair)
	}

	manualSwitch := mergeThemeTransitionRequests(nil, themeTransitionRequest{
		source: themeSourceManualSwitch, target: theme.ModeLight, hasTarget: true, manual: true,
	}, now, config)
	upgraded := mergeThemeTransitionRequests(&manualSwitch, themeTransitionRequest{
		source: themeSourceManualRepair, manual: true, fullRepair: true,
	}, now.Add(time.Millisecond), config)
	if !upgraded.hasTarget || upgraded.target != theme.ModeLight || !upgraded.fullRepair {
		t.Fatalf("manual repair did not upgrade pending manual switch: %+v", upgraded)
	}
}

func TestMergeThemeTransitionRequestsCapsRepeatedEventsAtHardDeadline(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.automaticGrace = time.Minute
	config.maxWait = 8 * time.Second
	now := time.Now()
	request := mergeThemeTransitionRequests(nil, themeTransitionRequest{source: themeSourceDisplay, displayChanged: true}, now, config)
	request = mergeThemeTransitionRequests(&request, themeTransitionRequest{source: themeSourceDisplay, displayChanged: true}, now.Add(7*time.Second), config)
	if want := now.Add(config.maxWait); !request.dueAt.Equal(want) {
		t.Fatalf("dueAt = %s, want hard deadline %s", request.dueAt, want)
	}
}

func TestWaitForStableThemeTopologyRequiresConsecutiveMatches(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.topologySamples = 3
	config.topologyProbe = time.Millisecond
	a := displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}}}
	b := displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}, {TargetID: 2}}}
	sequence := []displaytopology.Snapshot{a, b, b, b}
	var calls atomic.Int32
	query := func() (displaytopology.Snapshot, error) {
		index := int(calls.Add(1)) - 1
		if index >= len(sequence) {
			index = len(sequence) - 1
		}
		return sequence[index], nil
	}
	now := time.Now
	result := waitForStableThemeTopology(7, now().Add(100*time.Millisecond), make(chan struct{}), config, now, query)
	if !result.stable || result.generation != 7 || result.snapshot.ActiveDisplayCount() != 2 || calls.Load() != 4 {
		t.Fatalf("stable topology result = %+v calls=%d", result, calls.Load())
	}
}

func TestWaitForStableThemeTopologyStopsAtDeadlineOnQueryFailure(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.topologyProbe = time.Millisecond
	now := time.Now
	result := waitForStableThemeTopology(1, now().Add(5*time.Millisecond), make(chan struct{}), config, now,
		func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{}, errors.New("query failed")
		})
	if result.stable || result.err == nil || result.waited < 4*time.Millisecond {
		t.Fatalf("deadline result = %+v", result)
	}
}

func TestThemeTopologyProbeIgnoresCanceledGenerationAfterReplacementStarts(t *testing.T) {
	var probe themeTopologyWaitState
	oldCancel := probe.begin(1)
	probe.cancelWait()
	select {
	case <-oldCancel:
	default:
		t.Fatal("replaced probe was not canceled")
	}
	currentCancel := probe.begin(2)
	if probe.finish(1) {
		t.Fatal("stale probe result completed the current generation")
	}
	if !probe.active() {
		t.Fatal("stale probe result cleared the current cancel handle")
	}
	select {
	case <-currentCancel:
		t.Fatal("stale probe result canceled the current generation")
	default:
	}
	if !probe.finish(2) || probe.active() {
		t.Fatal("current probe result did not complete its own generation")
	}
}

func TestThemeCoordinatorUsesFullRepairForAutomaticMultiDisplaySwitch(t *testing.T) {
	config := testThemeCoordinatorConfig()
	current := atomic.Int32{}
	current.Store(int32(theme.ModeLight))
	var switches, repairs, broadcasts atomic.Int32
	results := make(chan themeTransitionResult, 2)
	coordinator := newThemeTransitionCoordinator(config, themeCoordinatorDeps{
		now: time.Now,
		queryTopology: func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}, {TargetID: 2}}}, nil
		},
		currentTheme: func() theme.Mode { return theme.Mode(current.Load()) },
		switchTheme: func(mode theme.Mode) error {
			switches.Add(1)
			current.Store(int32(mode))
			return nil
		},
		rebroadcastTheme:    func() { broadcasts.Add(1) },
		fullRepairTheme:     func() error { repairs.Add(1); return nil },
		fullRepairAvailable: func() bool { return true },
		onComplete:          func(result themeTransitionResult) { results <- result },
	})
	t.Cleanup(coordinator.stop)

	coordinator.submit(themeTransitionRequest{source: themeSourceBattery, target: theme.ModeDark, hasTarget: true})
	result := waitThemeCoordinatorResult(t, results)
	if result.err != nil || !result.switched || !result.fullRepairTried || switches.Load() != 1 || repairs.Load() != 1 || broadcasts.Load() != 0 {
		t.Fatalf("multi-display transition = %+v switches=%d repairs=%d broadcasts=%d", result, switches.Load(), repairs.Load(), broadcasts.Load())
	}
}

func TestThemeCoordinatorCancelAutomaticDropsPendingSwitch(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.automaticGrace = 40 * time.Millisecond
	results := make(chan themeTransitionResult, 1)
	var switches atomic.Int32
	coordinator := newThemeTransitionCoordinator(config, themeCoordinatorDeps{
		now: time.Now,
		queryTopology: func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}}}, nil
		},
		currentTheme:        func() theme.Mode { return theme.ModeLight },
		switchTheme:         func(theme.Mode) error { switches.Add(1); return nil },
		rebroadcastTheme:    func() {},
		fullRepairTheme:     func() error { return nil },
		fullRepairAvailable: func() bool { return true },
		onComplete:          func(result themeTransitionResult) { results <- result },
	})
	t.Cleanup(coordinator.stop)

	coordinator.submit(themeTransitionRequest{source: themeSourceBattery, target: theme.ModeDark, hasTarget: true})
	coordinator.cancelAutomatic()
	select {
	case result := <-results:
		t.Fatalf("canceled transition completed: %+v", result)
	case <-time.After(100 * time.Millisecond):
	}
	if switches.Load() != 0 {
		t.Fatalf("canceled transition switched theme %d times", switches.Load())
	}
}

func TestThemeCoordinatorCancelAutomaticPreventsBusyRecovery(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.postSwitchDelay = 40 * time.Millisecond
	current := atomic.Int32{}
	current.Store(int32(theme.ModeLight))
	switched := make(chan struct{})
	results := make(chan themeTransitionResult, 2)
	var repairs atomic.Int32
	coordinator := newThemeTransitionCoordinator(config, themeCoordinatorDeps{
		now: time.Now,
		queryTopology: func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}, {TargetID: 2}}}, nil
		},
		currentTheme: func() theme.Mode { return theme.Mode(current.Load()) },
		switchTheme: func(mode theme.Mode) error {
			current.Store(int32(mode))
			close(switched)
			return nil
		},
		rebroadcastTheme:    func() {},
		fullRepairTheme:     func() error { repairs.Add(1); return nil },
		fullRepairAvailable: func() bool { return true },
		onComplete:          func(result themeTransitionResult) { results <- result },
	})
	t.Cleanup(coordinator.stop)

	coordinator.submit(themeTransitionRequest{source: themeSourceBattery, target: theme.ModeDark, hasTarget: true})
	select {
	case <-switched:
	case <-time.After(time.Second):
		t.Fatal("theme switch did not start")
	}
	coordinator.cancelAutomatic()
	first := waitThemeCoordinatorResult(t, results)
	if first.fullRepairTried || !first.rebroadcast || repairs.Load() != 0 {
		t.Fatalf("canceled busy transition performed full repair: %+v repairs=%d", first, repairs.Load())
	}
	select {
	case result := <-results:
		t.Fatalf("cancel scheduled an automatic recovery: %+v", result)
	case <-time.After(100 * time.Millisecond):
	}
}

func TestThemeCoordinatorDoesNotInterruptRepairAndRateLimitsFollowUp(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.fullRepairRateLimit = time.Hour
	current := atomic.Int32{}
	current.Store(int32(theme.ModeLight))
	repairStarted := make(chan struct{})
	releaseRepair := make(chan struct{})
	results := make(chan themeTransitionResult, 3)
	var repairs, broadcasts atomic.Int32
	coordinator := newThemeTransitionCoordinator(config, themeCoordinatorDeps{
		now: time.Now,
		queryTopology: func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}, {TargetID: 2}}}, nil
		},
		currentTheme:     func() theme.Mode { return theme.Mode(current.Load()) },
		switchTheme:      func(mode theme.Mode) error { current.Store(int32(mode)); return nil },
		rebroadcastTheme: func() { broadcasts.Add(1) },
		fullRepairTheme: func() error {
			if repairs.Add(1) == 1 {
				close(repairStarted)
				<-releaseRepair
			}
			return nil
		},
		fullRepairAvailable: func() bool { return true },
		onComplete:          func(result themeTransitionResult) { results <- result },
	})
	t.Cleanup(coordinator.stop)

	coordinator.submit(themeTransitionRequest{source: themeSourceBattery, target: theme.ModeDark, hasTarget: true})
	select {
	case <-repairStarted:
	case <-time.After(time.Second):
		t.Fatal("full repair did not start")
	}
	coordinator.submit(themeTransitionRequest{source: themeSourceDisplay, displayChanged: true})
	close(releaseRepair)
	first := waitThemeCoordinatorResult(t, results)
	second := waitThemeCoordinatorResult(t, results)
	if !first.fullRepairTried || second.fullRepairTried || repairs.Load() != 1 || broadcasts.Load() != 1 {
		t.Fatalf("repair/follow-up results = first:%+v second:%+v repairs=%d broadcasts=%d", first, second, repairs.Load(), broadcasts.Load())
	}
}

func TestThemeCoordinatorDefersFullRepairWhenNewDisplayEventArrivesAfterSwitch(t *testing.T) {
	config := testThemeCoordinatorConfig()
	config.postSwitchDelay = 40 * time.Millisecond
	current := atomic.Int32{}
	current.Store(int32(theme.ModeLight))
	switched := make(chan struct{})
	results := make(chan themeTransitionResult, 3)
	var repairs atomic.Int32
	coordinator := newThemeTransitionCoordinator(config, themeCoordinatorDeps{
		now: time.Now,
		queryTopology: func() (displaytopology.Snapshot, error) {
			return displaytopology.Snapshot{Paths: []displaytopology.Path{{TargetID: 1}, {TargetID: 2}}}, nil
		},
		currentTheme: func() theme.Mode { return theme.Mode(current.Load()) },
		switchTheme: func(mode theme.Mode) error {
			current.Store(int32(mode))
			close(switched)
			return nil
		},
		rebroadcastTheme:    func() {},
		fullRepairTheme:     func() error { repairs.Add(1); return nil },
		fullRepairAvailable: func() bool { return true },
		onComplete:          func(result themeTransitionResult) { results <- result },
	})
	t.Cleanup(coordinator.stop)

	coordinator.submit(themeTransitionRequest{source: themeSourceBattery, target: theme.ModeDark, hasTarget: true})
	select {
	case <-switched:
	case <-time.After(time.Second):
		t.Fatal("theme switch did not start")
	}
	coordinator.submit(themeTransitionRequest{source: themeSourceDisplay, displayChanged: true})
	first := waitThemeCoordinatorResult(t, results)
	if first.fullRepairTried || !first.rebroadcast || !first.retryWhenStable || first.topologyStable {
		t.Fatalf("stale operation was not safely deferred: %+v", first)
	}
	second := waitThemeCoordinatorResult(t, results)
	if !second.fullRepairTried || repairs.Load() != 1 {
		t.Fatalf("stable follow-up did not perform one repair: %+v repairs=%d", second, repairs.Load())
	}
}

func testThemeCoordinatorConfig() themeCoordinatorConfig {
	return themeCoordinatorConfig{
		automaticGrace:      time.Millisecond,
		topologyProbe:       time.Millisecond,
		topologySamples:     1,
		maxWait:             100 * time.Millisecond,
		postSwitchDelay:     0,
		fullRepairRateLimit: time.Second,
		recentTransition:    time.Second,
	}
}

func waitThemeCoordinatorResult(t *testing.T, results <-chan themeTransitionResult) themeTransitionResult {
	t.Helper()
	select {
	case result := <-results:
		return result
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for theme coordinator")
		return themeTransitionResult{}
	}
}
