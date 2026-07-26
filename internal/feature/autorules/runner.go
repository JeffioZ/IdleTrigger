// Package autorules evaluates built-in automatic tasks. It owns one shared,
// low-frequency process snapshot and one clock ticker; callbacks never mutate
// application state directly.
package autorules

import (
	"reflect"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/wallclock"
)

const (
	processScanInterval = 5 * time.Second
	processExitGrace    = 5 * time.Second
	scheduleDueGrace    = 2 * time.Minute
)

type EffectiveState struct {
	StayAwake      bool
	PauseStayAwake bool
	KeepScreenOn   bool
	EnableIdle     bool
	PauseIdle      bool
	IdleMinutes    int
	ActiveRuleIDs  []string
	NextRuleID     string
	NextOccurrence time.Time
}

type Event struct {
	RuleID         string
	RuleName       string
	Action         automation.Action
	WarningSeconds int
	Occurrence     string
}

type Callbacks struct {
	OnState      func(EffectiveState)
	OnEvent      func(Event)
	OnCheckpoint func(map[string]string)
	OnError      func(error)
}

type Runner struct {
	rules           []automation.Rule
	callbacks       Callbacks
	lastOccurrences map[string]string
	stopCh          chan struct{}
	doneCh          chan struct{}
	mu              sync.Mutex
	running         bool
	now             func() time.Time
}

func New(rules []automation.Rule, lastOccurrences map[string]string, callbacks Callbacks) *Runner {
	normalized, issues := automation.PrepareRules(rules)
	normalized = automation.RuntimeRules(normalized, issues)
	validIDs := make(map[string]struct{}, len(normalized))
	for _, rule := range normalized {
		validIDs[rule.ID] = struct{}{}
	}
	last := make(map[string]string, len(validIDs))
	for key, value := range lastOccurrences {
		if _, valid := validIDs[key]; valid {
			last[key] = value
		}
	}
	return &Runner{rules: normalized, callbacks: callbacks, lastOccurrences: last, now: time.Now}
}

func (r *Runner) Start() {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.running {
		return
	}
	if r.doneCh != nil {
		select {
		case <-r.doneCh:
		default:
			return
		}
	}
	r.stopCh = make(chan struct{})
	r.doneCh = make(chan struct{})
	r.running = true
	go r.loop(r.stopCh, r.doneCh)
}

func (r *Runner) Stop() {
	r.mu.Lock()
	done := r.doneCh
	if r.running {
		close(r.stopCh)
		r.running = false
	}
	r.mu.Unlock()
	if done != nil {
		<-done
	}
}

// Stopping is closed as soon as Stop begins. Callback adapters should select
// on it while enqueueing work so a saturated downstream queue cannot form a
// circular wait with Stop.
func (r *Runner) Stopping() <-chan struct{} {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.stopCh
}

func (r *Runner) isStopping() bool {
	stop := r.Stopping()
	if stop == nil {
		return false
	}
	select {
	case <-stop:
		return true
	default:
		return false
	}
}

type pendingEvent struct {
	rule       automation.Rule
	occurrence string
	expires    time.Time
}

type loopState struct {
	counts          map[string]int
	lastCounts      map[string]int
	absentSince     map[string]time.Time
	processKnown    bool // true only when the most recent process scan succeeded
	previousRunning map[string]bool
	pending         map[string]pendingEvent
	lastEffective   EffectiveState
	effectiveKnown  bool
	lastProcessScan time.Time
}

func (r *Runner) loop(stop <-chan struct{}, done chan<- struct{}) {
	defer close(done)
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	state := loopState{
		counts:          make(map[string]int),
		lastCounts:      make(map[string]int),
		absentSince:     make(map[string]time.Time),
		previousRunning: make(map[string]bool),
		pending:         make(map[string]pendingEvent),
	}
	r.step(r.now(), &state, true)
	for {
		select {
		case <-stop:
			return
		case now := <-ticker.C:
			r.step(now, &state, false)
		}
	}
}

func (r *Runner) step(now time.Time, state *loopState, forceScan bool) {
	if r.isStopping() {
		return
	}
	if forceScan || state.lastProcessScan.IsZero() || now.Sub(state.lastProcessScan) >= processScanInterval {
		if err := r.scanProcesses(now, state); err != nil && r.callbacks.OnError != nil {
			r.callbacks.OnError(err)
		}
		state.lastProcessScan = now
	}
	effective := r.evaluateState(now, state.counts, state.processKnown)
	if !state.effectiveKnown || !reflect.DeepEqual(effective, state.lastEffective) {
		state.lastEffective = effective
		state.effectiveKnown = true
		if r.callbacks.OnState != nil {
			r.callbacks.OnState(effective)
		}
	}
	if r.isStopping() {
		return
	}
	r.evaluateEvents(now, state)
}

func (r *Runner) scanProcesses(now time.Time, state *loopState) error {
	targets := collectTargets(r.rules)
	if len(targets) == 0 {
		state.counts = make(map[string]int)
		state.processKnown = true
		return nil
	}
	state.processKnown = false
	instances, err := processcatalog.SnapshotNames()
	if err != nil {
		return err
	}
	pathNames := make(map[string]struct{})
	for _, target := range targets {
		if target.Match == automation.MatchPath {
			pathNames[strings.ToLower(target.Executable)] = struct{}{}
		}
	}
	if len(pathNames) > 0 {
		instances = processcatalog.EnrichMatchingPaths(instances, pathNames)
	}
	raw := make(map[string]int, len(targets))
	for _, instance := range instances {
		for _, target := range targets {
			if !strings.EqualFold(instance.Executable, target.Executable) {
				continue
			}
			if target.Match == automation.MatchPath && (instance.Path == "" || !strings.EqualFold(instance.Path, target.Path)) {
				continue
			}
			raw[target.Key()]++
		}
	}
	state.counts = stabilizeCounts(now, targets, raw, state.absentSince, state.lastCounts)
	state.processKnown = true
	return nil
}

func stabilizeCounts(now time.Time, targets []automation.ProcessTarget, raw map[string]int, absentSince map[string]time.Time, lastCounts map[string]int) map[string]int {
	stable := make(map[string]int, len(targets))
	for _, target := range targets {
		key := target.Key()
		count := raw[key]
		if count > 0 {
			lastCounts[key] = count
			delete(absentSince, key)
			stable[key] = count
			continue
		}
		if lastCounts[key] <= 0 {
			continue
		}
		absent := absentSince[key]
		if absent.IsZero() {
			absentSince[key] = now
			stable[key] = lastCounts[key]
			continue
		}
		if now.Sub(absent) < processExitGrace {
			stable[key] = lastCounts[key]
			continue
		}
		delete(lastCounts, key)
		delete(absentSince, key)
	}
	return stable
}

func (r *Runner) evaluateState(now time.Time, counts map[string]int, processKnown bool) EffectiveState {
	result := EffectiveState{IdleMinutes: automation.DefaultIdleMinutes}
	idleMinutesSet := false
	for _, rule := range r.rules {
		if !rule.Enabled ||
			!automation.IsStateAction(rule.Action) ||
			!canEvaluateProcessCondition(rule, processKnown) ||
			!stateRuleActive(rule, now, counts) {
			continue
		}
		result.ActiveRuleIDs = append(result.ActiveRuleIDs, rule.ID)
		switch rule.Action {
		case automation.ActionStayAwake:
			result.StayAwake = true
			result.KeepScreenOn = result.KeepScreenOn || rule.KeepScreenOn
		case automation.ActionPauseStayAwake:
			result.PauseStayAwake = true
		case automation.ActionEnableIdle:
			result.EnableIdle = true
			if rule.IdleMinutes > 0 && (!idleMinutesSet || rule.IdleMinutes < result.IdleMinutes) {
				result.IdleMinutes = rule.IdleMinutes
				idleMinutesSet = true
			}
		case automation.ActionPauseIdle:
			result.PauseIdle = true
		}
	}
	sort.Strings(result.ActiveRuleIDs)
	result.NextRuleID, result.NextOccurrence = nextScheduled(r.rules, now)
	return result
}

func (r *Runner) evaluateEvents(now time.Time, state *loopState) {
	for key, pending := range state.pending {
		if r.isStopping() {
			return
		}
		if !pending.expires.IsZero() && !now.Before(pending.expires) {
			delete(state.pending, key)
			r.markOccurrence(pending.rule.ID, pending.occurrence)
			continue
		}
		if canEvaluateProcessCondition(pending.rule, state.processKnown) && processCondition(pending.rule, state.counts) {
			delete(state.pending, key)
			r.fire(pending.rule, pending.occurrence)
		}
	}
	for _, rule := range r.rules {
		if r.isStopping() {
			return
		}
		if !rule.Enabled || !automation.IsEventAction(rule.Action) {
			continue
		}
		if rule.Trigger == automation.TriggerProcessStarted || rule.Trigger == automation.TriggerProcessExited {
			// A failed initial process scan must not establish an artificial
			// "not running" baseline. The first successful snapshot only seeds
			// previousRunning; transitions begin with later snapshots.
			if !state.processKnown {
				continue
			}
			any := anyTargetRunning(rule.Processes, state.counts)
			previous, known := state.previousRunning[rule.ID]
			state.previousRunning[rule.ID] = any
			if known {
				if rule.Trigger == automation.TriggerProcessStarted && !previous && any {
					r.fire(rule, "process-started:"+now.Format(time.RFC3339))
				}
				if rule.Trigger == automation.TriggerProcessExited && previous && !any {
					r.fire(rule, "process-exited:"+now.Format(time.RFC3339))
				}
			}
			continue
		}
		if _, waiting := state.pending[rule.ID]; waiting {
			continue
		}
		occurrence, due := scheduledOccurrence(rule, now)
		if !due || r.lastOccurrences[rule.ID] == occurrence {
			continue
		}
		if canEvaluateProcessCondition(rule, state.processKnown) && processCondition(rule, state.counts) {
			r.fire(rule, occurrence)
			continue
		}
		if rule.BlockedPolicy == automation.BlockedWait && rule.MaxWaitMinutes > 0 {
			state.pending[rule.ID] = pendingEvent{rule: rule, occurrence: occurrence, expires: now.Add(time.Duration(rule.MaxWaitMinutes) * time.Minute)}
			continue
		}
		r.markOccurrence(rule.ID, occurrence)
	}
}

func (r *Runner) fire(rule automation.Rule, occurrence string) {
	r.markOccurrence(rule.ID, occurrence)
	if r.callbacks.OnEvent != nil {
		r.callbacks.OnEvent(Event{RuleID: rule.ID, RuleName: rule.Name, Action: rule.Action, WarningSeconds: rule.WarningSeconds, Occurrence: occurrence})
	}
}

func (r *Runner) markOccurrence(ruleID, occurrence string) {
	r.lastOccurrences[ruleID] = occurrence
	if r.callbacks.OnCheckpoint != nil {
		copyState := make(map[string]string, len(r.lastOccurrences))
		for key, value := range r.lastOccurrences {
			copyState[key] = value
		}
		r.callbacks.OnCheckpoint(copyState)
	}
}

func collectTargets(rules []automation.Rule) []automation.ProcessTarget {
	seen := make(map[string]struct{})
	var out []automation.ProcessTarget
	for _, rule := range rules {
		if !rule.Enabled {
			continue
		}
		for _, target := range rule.Processes {
			if _, exists := seen[target.Key()]; exists {
				continue
			}
			seen[target.Key()] = struct{}{}
			out = append(out, target)
		}
	}
	return out
}

func stateRuleActive(rule automation.Rule, now time.Time, counts map[string]int) bool {
	var active bool
	switch rule.Trigger {
	case automation.TriggerProcessRunning:
		active = processCondition(rule, counts)
	case automation.TriggerTimeWindow:
		active = withinWindow(rule, now)
	default:
		return false
	}
	if active && rule.Trigger == automation.TriggerTimeWindow && len(rule.Processes) > 0 {
		active = processCondition(rule, counts)
	}
	return active
}

func processCondition(rule automation.Rule, counts map[string]int) bool {
	if len(rule.Processes) == 0 {
		return true
	}
	switch rule.ProcessLogic {
	case automation.ProcessAll:
		for _, target := range rule.Processes {
			if counts[target.Key()] <= 0 {
				return false
			}
		}
		return true
	case automation.ProcessNone:
		return !anyTargetRunning(rule.Processes, counts)
	default:
		return anyTargetRunning(rule.Processes, counts)
	}
}

func canEvaluateProcessCondition(rule automation.Rule, processKnown bool) bool {
	return len(rule.Processes) == 0 || processKnown
}

func anyTargetRunning(targets []automation.ProcessTarget, counts map[string]int) bool {
	for _, target := range targets {
		if counts[target.Key()] > 0 {
			return true
		}
	}
	return false
}

func withinWindow(rule automation.Rule, now time.Time) bool {
	start, errStart := time.Parse("15:04", rule.Time)
	end, errEnd := time.Parse("15:04", rule.EndTime)
	if errStart != nil || errEnd != nil {
		return false
	}
	minute := now.Hour()*60 + now.Minute()
	startMinute := start.Hour()*60 + start.Minute()
	endMinute := end.Hour()*60 + end.Minute()
	if startMinute == endMinute {
		return len(rule.Days) == 0 || containsDay(rule.Days, automation.WeekdayKey(now.Weekday()))
	}
	if startMinute < endMinute {
		return minute >= startMinute && minute < endMinute && (len(rule.Days) == 0 || containsDay(rule.Days, automation.WeekdayKey(now.Weekday())))
	}
	if minute >= startMinute {
		return len(rule.Days) == 0 || containsDay(rule.Days, automation.WeekdayKey(now.Weekday()))
	}
	if minute < endMinute {
		return len(rule.Days) == 0 || containsDay(rule.Days, automation.WeekdayKey(now.AddDate(0, 0, -1).Weekday()))
	}
	return false
}

func scheduledOccurrence(rule automation.Rule, now time.Time) (string, bool) {
	if rule.Trigger != automation.TriggerOnce && rule.Trigger != automation.TriggerDaily && rule.Trigger != automation.TriggerWeekly {
		return "", false
	}
	parsed, err := time.Parse("15:04", rule.Time)
	if err != nil {
		return "", false
	}
	date := now.Format("2006-01-02")
	if rule.Trigger == automation.TriggerOnce && rule.Date != date {
		return "", false
	}
	if rule.Trigger == automation.TriggerWeekly && !containsDay(rule.Days, automation.WeekdayKey(now.Weekday())) {
		return "", false
	}
	candidate := wallclock.At(now, parsed.Hour()*60+parsed.Minute())
	return date + "T" + rule.Time, !now.Before(candidate) && now.Before(candidate.Add(scheduleDueGrace))
}

func nextScheduled(rules []automation.Rule, now time.Time) (string, time.Time) {
	var id string
	var next time.Time
	for _, rule := range rules {
		if !rule.Enabled || !automation.IsEventAction(rule.Action) || rule.Trigger == automation.TriggerProcessStarted || rule.Trigger == automation.TriggerProcessExited {
			continue
		}
		parsed, err := time.Parse("15:04", rule.Time)
		if err != nil {
			continue
		}
		if rule.Trigger == automation.TriggerOnce {
			date, err := time.ParseInLocation("2006-01-02", rule.Date, now.Location())
			if err != nil {
				continue
			}
			candidate := wallclock.At(date, parsed.Hour()*60+parsed.Minute())
			if candidate.After(now) && (next.IsZero() || candidate.Before(next)) {
				next, id = candidate, rule.ID
			}
			continue
		}
		for offset := 0; offset <= 7; offset++ {
			day := now.AddDate(0, 0, offset)
			candidate := wallclock.At(day, parsed.Hour()*60+parsed.Minute())
			if !candidate.After(now) {
				continue
			}
			if rule.Trigger == automation.TriggerWeekly && !containsDay(rule.Days, automation.WeekdayKey(day.Weekday())) {
				continue
			}
			if next.IsZero() || candidate.Before(next) {
				next, id = candidate, rule.ID
			}
			break
		}
	}
	return id, next
}

func containsDay(days []string, target string) bool {
	for _, day := range days {
		if strings.EqualFold(day, target) {
			return true
		}
	}
	return false
}
