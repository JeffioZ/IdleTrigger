package idle

import (
	"sync/atomic"
	"testing"
	"time"
)

func TestNegativeWarningOffsetIsClamped(t *testing.T) {
	m := New(time.Minute, -time.Second, nil, nil, time.Second)
	if m.warningOffset != 0 {
		t.Fatalf("warning offset = %s, want 0", m.warningOffset)
	}
}

func TestElapsedSinceLastInputHandlesWrapAndSyntheticFutureTime(t *testing.T) {
	if got := elapsedSinceLastInput(25, ^uint32(0)-24); got != 50*time.Millisecond {
		t.Fatalf("wrapped elapsed = %s, want 50ms", got)
	}
	if got := elapsedSinceLastInput(100, 200); got != 0 {
		t.Fatalf("future input timestamp should reset idle, got %s", got)
	}
}

func TestIdleDurationUsesMillisecondsOnce(t *testing.T) {
	if got := elapsedSinceLastInput(17_750, 0); got != 17750*time.Millisecond {
		t.Fatalf("elapsed = %s, want 17.75s", got)
	}
}

func TestStartStopCanRepeat(t *testing.T) {
	m := New(time.Hour, 0, nil, nil, time.Millisecond)
	for i := 0; i < 3; i++ {
		m.Start()
		m.Stop()
	}
}

func TestTriggerDoesNotDependOnWarning(t *testing.T) {
	triggered := make(chan struct{}, 1)
	m := New(30*time.Millisecond, 15*time.Millisecond, nil, func() { triggered <- struct{}{} }, time.Millisecond)
	m.idleDuration = func() (time.Duration, error) { return 2 * time.Minute, nil }
	m.inputTimestamp = func() (uint32, error) { return 1, nil }
	m.Start()
	defer m.Stop()

	select {
	case <-triggered:
	case <-time.After(time.Second):
		t.Fatal("trigger did not fire after the idle threshold")
	}
}

func TestStartDoesNotInheritExistingIdleDuration(t *testing.T) {
	triggered := make(chan struct{}, 1)
	m := New(80*time.Millisecond, 0, nil, func() { triggered <- struct{}{} }, time.Millisecond)
	m.idleDuration = func() (time.Duration, error) { return time.Hour, nil }
	m.inputTimestamp = func() (uint32, error) { return 1, nil }
	m.Start()
	defer m.Stop()

	select {
	case <-triggered:
		t.Fatal("monitor triggered using idle time from before Start")
	case <-time.After(30 * time.Millisecond):
	}

	select {
	case <-triggered:
	case <-time.After(time.Second):
		t.Fatal("monitor did not trigger after its own timeout window")
	}
}

func TestTriggerStartsFreshTimeoutWindow(t *testing.T) {
	triggered := make(chan struct{}, 2)
	m := New(40*time.Millisecond, 0, nil, func() { triggered <- struct{}{} }, time.Millisecond)
	m.idleDuration = func() (time.Duration, error) { return time.Hour, nil }
	m.inputTimestamp = func() (uint32, error) { return 1, nil }
	m.Start()
	defer m.Stop()

	select {
	case <-triggered:
	case <-time.After(time.Second):
		t.Fatal("first trigger did not fire")
	}
	select {
	case <-triggered:
		t.Fatal("trigger repeated without a fresh timeout window")
	case <-time.After(20 * time.Millisecond):
	}
	select {
	case <-triggered:
	case <-time.After(time.Second):
		t.Fatal("fresh timeout window did not trigger")
	}
}

func TestActivityCallbackFollowsWarning(t *testing.T) {
	warned := make(chan struct{}, 1)
	activity := make(chan struct{}, 1)
	var input atomic.Uint32
	input.Store(1)
	// Make the warning eligible on the first sample. The test exercises the
	// warning-to-activity transition, not scheduler timing within a short window.
	m := New(time.Hour, time.Hour, func() {
		input.Store(2)
		warned <- struct{}{}
	}, nil, time.Millisecond)
	m.idleDuration = func() (time.Duration, error) { return time.Hour, nil }
	m.inputTimestamp = func() (uint32, error) { return input.Load(), nil }
	m.SetOnActivity(func() { activity <- struct{}{} })
	m.Start()
	defer m.Stop()

	select {
	case <-warned:
	case <-time.After(time.Second):
		t.Fatal("warning did not fire")
	}
	select {
	case <-activity:
	case <-time.After(time.Second):
		t.Fatal("activity callback did not fire after warning")
	}
}

func TestInputResetCallbackReportsTimestampChange(t *testing.T) {
	sampled := make(chan struct{}, 1)
	resets := make(chan InputReset, 1)
	var input atomic.Uint32
	input.Store(100)
	m := New(time.Hour, 30*time.Second, nil, nil, time.Millisecond)
	m.idleDuration = func() (time.Duration, error) { return time.Hour, nil }
	m.inputTimestamp = func() (uint32, error) { return input.Load(), nil }
	m.SetOnSample(func(Sample) {
		select {
		case sampled <- struct{}{}:
		default:
		}
	})
	m.SetOnInputReset(func(reset InputReset) { resets <- reset })
	m.Start()
	defer m.Stop()

	select {
	case <-sampled:
	case <-time.After(time.Second):
		t.Fatal("initial sample did not fire")
	}
	input.Store(150)

	var reset InputReset
	select {
	case reset = <-resets:
	case <-time.After(time.Second):
		t.Fatal("input reset callback did not fire")
	}
	if reset.PreviousLastInputTick != 100 || reset.LastInputTick != 150 {
		t.Fatalf("reset ticks = %d -> %d, want 100 -> 150", reset.PreviousLastInputTick, reset.LastInputTick)
	}
	if reset.SessionIdleBeforeReset != 50*time.Millisecond {
		t.Fatalf("session idle before reset = %s, want 50ms", reset.SessionIdleBeforeReset)
	}
	if reset.Threshold != time.Hour || reset.WarnAt != 59*time.Minute+30*time.Second {
		t.Fatalf("threshold/warnAt = %s/%s, want 1h/59m30s", reset.Threshold, reset.WarnAt)
	}
}

func TestKeepaliveInputResetCanBeIgnored(t *testing.T) {
	resets := make(chan InputReset, 4)
	samples := make(chan Sample, 16)
	var input atomic.Uint32
	input.Store(1000)
	m := New(80*time.Millisecond, 0, nil, nil, time.Millisecond)
	m.SetEnhancedIdleMonitor(true)
	m.idleDuration = func() (time.Duration, error) { return 0, nil }
	m.inputTimestamp = func() (uint32, error) { return input.Load(), nil }
	m.SetOnInputReset(func(reset InputReset) { resets <- reset })
	m.SetOnSample(func(sample Sample) {
		select {
		case samples <- sample:
		default:
		}
	})
	m.Start()
	defer m.Stop()

	select {
	case <-samples:
	case <-time.After(time.Second):
		t.Fatal("initial sample did not fire")
	}

	var ignored InputReset
	for _, tick := range []uint32{51_000, 101_000, 151_000} {
		input.Store(tick)
		select {
		case ignored = <-resets:
		case <-time.After(time.Second):
			t.Fatalf("input reset did not fire for tick %d", tick)
		}
	}
	if ignored.Reason != "ignored_as_periodic_input" {
		t.Fatalf("last reset reason = %q, want ignored_as_periodic_input", ignored.Reason)
	}
	if !ignored.Ignored {
		t.Fatalf("ignored reset flag = false: %+v", ignored)
	}

	deadline := time.After(time.Second)
	for {
		select {
		case sample := <-samples:
			if sample.Idle > 0 {
				return
			}
		case <-deadline:
			t.Fatal("logical idle did not continue after ignored periodic input")
		}
	}
}
