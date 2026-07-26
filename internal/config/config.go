// Package config reads and writes IdleTrigger's TOML configuration file.
// The config file (IdleTrigger.toml) lives next to the executable and is
// plain-text TOML — users can edit it directly.
package config

import "github.com/JeffioZ/idletrigger/internal/automation"

// Action is a system power action.
type Action string

const (
	ActionSleep     Action = "sleep"
	ActionHibernate Action = "hibernate"
	ActionShutdown  Action = "shutdown"
	ActionLock      Action = "lock"

	DefaultIdleTimeoutMinutes = 30
)

var idleActions = [...]Action{ActionSleep, ActionHibernate, ActionShutdown, ActionLock}

// ValidIdleAction reports whether action can be used by the idle monitor.
func ValidIdleAction(action Action) bool {
	return IdleActionIndex(action) >= 0
}

// IdleActionAt returns the idle-monitor action at the stable UI index.
func IdleActionAt(index int) (Action, bool) {
	if index < 0 || index >= len(idleActions) {
		return "", false
	}
	return idleActions[index], true
}

// IdleActionIndex returns the stable UI index for an idle-monitor action.
func IdleActionIndex(action Action) int {
	for index, candidate := range idleActions {
		if action == candidate {
			return index
		}
	}
	return -1
}

const configTemplateVersion = 13

// Config holds all user-configurable settings.
type Config struct {
	// Language for UI strings: "auto" (follow OS), "en", "zh-CN".
	Language string `toml:"language"`

	// IdleTimeoutMinutes is the idle time in minutes before the selected
	// action runs after no keyboard or mouse input. 0 disables the monitor.
	IdleTimeoutMinutes int `toml:"idle_timeout_minutes"`

	// IdleAction is the system action to run after the idle time is reached.
	IdleAction Action `toml:"idle_action"`

	// IdleWarningSeconds controls the in-app warning shown before the idle
	// action fires. 0 disables it for silent operation. Default: 30.
	IdleWarningSeconds int `toml:"idle_warning_seconds"`

	// IdleEnhancedMonitor enables enhanced idle monitoring when Windows idle
	// time is refreshed by a stable low-frequency source. Default: false.
	IdleEnhancedMonitor bool `toml:"idle_enhanced_monitor"`

	// NoSleepEnabled prevents the system from sleeping automatically.
	NoSleepEnabled bool `toml:"nosleep_enabled"`

	// KeepScreenOn prevents the display from turning off while NoSleep is active.
	KeepScreenOn bool `toml:"keep_screen_on"`

	// NoSleepOnBattery allows NoSleep to remain active when running on
	// battery power.  When false, NoSleep is auto-disabled on battery.
	NoSleepOnBattery bool `toml:"nosleep_on_battery"`

	// NoSleepBatteryThreshold is the minimum battery percentage below which
	// NoSleep is forced off (0–100, default 20).
	NoSleepBatteryThreshold int `toml:"nosleep_battery_threshold"`

	// HotkeysEnabled enables global keyboard shortcuts.
	HotkeysEnabled bool `toml:"hotkeys_enabled"`

	// AutomationEnabled is the master switch for built-in automatic tasks.
	AutomationEnabled bool `toml:"automation_enabled"`

	// AutomationRules contains only built-in actions. It cannot launch custom
	// commands, scripts, services, or arbitrary executables.
	AutomationRules []automation.Rule `toml:"automation_rules"`

	// AutomationIssues, SourceRevision, and LoadError are runtime metadata
	// populated while loading. They are never serialized into the user's TOML.
	AutomationIssues []automation.RuleIssue `toml:"-"`
	SourceRevision   string                 `toml:"-"`
	LoadError        string                 `toml:"-"`

	// LoggingEnabled writes debug logs to IdleTrigger.log.
	LoggingEnabled bool `toml:"logging_enabled"`

	// ThemeSwitchEnabled auto-switches Windows theme at scheduled times.
	ThemeSwitchEnabled bool `toml:"theme_switch_enabled"`

	// ThemeLightTime is when to switch to light mode (HH:MM).
	ThemeLightTime string `toml:"theme_light_time"`

	// ThemeDarkTime is when to switch to dark mode (HH:MM).
	ThemeDarkTime string `toml:"theme_dark_time"`

	// ThemeMode: "fixed" (use times above) or "sunrise" (calculate from location).
	ThemeMode string `toml:"theme_mode"`

	// ThemeLatitude / ThemeLongitude override automatic sunrise/sunset location.
	ThemeLatitude  float64 `toml:"theme_latitude"`
	ThemeLongitude float64 `toml:"theme_longitude"`

	// ThemeIPLocationEnabled allows an HTTPS IP geolocation lookup when
	// sunrise/sunset mode has no explicit coordinates.
	ThemeIPLocationEnabled bool `toml:"theme_ip_location_enabled"`

	// ThemeDarkOnBattery switches to dark mode when on battery.
	ThemeDarkOnBattery bool `toml:"theme_dark_on_battery"`

	// ThemeSkipFullscreen prevents switching during fullscreen apps,
	// presentation mode, or sustained foreground 3D activity.
	ThemeSkipFullscreen bool `toml:"theme_skip_fullscreen"`

	// AutostartEnabled mirrors the current user's registry Run entry. It is
	// runtime state rather than a TOML setting.
	AutostartEnabled bool `toml:"-"`
}

// DefaultConfig returns the factory-default configuration.
func DefaultConfig() Config {
	return Config{
		Language:                "auto",
		IdleTimeoutMinutes:      DefaultIdleTimeoutMinutes,
		IdleAction:              ActionSleep,
		IdleWarningSeconds:      30,
		IdleEnhancedMonitor:     false,
		NoSleepEnabled:          false,
		KeepScreenOn:            false,
		NoSleepOnBattery:        false,
		NoSleepBatteryThreshold: 20,
		HotkeysEnabled:          false,
		AutomationEnabled:       true,
		AutomationRules:         nil,
		LoggingEnabled:          false,
		ThemeSwitchEnabled:      false,
		ThemeLightTime:          "07:00",
		ThemeDarkTime:           "19:00",
		ThemeMode:               "sunrise",
		ThemeLatitude:           0,
		ThemeLongitude:          0,
		ThemeIPLocationEnabled:  false,
		ThemeDarkOnBattery:      true,
		ThemeSkipFullscreen:     true,
		AutostartEnabled:        false,
	}
}
