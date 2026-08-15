package app

import (
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/displaytopology"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

const (
	themeAutomaticGrace       = 2500 * time.Millisecond
	themeTopologyProbe        = 450 * time.Millisecond
	themeTopologySamples      = 3
	themeTransitionMaxWait    = 8 * time.Second
	themePostSwitchDelay      = 1200 * time.Millisecond
	themeFullRepairRateLimit  = 30 * time.Second
	themeRecentTransitionSpan = 15 * time.Second
)

type themeTransitionSource string

const (
	themeSourceSchedule     themeTransitionSource = "schedule"
	themeSourceBattery      themeTransitionSource = "battery"
	themeSourceResume       themeTransitionSource = "resume"
	themeSourceDisplay      themeTransitionSource = "display"
	themeSourceManualSwitch themeTransitionSource = "manual-switch"
	themeSourceManualRepair themeTransitionSource = "manual-repair"
)

type themeTransitionRequest struct {
	source          themeTransitionSource
	target          theme.Mode
	hasTarget       bool
	manual          bool
	fullRepair      bool
	displayChanged  bool
	resumed         bool
	recoveryAttempt bool
	actionKey       string
	submission      uint64
	firstAt         time.Time
	dueAt           time.Time
	generation      uint64
}

type themeCoordinatorConfig struct {
	automaticGrace      time.Duration
	topologyProbe       time.Duration
	topologySamples     int
	maxWait             time.Duration
	postSwitchDelay     time.Duration
	fullRepairRateLimit time.Duration
	recentTransition    time.Duration
}

func defaultThemeCoordinatorConfig() themeCoordinatorConfig {
	return themeCoordinatorConfig{
		automaticGrace:      themeAutomaticGrace,
		topologyProbe:       themeTopologyProbe,
		topologySamples:     themeTopologySamples,
		maxWait:             themeTransitionMaxWait,
		postSwitchDelay:     themePostSwitchDelay,
		fullRepairRateLimit: themeFullRepairRateLimit,
		recentTransition:    themeRecentTransitionSpan,
	}
}

type themeCoordinatorDeps struct {
	now                 func() time.Time
	queryTopology       func() (displaytopology.Snapshot, error)
	currentTheme        func() theme.Mode
	switchTheme         func(theme.Mode) error
	rebroadcastTheme    func()
	fullRepairTheme     func() error
	fullRepairAvailable func() bool
	onComplete          func(themeTransitionResult)
}

type themeCoordinatorCommand struct {
	request         *themeTransitionRequest
	cancelAutomatic bool
}

type themeTopologyResult struct {
	generation uint64
	snapshot   displaytopology.Snapshot
	stable     bool
	err        error
	waited     time.Duration
}

type themeTopologyWaitState struct {
	generation uint64
	cancel     chan struct{}
}

func (p *themeTopologyWaitState) begin(generation uint64) <-chan struct{} {
	p.cancelWait()
	p.generation = generation
	p.cancel = make(chan struct{})
	return p.cancel
}

func (p *themeTopologyWaitState) cancelWait() {
	if p.cancel != nil {
		close(p.cancel)
	}
	p.generation = 0
	p.cancel = nil
}

func (p *themeTopologyWaitState) finish(generation uint64) bool {
	if p.cancel == nil || p.generation != generation {
		return false
	}
	p.generation = 0
	p.cancel = nil
	return true
}

func (p *themeTopologyWaitState) active() bool { return p.cancel != nil }

type themeTransitionResult struct {
	request         themeTransitionRequest
	err             error
	switched        bool
	rebroadcast     bool
	fullRepairTried bool
	retryWhenStable bool
	topologyStable  bool
	displayCount    int
	topologyWait    time.Duration
}

type themeTransitionCoordinator struct {
	config         themeCoordinatorConfig
	deps           themeCoordinatorDeps
	commands       chan themeCoordinatorCommand
	topology       chan themeTopologyResult
	completed      chan themeTransitionResult
	stopCh         chan struct{}
	doneCh         chan struct{}
	stopOnce       sync.Once
	requestVersion atomic.Uint64
}

func newThemeTransitionCoordinator(config themeCoordinatorConfig, deps themeCoordinatorDeps) *themeTransitionCoordinator {
	if deps.now == nil {
		deps.now = time.Now
	}
	coordinator := &themeTransitionCoordinator{
		config:    config,
		deps:      deps,
		commands:  make(chan themeCoordinatorCommand, 32),
		topology:  make(chan themeTopologyResult, 1),
		completed: make(chan themeTransitionResult, 1),
		stopCh:    make(chan struct{}),
		doneCh:    make(chan struct{}),
	}
	go coordinator.loop()
	return coordinator
}

func (c *themeTransitionCoordinator) submit(request themeTransitionRequest) bool {
	if c == nil {
		return false
	}
	request.submission = c.requestVersion.Add(1)
	select {
	case c.commands <- themeCoordinatorCommand{request: &request}:
		return true
	case <-c.doneCh:
		return false
	}
}

func (c *themeTransitionCoordinator) cancelAutomatic() {
	if c == nil {
		return
	}
	c.requestVersion.Add(1)
	select {
	case c.commands <- themeCoordinatorCommand{cancelAutomatic: true}:
	case <-c.doneCh:
	}
}

func (c *themeTransitionCoordinator) stop() {
	if c == nil {
		return
	}
	c.stopOnce.Do(func() { close(c.stopCh) })
	<-c.doneCh
}

func (c *themeTransitionCoordinator) loop() {
	defer close(c.doneCh)
	var (
		pending                 *themeTransitionRequest
		dueTimer                *time.Timer
		dueCh                   <-chan time.Time
		stability               themeTopologyWaitState
		generation              uint64
		busy                    bool
		automaticEnabled        = true
		lastAutomaticTransition time.Time
		lastFullRepair          time.Time
	)

	stopDueTimer := func() {
		if dueTimer != nil && !dueTimer.Stop() {
			select {
			case <-dueTimer.C:
			default:
			}
		}
		dueTimer = nil
		dueCh = nil
	}
	cancelStability := func() {
		stability.cancelWait()
	}
	armPending := func() {
		stopDueTimer()
		if pending == nil || busy || stability.active() {
			return
		}
		delay := pending.dueAt.Sub(c.deps.now())
		if delay < 0 {
			delay = 0
		}
		dueTimer = time.NewTimer(delay)
		dueCh = dueTimer.C
	}

	for {
		select {
		case command := <-c.commands:
			if command.cancelAutomatic {
				automaticEnabled = false
				if pending != nil && !pending.manual {
					pending = nil
					generation++
					cancelStability()
					stopDueTimer()
				}
				continue
			}
			if command.request == nil {
				continue
			}
			if !command.request.manual {
				automaticEnabled = true
			}
			now := c.deps.now()
			merged := mergeThemeTransitionRequests(pending, *command.request, now, c.config)
			generation++
			merged.generation = generation
			pending = &merged
			cancelStability()
			armPending()

		case <-dueCh:
			stopDueTimer()
			if pending == nil || busy {
				continue
			}
			if pending.manual {
				request := *pending
				pending = nil
				busy = true
				go c.execute(request, themeTopologyResult{generation: request.generation, stable: true}, lastAutomaticTransition, lastFullRepair)
				continue
			}
			request := *pending
			cancel := stability.begin(request.generation)
			deadline := request.firstAt.Add(c.config.maxWait)
			go func() {
				result := waitForStableThemeTopology(request.generation, deadline, cancel, c.config, c.deps.now, c.deps.queryTopology)
				select {
				case c.topology <- result:
				case <-c.stopCh:
				}
			}()

		case topology := <-c.topology:
			if !stability.finish(topology.generation) {
				// A canceled probe can finish after its replacement has begun.
				// Never let that stale result clear the replacement's cancel
				// handle or arm a duplicate timer for the same request.
				continue
			}
			if pending == nil || busy || topology.generation != pending.generation {
				armPending()
				continue
			}
			request := *pending
			pending = nil
			busy = true
			go c.execute(request, topology, lastAutomaticTransition, lastFullRepair)

		case result := <-c.completed:
			busy = false
			if result.switched && !result.request.manual {
				lastAutomaticTransition = c.deps.now()
			}
			if result.fullRepairTried {
				lastFullRepair = c.deps.now()
			}
			if c.deps.onComplete != nil {
				c.deps.onComplete(result)
			}
			if automaticEnabled && result.retryWhenStable && !result.request.recoveryAttempt && (pending == nil || !pending.manual) {
				retry := themeTransitionRequest{
					source:          themeSourceDisplay,
					displayChanged:  true,
					recoveryAttempt: true,
					submission:      c.requestVersion.Load(),
				}
				merged := mergeThemeTransitionRequests(pending, retry, c.deps.now(), c.config)
				generation++
				merged.generation = generation
				pending = &merged
			}
			armPending()

		case <-c.stopCh:
			cancelStability()
			stopDueTimer()
			if busy {
				result := <-c.completed
				if c.deps.onComplete != nil {
					c.deps.onComplete(result)
				}
			}
			return
		}
	}
}

func mergeThemeTransitionRequests(existing *themeTransitionRequest, incoming themeTransitionRequest, now time.Time, config themeCoordinatorConfig) themeTransitionRequest {
	incoming.firstAt = now
	incoming.dueAt = now
	if !incoming.manual {
		incoming.dueAt = now.Add(config.automaticGrace)
	}
	if existing == nil {
		return capThemeRequestDue(incoming, config)
	}

	if incoming.manual {
		if !existing.manual {
			return incoming
		}
		merged := *existing
		if incoming.hasTarget {
			merged.target = incoming.target
			merged.hasTarget = true
		}
		merged.fullRepair = merged.fullRepair || incoming.fullRepair
		merged.submission = incoming.submission
		if incoming.fullRepair {
			merged.source = incoming.source
			merged.actionKey = incoming.actionKey
		}
		merged.dueAt = now
		return merged
	}
	if existing.manual {
		return *existing
	}

	merged := *existing
	if incoming.hasTarget {
		merged.target = incoming.target
		merged.hasTarget = true
		merged.source = incoming.source
	}
	if !merged.hasTarget {
		merged.source = incoming.source
	}
	merged.displayChanged = merged.displayChanged || incoming.displayChanged
	merged.resumed = merged.resumed || incoming.resumed
	merged.recoveryAttempt = merged.recoveryAttempt || incoming.recoveryAttempt
	merged.submission = incoming.submission
	merged.dueAt = now.Add(config.automaticGrace)
	return capThemeRequestDue(merged, config)
}

func capThemeRequestDue(request themeTransitionRequest, config themeCoordinatorConfig) themeTransitionRequest {
	if request.manual {
		return request
	}
	deadline := request.firstAt.Add(config.maxWait)
	if request.dueAt.After(deadline) {
		request.dueAt = deadline
	}
	return request
}

func waitForStableThemeTopology(generation uint64, deadline time.Time, cancel <-chan struct{}, config themeCoordinatorConfig,
	now func() time.Time, query func() (displaytopology.Snapshot, error)) themeTopologyResult {
	started := now()
	result := themeTopologyResult{generation: generation}
	var previous displaytopology.Snapshot
	havePrevious := false
	identical := 0
	for {
		snapshot, err := query()
		if err != nil {
			result.err = err
			havePrevious = false
			identical = 0
		} else {
			result.snapshot = snapshot
			result.err = nil
			if havePrevious && snapshot.Equal(previous) {
				identical++
			} else {
				identical = 1
			}
			previous = snapshot
			havePrevious = true
			if identical >= config.topologySamples {
				result.stable = true
				result.waited = now().Sub(started)
				return result
			}
		}

		remaining := deadline.Sub(now())
		if remaining <= 0 {
			result.waited = now().Sub(started)
			return result
		}
		wait := config.topologyProbe
		if wait > remaining {
			wait = remaining
		}
		timer := time.NewTimer(wait)
		select {
		case <-timer.C:
		case <-cancel:
			if !timer.Stop() {
				<-timer.C
			}
			result.err = errors.New("display topology wait canceled")
			result.waited = now().Sub(started)
			return result
		}
	}
}

func (c *themeTransitionCoordinator) execute(request themeTransitionRequest, topology themeTopologyResult,
	lastAutomaticTransition, lastFullRepair time.Time) {
	result := themeTransitionResult{
		request:        request,
		topologyStable: topology.stable,
		displayCount:   topology.snapshot.ActiveDisplayCount(),
		topologyWait:   topology.waited,
	}

	if request.hasTarget && c.deps.currentTheme() != request.target {
		if err := c.deps.switchTheme(request.target); err != nil {
			result.err = fmt.Errorf("switch theme: %w", err)
			c.completed <- result
			return
		}
		result.switched = true
	}

	if request.manual {
		if request.fullRepair {
			result.fullRepairTried = true
			result.err = c.deps.fullRepairTheme()
		}
		c.completed <- result
		return
	}

	needsRecovery := result.switched || request.displayChanged || request.resumed
	if !needsRecovery {
		c.completed <- result
		return
	}
	if c.config.postSwitchDelay > 0 {
		time.Sleep(c.config.postSwitchDelay)
	}
	if c.requestVersion.Load() != request.submission {
		// A newer power/display/manual request arrived after this operation
		// began. Never start the expensive DWM repair against a topology or
		// target that may already be stale; finish the lightweight pass and let
		// the coalesced request recover after its own stability check.
		c.deps.rebroadcastTheme()
		result.rebroadcast = true
		result.retryWhenStable = true
		result.topologyStable = false
		c.completed <- result
		return
	}

	if !topology.stable {
		c.deps.rebroadcastTheme()
		result.rebroadcast = true
		result.retryWhenStable = true
		if topology.err != nil {
			result.err = fmt.Errorf("display topology did not stabilize: %w", topology.err)
		}
		c.completed <- result
		return
	}

	recentTransition := !lastAutomaticTransition.IsZero() && c.deps.now().Sub(lastAutomaticTransition) <= c.config.recentTransition
	highRisk := (result.switched && (result.displayCount >= 2 || request.displayChanged || request.resumed)) ||
		(!result.switched && request.displayChanged && recentTransition) ||
		(!result.switched && request.resumed && result.displayCount >= 2)
	repairAllowed := lastFullRepair.IsZero() || c.deps.now().Sub(lastFullRepair) >= c.config.fullRepairRateLimit
	if highRisk && repairAllowed && c.deps.fullRepairAvailable() {
		result.fullRepairTried = true
		if err := c.deps.fullRepairTheme(); err != nil {
			result.err = fmt.Errorf("repair Windows theme surfaces: %w", err)
			result.retryWhenStable = true
		}
	} else {
		c.deps.rebroadcastTheme()
		result.rebroadcast = true
	}
	c.completed <- result
}

func (s *runtimeState) startThemeCoordinator() {
	if s.themeCoordinator != nil {
		return
	}
	s.themeCoordinator = newThemeTransitionCoordinator(defaultThemeCoordinatorConfig(), themeCoordinatorDeps{
		now:                 time.Now,
		queryTopology:       displaytopology.Query,
		currentTheme:        theme.Current,
		switchTheme:         theme.Switch,
		rebroadcastTheme:    theme.Rebroadcast,
		fullRepairTheme:     theme.Refresh,
		fullRepairAvailable: theme.FullDWMRefreshAvailable,
		onComplete:          s.onThemeTransitionComplete,
	})
}

func (s *runtimeState) stopThemeCoordinator() {
	if s.themeCoordinator == nil {
		return
	}
	s.themeCoordinator.stop()
	s.themeCoordinator = nil
}

func (s *runtimeState) requestAutomaticThemeSwitch(source string, target theme.Mode) error {
	if s.exiting.Load() || s.themeCoordinator == nil {
		return nil
	}
	requestSource := themeSourceSchedule
	if source == string(themeSourceBattery) {
		requestSource = themeSourceBattery
	}
	s.themeCoordinator.submit(themeTransitionRequest{source: requestSource, target: target, hasTarget: true})
	return nil
}

func (s *runtimeState) requestThemeEnvironmentRecovery(source themeTransitionSource) {
	if !s.cfg.ThemeSwitchEnabled || !s.themeAvailable() || s.themeCoordinator == nil {
		return
	}
	request := themeTransitionRequest{source: source}
	request.displayChanged = source == themeSourceDisplay
	request.resumed = source == themeSourceResume
	s.themeCoordinator.submit(request)
}

func (s *runtimeState) requestManualThemeSwitch(target theme.Mode) {
	if s.themeCoordinator == nil || s.themeOperationBusy {
		return
	}
	if s.themeCoordinator.submit(themeTransitionRequest{
		source: themeSourceManualSwitch, target: target, hasTarget: true, manual: true,
		actionKey: "menu_theme_switch_now",
	}) {
		s.setThemeOperationBusy(true)
	}
}

func (s *runtimeState) requestManualThemeRepair() {
	if s.themeCoordinator == nil || s.themeOperationBusy {
		return
	}
	if s.themeCoordinator.submit(themeTransitionRequest{
		source: themeSourceManualRepair, manual: true, fullRepair: true,
		actionKey: "menu_theme_repair",
	}) {
		s.setThemeOperationBusy(true)
	}
}

func (s *runtimeState) setThemeOperationBusy(busy bool) {
	if s.themeOperationBusy == busy {
		return
	}
	s.themeOperationBusy = busy
	trayicon.Post(func() { controlpanel.UpdateThemeOperationBusy(busy) })
}

func (s *runtimeState) onThemeTransitionComplete(result themeTransitionResult) {
	if s.exiting.Load() {
		return
	}
	s.post(func() {
		if result.request.manual {
			s.setThemeOperationBusy(false)
		}
		target := "current"
		if result.request.hasTarget {
			target = "light"
			if result.request.target == theme.ModeDark {
				target = "dark"
			}
		}
		mylog.Info("Theme transition completed: source=%s target=%s switched=%v topology_stable=%v displays=%d waited=%s rebroadcast=%v full_repair=%v error=%v",
			result.request.source, target, result.switched, result.topologyStable, result.displayCount,
			result.topologyWait.Round(time.Millisecond), result.rebroadcast, result.fullRepairTried, result.err)
		if result.err != nil && result.request.manual {
			s.showError(result.request.actionKey, result.err)
		}
		s.refreshTrayThemeIcon()
		if result.fullRepairTried {
			trayicon.Post(controlpanel.RefreshThemeAfterSystemRepair)
		} else if result.switched || result.rebroadcast {
			trayicon.Post(controlpanel.RefreshTheme)
		}
	})
}
