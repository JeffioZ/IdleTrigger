package app

import (
	"errors"
	"strings"
	"testing"
	"time"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/devtools"
	"github.com/JeffioZ/idletrigger/internal/feature/autorules"
	idlefeature "github.com/JeffioZ/idletrigger/internal/feature/idle"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
)

func TestOpenWithShellBuildsNativeArguments(t *testing.T) {
	previous := executeShell
	t.Cleanup(func() { executeShell = previous })

	var gotFile, gotArgs string
	executeShell = func(_ windows.Handle, _ *uint16, file, args, _ *uint16, _ int32) error {
		gotFile = windows.UTF16PtrToString(file)
		if args != nil {
			gotArgs = windows.UTF16PtrToString(args)
		}
		return nil
	}

	path := `C:\Program Files\IdleTrigger\IdleTrigger.toml`
	if err := openWithShell("notepad.exe", windows.EscapeArg(path)); err != nil {
		t.Fatalf("openWithShell: %v", err)
	}
	if gotFile != "notepad.exe" || gotArgs != `"C:\Program Files\IdleTrigger\IdleTrigger.toml"` {
		t.Fatalf("shell launch = %q %q", gotFile, gotArgs)
	}
}

func TestNoSleepRequestSources(t *testing.T) {
	cfg := config.DefaultConfig()
	state := runtimeState{cfg: cfg}
	if state.noSleepRequested() {
		t.Fatal("unexpected request")
	}
	state.autoState.StayAwake = true
	if !state.noSleepRequested() {
		t.Fatal("automatic task request was ignored")
	}
	state.autoState.StayAwake = false
	state.cfg.NoSleepEnabled = true
	if !state.noSleepRequested() {
		t.Fatal("user request was ignored")
	}
	state.autoState.PauseStayAwake = true
	if state.noSleepRequested() {
		t.Fatal("automatic pause did not override the user request")
	}
	state.autoState.PauseStayAwake = false
	state.cfg.NoSleepEnabled = false
	if state.noSleepRequested() {
		t.Fatal("cleared sources still requested Stay Awake")
	}
}

func TestDeveloperIdleMonitorUsesSafeRuntimeSettings(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.IdleTimeoutMinutes = 0
	cfg.IdleAction = config.ActionShutdown
	state := runtimeState{
		cfg:      cfg,
		devtools: devtools.Config{Enabled: true, IdleMonitorSeconds: 10},
	}
	if !state.idleMonitorRequested() {
		t.Fatal("developer idle monitor should run even when config monitor is disabled")
	}
	threshold, warning, action, active := state.effectiveIdleMonitorSettings()
	if !active || threshold != 10*time.Second || warning != 5*time.Second || action != config.ActionLock {
		t.Fatalf("developer idle settings = %s/%s/%s/%v", threshold, warning, action, active)
	}
	if state.cfg.IdleTimeoutMinutes != 0 || state.cfg.IdleAction != config.ActionShutdown {
		t.Fatalf("developer idle monitor changed config: %+v", state.cfg)
	}
}

func TestDeveloperIdleMonitorStillRespectsStayAwakeMutualExclusion(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.NoSleepEnabled = true
	state := runtimeState{
		cfg:      cfg,
		devtools: devtools.Config{Enabled: true, IdleMonitorSeconds: 10},
	}
	if !state.idleSuspended() {
		t.Fatal("developer idle monitor must remain suspended by Stay Awake")
	}
}

func TestBatteryLoopIsLazy(t *testing.T) {
	s := &runtimeState{
		cfg:       config.DefaultConfig(),
		requestCh: make(chan runtimeRequest, 1),
	}
	s.syncBatteryLoop()
	if s.batteryStop != nil {
		t.Fatal("battery loop started with all battery-aware features disabled")
	}

	s.cfg.NoSleepEnabled = true
	s.syncBatteryLoop()
	if s.batteryStop == nil {
		t.Fatal("battery loop did not start for Stay Awake")
	}

	s.cfg.NoSleepEnabled = false
	s.syncBatteryLoop()
	if s.batteryStop != nil {
		t.Fatal("battery loop did not stop after Stay Awake was disabled")
	}
}

func TestBatteryPolicyBlocksAndRestores(t *testing.T) {
	cfg := config.DefaultConfig()
	battery := powerstate.Status{Battery: true, Percent: 80, Valid: true}
	if !batteryPolicyBlocks(cfg, battery) {
		t.Fatal("battery policy should block by default")
	}
	cfg.NoSleepOnBattery = true
	if batteryPolicyBlocks(cfg, battery) {
		t.Fatal("battery policy should allow an adequately charged battery")
	}
	battery.Percent = 10
	if !batteryPolicyBlocks(cfg, battery) {
		t.Fatal("low battery threshold was ignored")
	}
	battery.Percent = 0
	if !batteryPolicyBlocks(cfg, battery) {
		t.Fatal("empty battery should remain below the safety threshold")
	}
	battery.ACLine = true
	if batteryPolicyBlocks(cfg, battery) {
		t.Fatal("AC power should clear the battery block")
	}
}

func TestBatteryPolicyReasons(t *testing.T) {
	cfg := config.DefaultConfig()
	tests := []struct {
		name   string
		status powerstate.Status
		setup  func(*config.Config)
		want   string
	}{
		{"unknown", powerstate.Status{}, nil, "power-status-unknown"},
		{"ac", powerstate.Status{Valid: true, ACLine: true, Battery: true, Percent: 80}, nil, "ac-or-no-battery"},
		{"battery disabled", powerstate.Status{Valid: true, Battery: true, Percent: 80}, nil, "battery-not-allowed"},
		{"battery low", powerstate.Status{Valid: true, Battery: true, Percent: 10}, func(c *config.Config) { c.NoSleepOnBattery = true }, "battery-below-threshold"},
		{"battery allowed", powerstate.Status{Valid: true, Battery: true, Percent: 80}, func(c *config.Config) { c.NoSleepOnBattery = true }, "battery-allowed"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			current := cfg
			if tt.setup != nil {
				tt.setup(&current)
			}
			if got := batteryPolicyReason(current, tt.status); got != tt.want {
				t.Fatalf("batteryPolicyReason() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestPowerEventClassification(t *testing.T) {
	for _, tt := range []struct {
		event  uint32
		name   string
		resume bool
	}{
		{pbtAPMSuspend, "suspend", false},
		{pbtAPMResumeSuspend, "resume-user", true},
		{pbtAPMResumeAutomatic, "resume-automatic", true},
		{pbtPowerSettingChange, "power-setting-change", false},
		{0xffff, "unknown", false},
	} {
		if got := powerEventName(tt.event); got != tt.name {
			t.Errorf("powerEventName(0x%x) = %q, want %q", tt.event, got, tt.name)
		}
		if got := isResumePowerEvent(tt.event); got != tt.resume {
			t.Errorf("isResumePowerEvent(0x%x) = %v, want %v", tt.event, got, tt.resume)
		}
	}
}

func TestRuntimeModeConfigTransitions(t *testing.T) {
	cfg := config.DefaultConfig()
	setNoSleepConfig(&cfg, true, true)
	if !cfg.NoSleepEnabled || !cfg.KeepScreenOn || cfg.IdleTimeoutMinutes != 0 {
		t.Fatalf("Stay Awake transition = %+v", cfg)
	}

	setIdleTimeoutConfig(&cfg, 90)
	if cfg.NoSleepEnabled || cfg.IdleTimeoutMinutes != 90 {
		t.Fatalf("idle-monitor transition = %+v", cfg)
	}
}

func TestActionAvailability(t *testing.T) {
	tests := []struct {
		action config.Action
		caps   powerstate.Capabilities
		want   bool
	}{
		{config.ActionSleep, powerstate.Capabilities{}, false},
		{config.ActionSleep, powerstate.Capabilities{SleepAvailable: true}, true},
		{config.ActionHibernate, powerstate.Capabilities{}, false},
		{config.ActionHibernate, powerstate.Capabilities{HibernateAvailable: true}, true},
		{config.ActionShutdown, powerstate.Capabilities{}, true},
		{config.ActionLock, powerstate.Capabilities{}, true},
	}
	for _, tt := range tests {
		if got := actionAvailable(tt.action, tt.caps); got != tt.want {
			t.Errorf("actionAvailable(%q, %+v) = %v, want %v", tt.action, tt.caps, got, tt.want)
		}
	}
}

func TestIPLocationLookupEligibility(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.ThemeMode = "sunrise"
	cfg.ThemeIPLocationEnabled = true
	if !ipLocationLookupEnabled(cfg) {
		t.Fatal("IP lookup should be enabled for sunrise mode without coordinates")
	}
	cfg.ThemeLatitude = 31.2
	if ipLocationLookupEnabled(cfg) {
		t.Fatal("manual coordinates should disable IP lookup")
	}
	cfg.ThemeLatitude = 0
	cfg.ThemeMode = "fixed"
	if ipLocationLookupEnabled(cfg) {
		t.Fatal("fixed mode should disable IP lookup")
	}
	cfg.ThemeMode = "sunrise"
	cfg.ThemeIPLocationEnabled = false
	if ipLocationLookupEnabled(cfg) {
		t.Fatal("disabled option should disable IP lookup")
	}
}

func TestIPLocationRetryIsLimitedToOncePerCycle(t *testing.T) {
	state := &runtimeState{ipLocationGeneration: 7}
	state.handleIPLocationFailure(7)
	if state.ipLocationRetry == nil {
		t.Fatal("first failure did not schedule a retry")
	}
	state.cancelIPLocationRetry()
	state.ipLocationRetried = true
	state.handleIPLocationFailure(7)
	if state.ipLocationRetry != nil {
		t.Fatal("second failure scheduled another retry")
	}
	state.ipLocationRetried = false
	state.handleIPLocationFailure(6)
	if state.ipLocationRetry != nil {
		t.Fatal("stale lookup generation scheduled a retry")
	}
}

func TestIPLocationConfigSyncDoesNotResetActiveCycle(t *testing.T) {
	state := &runtimeState{cfg: config.DefaultConfig(), ipLocationGeneration: 3, ipLocationRetried: true}
	state.cfg.ThemeMode = "sunrise"
	state.cfg.ThemeIPLocationEnabled = true
	state.syncIPLocationCycle(true)
	if state.ipLocationGeneration != 3 || !state.ipLocationRetried {
		t.Fatalf("unchanged eligible config reset cycle: generation=%d retried=%v", state.ipLocationGeneration, state.ipLocationRetried)
	}

	state.cfg.ThemeIPLocationEnabled = false
	state.syncIPLocationCycle(true)
	if state.ipLocationGeneration != 4 || state.ipLocationRetried {
		t.Fatalf("disabled config did not stop cycle: generation=%d retried=%v", state.ipLocationGeneration, state.ipLocationRetried)
	}
}

func TestStatusLineUsesLocalizedPunctuation(t *testing.T) {
	state := runtimeState{lang: "zh-CN"}
	if got := state.statusLine("status_power", "交流电源"); got != "电源：交流电源" {
		t.Fatalf("localized status line = %q", got)
	}
}

func TestThemeCapabilityDisablesScheduleAndIPLookupWithoutChangingConfig(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.ThemeSwitchEnabled = true
	cfg.ThemeIPLocationEnabled = true
	state := runtimeState{
		cfg:                 cfg,
		lang:                "zh-CN",
		themeSupportChecked: true,
		themeSupported:      false,
		themeSupportErr:     errors.New("access denied"),
	}

	if state.themeAvailable() {
		t.Fatal("unsupported theme capability reported as available")
	}
	if state.themeIPLocationLookupEnabled() {
		t.Fatal("IP location remained active while the theme feature is unavailable")
	}
	if got := state.themeScheduleText(true); got != "当前系统不支持昼夜模式" {
		t.Fatalf("unsupported schedule text = %q", got)
	}
	if !state.cfg.ThemeSwitchEnabled || !state.cfg.ThemeIPLocationEnabled {
		t.Fatal("capability detection must preserve the saved user configuration")
	}
}

func TestTooltipStaysWithinTrayLimit(t *testing.T) {
	for _, lang := range []string{"zh-CN", "en"} {
		cfg := config.DefaultConfig()
		cfg.Language = lang
		cfg.IdleTimeoutMinutes = 30
		cfg.IdleAction = config.ActionSleep
		cfg.ThemeSwitchEnabled = true
		cfg.ThemeMode = "sunrise"
		cfg.HotkeysEnabled = true
		cfg.AutostartEnabled = true
		cfg.LoggingEnabled = true
		state := runtimeState{cfg: cfg, lang: lang}

		got := state.buildTooltip()
		if strings.Contains(got, "…") {
			t.Fatalf("tooltip should not be hard-truncated for %s: %q", lang, got)
		}
		if strings.Contains(got, "·") {
			t.Fatalf("tooltip should avoid separator truncation risk for %s: %q", lang, got)
		}
		if hasDanglingTooltipSuffix(got) {
			t.Fatalf("tooltip has dangling suffix for %s: %q", lang, got)
		}
		if length := len([]rune(got)); length > 120 {
			t.Fatalf("tooltip is too long for %s: %d %q", lang, length, got)
		}
	}
}

func TestThemeTooltipUsesDedicatedShortScheduleFormats(t *testing.T) {
	for _, tc := range []struct {
		lang string
		want string
	}{
		{lang: "en", want: "On L07:00/D19:00"},
		{lang: "zh-CN", want: "开 浅07:00/深19:00"},
	} {
		cfg := config.DefaultConfig()
		cfg.Language = tc.lang
		cfg.ThemeSwitchEnabled = true
		cfg.ThemeMode = "fixed"
		cfg.ThemeLightTime = "07:00"
		cfg.ThemeDarkTime = "19:00"
		state := runtimeState{
			cfg:                 cfg,
			lang:                tc.lang,
			themeSupportChecked: true,
			themeSupported:      true,
		}
		if got := state.themeTooltipValueShort(); got != tc.want {
			t.Fatalf("short theme schedule for %s = %q, want %q", tc.lang, got, tc.want)
		}
	}
}

func TestTooltipKeepsOnlyOperationalStates(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.ThemeSwitchEnabled = false
	cfg.HotkeysEnabled = true
	cfg.AutostartEnabled = true
	cfg.LoggingEnabled = true
	state := runtimeState{cfg: cfg, lang: "zh-CN"}

	got := state.buildTooltip()
	lines := strings.Split(got, "\n")
	if len(lines) != 4 {
		t.Fatalf("tooltip should keep status lines visible: %q", got)
	}
	if lines[0] != "IdleTrigger" {
		t.Fatalf("tooltip title = %q, want IdleTrigger", lines[0])
	}
	if !strings.Contains(got, "主题：关") {
		t.Fatalf("theme should remain visible: %q", got)
	}
	for _, omitted := range []string{"热键", "自启", "日志"} {
		if strings.Contains(got, omitted) {
			t.Fatalf("non-operational setting %q should not be in tooltip: %q", omitted, got)
		}
	}
}

func TestTooltipTitleAddsReleaseVersion(t *testing.T) {
	if got := tooltipTitle("1.3.5"); got != "IdleTrigger v1.3.5" {
		t.Fatalf("release tooltip title = %q", got)
	}
	if got := tooltipTitle("dev"); got != "IdleTrigger" {
		t.Fatalf("development tooltip title = %q", got)
	}
}

func TestAutomationTaskDoesNotSuspendIdleWithoutAwakeRequest(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.IdleTimeoutMinutes = 30
	state := runtimeState{cfg: cfg, lang: "zh-CN", autoState: autorules.EffectiveState{EnableIdle: true}}
	if state.idleSuspended() {
		t.Fatal("non-awake automatic task should not suspend idle monitoring")
	}
	if !state.idleMonitorRequested() {
		t.Fatal("automatic idle task should request idle monitoring")
	}
}

func TestIdleSuspendedByEffectiveKeepAwake(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.NoSleepEnabled = true
	cfg.IdleTimeoutMinutes = 30
	state := runtimeState{cfg: cfg, lang: "zh-CN"}
	if !state.idleSuspended() {
		t.Fatal("effective keep-awake should suspend the configured idle monitor")
	}
	if got := state.buildTooltip(); !strings.Contains(got, "空闲监测：已暂停") {
		t.Fatalf("tooltip should expose idle suspension: %q", got)
	}
	if got := state.monitorStatusText(); got != "保持唤醒时暂停" {
		t.Fatalf("monitor status = %q, want paused by Stay Awake", got)
	}
}

func TestAutomaticIdleStatusUsesEffectiveDuration(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.IdleTimeoutMinutes = 0
	cfg.IdleAction = config.ActionLock
	state := runtimeState{
		cfg:       cfg,
		lang:      "zh-CN",
		autoState: autorules.EffectiveState{EnableIdle: true, IdleMinutes: 45},
		mon:       &idlefeature.Monitor{},
	}
	if got := state.monitorStatusText(); got != "45 分钟 → 锁定" {
		t.Fatalf("automatic idle status = %q", got)
	}
}

func TestStayAwakeStatusExplainsBatteryPause(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.NoSleepEnabled = true
	state := runtimeState{cfg: cfg, lang: "zh-CN", batteryBlocked: true}
	if got := state.noSleepStatusText(); got != "已由电池策略暂停" {
		t.Fatalf("Stay Awake status = %q", got)
	}
}

func TestAutomationKeepScreenRequestIsIndependent(t *testing.T) {
	state := runtimeState{cfg: config.DefaultConfig()}
	state.autoState = autorules.EffectiveState{StayAwake: true, KeepScreenOn: true}
	if !state.noSleepRequested() || !state.effectiveKeepScreenOn() {
		t.Fatal("automatic Stay Awake screen request was ignored")
	}
	state.cfg.NoSleepEnabled = true
	state.cfg.KeepScreenOn = false
	if !state.effectiveKeepScreenOn() {
		t.Fatal("manual source must not clear an automatic screen request")
	}
	state.autoState = autorules.EffectiveState{}
	if state.effectiveKeepScreenOn() {
		t.Fatal("screen-on request remained after the automatic source ended")
	}
}

func TestStayAwakeStatusExplainsAutomationPause(t *testing.T) {
	state := runtimeState{cfg: config.DefaultConfig(), lang: "zh-CN"}
	state.cfg.NoSleepEnabled = true
	state.autoState.PauseStayAwake = true
	if got := state.noSleepStatusText(); got != "已由自动任务暂停" {
		t.Fatalf("status = %q", got)
	}
}

func TestAutomationOverviewText(t *testing.T) {
	now := time.Date(2026, 7, 15, 23, 0, 0, 0, time.Local)
	tests := []struct {
		name    string
		enabled bool
		rules   []automation.Rule
		next    time.Time
		want    string
	}{
		{name: "empty", enabled: true, want: "尚未创建任务"},
		{name: "paused", rules: []automation.Rule{{Enabled: true}, {Enabled: false}}, want: "自动任务已暂停 · 已配置 2 条"},
		{name: "enabled", enabled: true, rules: []automation.Rule{{Enabled: true}, {Enabled: false}}, want: "已启用 1 条 · 暂无计划执行的系统操作"},
		{name: "next", enabled: true, rules: []automation.Rule{{Enabled: true}}, next: now, want: "已启用 1 条 · 下次：2026-07-15 23:00"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			state := runtimeState{
				cfg:       config.Config{AutomationEnabled: test.enabled, AutomationRules: test.rules},
				lang:      "zh-CN",
				autoState: autorules.EffectiveState{NextOccurrence: test.next},
			}
			if got := state.automationOverviewText(); got != test.want {
				t.Fatalf("automationOverviewText() = %q, want %q", got, test.want)
			}
		})
	}
}

func hasDanglingTooltipSuffix(value string) bool {
	return strings.HasSuffix(value, " ") ||
		strings.HasSuffix(value, "·") ||
		strings.HasSuffix(value, ":") ||
		strings.HasSuffix(value, "：") ||
		strings.HasSuffix(value, "/")
}
