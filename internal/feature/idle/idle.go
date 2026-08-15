// Package idle tracks user idle time via the Win32 GetLastInputInfo API
// and fires callbacks when the configured threshold is approached and exceeded.
package idle

import (
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

var user32 = windows.NewLazySystemDLL("user32.dll")
var kernel32dll = windows.NewLazySystemDLL("kernel32.dll")
var getLastInputInfoProc = user32.NewProc("GetLastInputInfo")
var getTickCountProc = kernel32dll.NewProc("GetTickCount")
var getTickCount64Proc = kernel32dll.NewProc("GetTickCount64")

type lastInputInfo struct {
	cbSize uint32
	dwTime uint32
}

func getLastInputTime() (uint32, error) {
	var lii lastInputInfo
	lii.cbSize = uint32(unsafe.Sizeof(lii))
	r, _, err := getLastInputInfoProc.Call(uintptr(unsafe.Pointer(&lii)))
	if r == 0 {
		return 0, err
	}
	return lii.dwTime, nil
}

func getTickCount() uint32 {
	r, _, _ := getTickCountProc.Call()
	return uint32(r)
}

func getTickCount64() uint64 {
	r, _, _ := getTickCount64Proc.Call()
	return uint64(r)
}

// IdleSnapshot contains the raw Windows idle-time inputs and IdleTrigger's
// converted value. It is intended for diagnostics and tests.
type IdleSnapshot struct {
	NowTick64     uint64
	NowTick32     uint32
	LastInputTick uint32
	RawDeltaMS    int32
	Idle          time.Duration
}

// Snapshot returns the raw GetLastInputInfo/GetTickCount values used to derive
// the current session's idle duration.
func Snapshot() (IdleSnapshot, error) {
	last, err := getLastInputTime()
	if err != nil {
		return IdleSnapshot{}, err
	}
	now64 := getTickCount64()
	now32 := uint32(now64)
	rawDelta := int32(now32 - last)
	return IdleSnapshot{
		NowTick64:     now64,
		NowTick32:     now32,
		LastInputTick: last,
		RawDeltaMS:    rawDelta,
		Idle:          elapsedSinceLastInput(now32, last),
	}, nil
}

// IdleDuration returns how long the user has been idle.
func IdleDuration() (time.Duration, error) {
	last, err := getLastInputTime()
	if err != nil {
		return 0, err
	}
	return elapsedSinceLastInput(getTickCount(), last), nil
}

func elapsedSinceLastInput(now, last uint32) time.Duration {
	// GetLastInputInfo's timestamp may move forward unexpectedly when another
	// process injects input. Treat a negative signed delta as fresh activity;
	// IdleTrigger's maximum configured timeout is far below the 24.8-day signed
	// tick interval, so this cannot mask a configured timeout.
	delta := int32(now - last)
	if delta < 0 {
		return 0
	}
	return time.Duration(delta) * time.Millisecond
}

// Monitor periodically checks idle time and fires:
//   - onWarning: when idle reaches (threshold − warningOffset)
//   - onTrigger: when idle reaches threshold
//
// Both fire at most once per idle session and reset when activity resumes.
type Monitor struct {
	thresholdNs    atomic.Int64 // nanoseconds
	ignorePeriodic atomic.Bool
	warningOffset  time.Duration
	onWarning      func()
	onTrigger      func()
	onActivity     func()
	onSample       func(Sample)
	onInputReset   func(InputReset)
	pollInterval   time.Duration
	idleDuration   func() (time.Duration, error)
	inputTimestamp func() (uint32, error)
	startedAt      time.Time
	lastInputTick  uint32
	seenInputTick  bool
	periodic       periodicInputState

	warned    atomic.Bool
	triggered atomic.Bool
	stopCh    chan struct{}
	doneCh    chan struct{}
	running   bool
	mu        sync.Mutex
}

// Sample describes one successful idle-monitor poll after startup-window
// clamping has been applied.
type Sample struct {
	Idle                    time.Duration
	Threshold               time.Duration
	WarnAt                  time.Duration
	InputTimestampAvailable bool
	LastInputTick           uint32
	StartWindowClamped      bool
	Warned                  bool
	Triggered               bool
}

// InputReset describes a GetLastInputInfo timestamp change that starts a new
// idle session.
type InputReset struct {
	PreviousLastInputTick  uint32
	LastInputTick          uint32
	SessionIdleBeforeReset time.Duration
	Threshold              time.Duration
	WarnAt                 time.Duration
	WasWarned              bool
	WasTriggered           bool
	Ignored                bool
	Reason                 string
	PeriodicCount          int
	PeriodicBaseline       time.Duration
}

type periodicInputState struct {
	count          int
	baseline       time.Duration
	useLogicalIdle bool
}

const (
	periodicInputMinInterval = 20 * time.Second
	periodicInputMaxInterval = 2 * time.Minute
	periodicInputTolerance   = 5 * time.Second
	periodicInputRequired    = 3
)

// New creates a Monitor.
//   - threshold: idle duration after which onTrigger fires
//   - warningOffset: how long before threshold to fire onWarning (0 = no warning)
//   - onWarning: called when approaching the threshold (may be nil)
//   - onTrigger: called when threshold is reached
//   - pollInterval: check frequency (default 3 s if ≤ 0)
func New(
	threshold, warningOffset time.Duration,
	onWarning, onTrigger func(),
	pollInterval time.Duration,
) *Monitor {
	if warningOffset < 0 {
		warningOffset = 0
	}
	m := &Monitor{
		warningOffset:  warningOffset,
		onWarning:      onWarning,
		onTrigger:      onTrigger,
		pollInterval:   pollInterval,
		idleDuration:   IdleDuration,
		inputTimestamp: getLastInputTime,
	}
	m.thresholdNs.Store(int64(threshold))
	if pollInterval <= 0 {
		m.pollInterval = 3 * time.Second
	}
	return m
}

// Start begins monitoring in a background goroutine.
func (m *Monitor) Start() {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.running {
		return
	}
	m.stopCh = make(chan struct{})
	m.doneCh = make(chan struct{})
	m.startedAt = time.Now()
	m.seenInputTick = false
	m.periodic = periodicInputState{}
	m.running = true
	go m.loop(m.stopCh, m.doneCh)
}

// Stop signals the monitor to exit.
func (m *Monitor) Stop() {
	m.mu.Lock()
	if !m.running {
		m.mu.Unlock()
		return
	}
	stopCh := m.stopCh
	doneCh := m.doneCh
	m.running = false
	close(stopCh)
	m.mu.Unlock()
	<-doneCh
}

// Threshold returns the current idle threshold.
func (m *Monitor) Threshold() time.Duration { return time.Duration(m.thresholdNs.Load()) }

// SetEnhancedIdleMonitor enables a compatibility mode that keeps logical idle
// time moving across stable, low-frequency input timestamp refreshes.
func (m *Monitor) SetEnhancedIdleMonitor(enabled bool) { m.ignorePeriodic.Store(enabled) }

// SetOnActivity sets a callback for the first user activity after a warning.
// It must be configured before Start.
func (m *Monitor) SetOnActivity(fn func()) { m.onActivity = fn }

// SetOnSample sets a callback for successful idle polls. It must be configured
// before Start.
func (m *Monitor) SetOnSample(fn func(Sample)) { m.onSample = fn }

// SetOnInputReset sets a callback for input timestamp changes. It must be
// configured before Start.
func (m *Monitor) SetOnInputReset(fn func(InputReset)) { m.onInputReset = fn }

func (m *Monitor) loop(stopCh <-chan struct{}, doneCh chan<- struct{}) {
	ticker := time.NewTicker(m.pollInterval)
	defer ticker.Stop()
	defer close(doneCh)

	for {
		select {
		case <-stopCh:
			return
		case <-ticker.C:
			inputTimestampAvailable, sessionReset := m.observeInputTimestamp()
			if sessionReset {
				continue
			}
			sample, err := m.sample(inputTimestampAvailable)
			if err != nil {
				continue
			}
			if m.onSample != nil {
				m.onSample(sample)
			}
			m.evaluateSample(sample)
		}
	}
}

func (m *Monitor) resetSession() {
	m.startedAt = time.Now()
	m.warned.Store(false)
	m.triggered.Store(false)
	m.periodic.useLogicalIdle = false
}

func (m *Monitor) classifyInputReset(interval time.Duration) (bool, string) {
	if !m.ignorePeriodic.Load() {
		m.periodic = periodicInputState{}
		return false, "accepted_standard_input"
	}
	if interval < periodicInputMinInterval || interval > periodicInputMaxInterval {
		m.periodic.count = 0
		m.periodic.baseline = 0
		return false, "accepted_outside_periodic_window"
	}
	if m.periodic.count == 0 || m.periodic.baseline <= 0 {
		m.periodic.count = 1
		m.periodic.baseline = interval
		return false, "accepted_collecting_periodic_pattern"
	}
	if !durationClose(interval, m.periodic.baseline, periodicInputTolerance) {
		m.periodic.count = 1
		m.periodic.baseline = interval
		return false, "accepted_periodic_pattern_changed"
	}
	m.periodic.baseline = rollingAverageDuration(m.periodic.baseline, m.periodic.count, interval)
	m.periodic.count++
	if m.periodic.count < periodicInputRequired {
		return false, "accepted_collecting_periodic_pattern"
	}
	return true, "ignored_as_periodic_input"
}

func durationClose(a, b, tolerance time.Duration) bool {
	if a > b {
		return a-b <= tolerance
	}
	return b-a <= tolerance
}

func rollingAverageDuration(current time.Duration, count int, next time.Duration) time.Duration {
	if count <= 0 {
		return next
	}
	return time.Duration((int64(current)*int64(count) + int64(next)) / int64(count+1))
}
