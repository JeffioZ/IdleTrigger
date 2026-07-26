// Package theme auto-switches Windows between light and dark themes
// at scheduled times or sunrise/sunset via registry + WM_SETTINGCHANGE.
package theme

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/wallclock"
)

type ScheduleInfo struct {
	LightTime     string
	DarkTime      string
	OK            bool
	FixedFallback bool
}

// ScheduleTimes returns today's light and dark switch times as HH:MM.
func ScheduleTimes(mode, lightTime, darkTime string, lat, lon float64, now time.Time) (string, string, bool) {
	info := ScheduleInfoFor(mode, lightTime, darkTime, lat, lon, now)
	return info.LightTime, info.DarkTime, info.OK
}

// ScheduleInfoFor returns the theme schedule and reports when sunrise/sunset
// mode had to fall back to fixed times, such as during polar day/night.
func ScheduleInfoFor(mode, lightTime, darkTime string, lat, lon float64, now time.Time) ScheduleInfo {
	var lightMin, darkMin int
	if mode == "sunrise" {
		lightMin, darkMin = CalcSunriseSunset(now, lat, lon)
		if lightMin >= 0 && darkMin >= 0 {
			return ScheduleInfo{LightTime: formatMinute(lightMin), DarkTime: formatMinute(darkMin), OK: true}
		}
	}

	lightMin = parseTime(lightTime)
	darkMin = parseTime(darkTime)
	if lightMin < 0 || darkMin < 0 {
		return ScheduleInfo{}
	}
	return ScheduleInfo{
		LightTime:     formatMinute(lightMin),
		DarkTime:      formatMinute(darkMin),
		OK:            true,
		FixedFallback: mode == "sunrise",
	}
}

// Scheduler checks time periodically and switches theme.
type Scheduler struct {
	mode           string // "fixed" or "sunrise"
	lightTime      string // HH:MM
	darkTime       string // HH:MM
	latitude       float64
	longitude      float64
	skipFullscreen bool
	darkOnBattery  bool
	stopCh         chan struct{}
	doneCh         chan struct{}
	wakeCh         chan struct{}
	running        bool
	mu             sync.Mutex
	checkMu        sync.Mutex
	manualUntil    atomic.Int64
	logMu          sync.Mutex
	lastSwitchErr  string
	lastPauseKey   string
	lastPauseErr   string
	pauseCheck     func(<-chan struct{}) (ThemeSwitchPauseReason, error)
	switchTheme    func(Mode) error
}

// NewScheduler creates a Scheduler.
func NewScheduler(mode, lightTime, darkTime string, lat, lon float64, skipFullscreen, darkOnBattery bool) *Scheduler {
	return &Scheduler{
		mode:           mode,
		lightTime:      lightTime,
		darkTime:       darkTime,
		latitude:       lat,
		longitude:      lon,
		skipFullscreen: skipFullscreen,
		darkOnBattery:  darkOnBattery,
		pauseCheck:     DetectThemeSwitchPause,
		switchTheme:    Switch,
	}
}

// Start begins the background check loop.
func (s *Scheduler) Start() {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.running {
		return
	}
	s.stopCh = make(chan struct{})
	s.doneCh = make(chan struct{})
	s.wakeCh = make(chan struct{}, 1)
	s.running = true
	go s.loop(s.stopCh, s.doneCh, s.wakeCh)
}

// Stop signals the loop to exit.
func (s *Scheduler) Stop() {
	s.mu.Lock()
	if !s.running {
		s.mu.Unlock()
		return
	}
	stopCh := s.stopCh
	doneCh := s.doneCh
	s.running = false
	close(stopCh)
	s.mu.Unlock()
	<-doneCh
}

func (s *Scheduler) loop(stopCh <-chan struct{}, doneCh chan<- struct{}, wakeCh <-chan struct{}) {
	ticker := time.NewTicker(60 * time.Second)
	defer ticker.Stop()
	defer close(doneCh)
	s.check(time.Now(), stopCh)
	for {
		select {
		case <-stopCh:
			return
		case <-wakeCh:
			s.check(time.Now(), stopCh)
		case now := <-ticker.C:
			s.check(now, stopCh)
		}
	}
}

// CheckNow queues an immediate power/time policy evaluation. It is used when
// Windows power state changes so dark-on-battery does not wait for the regular
// scheduler tick. The evaluation stays on the scheduler goroutine because its
// optional foreground-GPU sampling must never block the application state loop.
func (s *Scheduler) CheckNow() {
	if s == nil {
		return
	}
	s.mu.Lock()
	if !s.running {
		s.mu.Unlock()
		return
	}
	wakeCh := s.wakeCh
	s.mu.Unlock()
	select {
	case wakeCh <- struct{}{}:
	default:
	}
}

// HoldManualOverride keeps a user-selected theme until the next scheduled
// light or dark transition. Battery dark-mode policy remains authoritative.
func (s *Scheduler) HoldManualOverride(now time.Time) {
	if s == nil {
		return
	}
	lightMin, darkMin := s.scheduleMinutes(now)
	if lightMin < 0 || darkMin < 0 {
		return
	}
	var next time.Time
	for _, minute := range []int{lightMin, darkMin} {
		candidate := wallclock.At(now, minute)
		if !candidate.After(now) {
			candidate = wallclock.At(now.AddDate(0, 0, 1), minute)
		}
		if next.IsZero() || candidate.Before(next) {
			next = candidate
		}
	}
	s.manualUntil.Store(next.UnixNano())
}

func (s *Scheduler) check(now time.Time, cancel <-chan struct{}) {
	s.checkMu.Lock()
	defer s.checkMu.Unlock()
	// If dark-on-battery is enabled and running on battery, force dark.
	if s.darkOnBattery && onBattery() {
		if Current() != ModeDark {
			s.switchIfAllowed("battery", ModeDark, cancel)
		}
		return
	}
	lightMin, darkMin := s.scheduleMinutes(now)
	if lightMin < 0 || darkMin < 0 {
		return
	}
	if until := s.manualUntil.Load(); until > now.UnixNano() {
		return
	}

	lightToday := wallclock.At(now, lightMin)
	darkToday := wallclock.At(now, darkMin)

	var target Mode
	if lightToday.Before(darkToday) {
		if now.After(lightToday) && now.Before(darkToday) {
			target = ModeLight
		} else {
			target = ModeDark
		}
	} else {
		if now.After(darkToday) || now.Before(lightToday) {
			target = ModeDark
		} else {
			target = ModeLight
		}
	}

	if Current() != target {
		s.switchIfAllowed("schedule", target, cancel)
	}
}

func (s *Scheduler) switchIfAllowed(source string, target Mode, cancel <-chan struct{}) {
	if s.skipFullscreen && s.pauseCheck != nil {
		reason, err := s.pauseCheck(cancel)
		if errors.Is(err, errThemeEnvironmentCheckCanceled) {
			return
		}
		if err != nil {
			s.logPauseCheckFailure(err)
		} else {
			s.clearPauseCheckFailure()
		}
		if reason != ThemeSwitchPauseNone {
			s.logThemeSwitchPause(source, target, reason)
			return
		}
		s.clearThemeSwitchPause()
	}

	if err := s.switchTheme(target); err != nil {
		s.logSwitchFailure(source, target, err)
	} else {
		s.clearSwitchFailure()
	}
}

func (s *Scheduler) logSwitchFailure(reason string, target Mode, err error) {
	key := fmt.Sprintf("%s|%s|%v", reason, themeModeName(target), err)
	s.logMu.Lock()
	if s.lastSwitchErr == key {
		s.logMu.Unlock()
		return
	}
	s.lastSwitchErr = key
	s.logMu.Unlock()
	mylog.Info("Theme switch failed: reason=%s target=%s error=%v", reason, themeModeName(target), err)
}

func (s *Scheduler) clearSwitchFailure() {
	s.logMu.Lock()
	s.lastSwitchErr = ""
	s.logMu.Unlock()
}

func (s *Scheduler) logThemeSwitchPause(source string, target Mode, reason ThemeSwitchPauseReason) {
	key := fmt.Sprintf("%s|%s|%s", source, themeModeName(target), reason)
	s.logMu.Lock()
	if s.lastPauseKey == key {
		s.logMu.Unlock()
		return
	}
	s.lastPauseKey = key
	s.logMu.Unlock()
	mylog.Info("Theme switch paused: source=%s target=%s reason=%s", source, themeModeName(target), reason)
}

func (s *Scheduler) clearThemeSwitchPause() {
	s.logMu.Lock()
	s.lastPauseKey = ""
	s.logMu.Unlock()
}

func (s *Scheduler) logPauseCheckFailure(err error) {
	key := err.Error()
	s.logMu.Lock()
	if s.lastPauseErr == key {
		s.logMu.Unlock()
		return
	}
	s.lastPauseErr = key
	s.logMu.Unlock()
	mylog.Info("Theme presentation-state detection unavailable; continuing with the scheduled switch: error=%v", err)
}

func (s *Scheduler) clearPauseCheckFailure() {
	s.logMu.Lock()
	s.lastPauseErr = ""
	s.logMu.Unlock()
}

func themeModeName(mode Mode) string {
	if mode == ModeDark {
		return "dark"
	}
	return "light"
}

func (s *Scheduler) scheduleMinutes(now time.Time) (int, int) {
	if s.mode == "sunrise" {
		lightMin, darkMin := CalcSunriseSunset(now, s.latitude, s.longitude)
		if lightMin >= 0 && darkMin >= 0 {
			return lightMin, darkMin
		}
	}
	return parseTime(s.lightTime), parseTime(s.darkTime)
}

func parseTime(s string) int {
	parts := strings.SplitN(s, ":", 2)
	if len(parts) != 2 {
		return -1
	}
	h, err := strconv.Atoi(parts[0])
	if err != nil {
		return -1
	}
	m, err := strconv.Atoi(parts[1])
	if err != nil {
		return -1
	}
	if h < 0 || h > 23 || m < 0 || m > 59 {
		return -1
	}
	return h*60 + m
}

func formatMinute(minute int) string {
	for minute < 0 {
		minute += 1440
	}
	minute %= 1440
	return fmt.Sprintf("%02d:%02d", minute/60, minute%60)
}
