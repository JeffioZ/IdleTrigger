package config

import (
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/BurntSushi/toml"

	"github.com/JeffioZ/idletrigger/internal/automation"
)

func TestCoordinatesRejectNonFiniteValues(t *testing.T) {
	for _, value := range []float64{math.NaN(), math.Inf(1), math.Inf(-1)} {
		cfg := DefaultConfig()
		cfg.ThemeLatitude = value
		if err := cfg.Validate(); err == nil {
			t.Fatalf("Validate accepted non-finite latitude %v", value)
		}
		if got := NormalizeConfig(cfg).ThemeLatitude; got != DefaultConfig().ThemeLatitude {
			t.Fatalf("normalized latitude = %v, want default", got)
		}

		cfg = DefaultConfig()
		cfg.ThemeLongitude = value
		if err := cfg.Validate(); err == nil {
			t.Fatalf("Validate accepted non-finite longitude %v", value)
		}
		if got := NormalizeConfig(cfg).ThemeLongitude; got != DefaultConfig().ThemeLongitude {
			t.Fatalf("normalized longitude = %v, want default", got)
		}
	}
}

func TestIdleActionIndexRoundTrip(t *testing.T) {
	for index := 0; ; index++ {
		action, ok := IdleActionAt(index)
		if !ok {
			if index != 4 {
				t.Fatalf("idle action count = %d, want 4", index)
			}
			break
		}
		if got := IdleActionIndex(action); got != index {
			t.Fatalf("IdleActionIndex(%q) = %d, want %d", action, got, index)
		}
	}
}

func TestDefaultConfigValid(t *testing.T) {
	if err := DefaultConfig().Validate(); err != nil {
		t.Fatalf("default config must be valid: %v", err)
	}
}

func TestHotkeyDocumentationTemplateVersion(t *testing.T) {
	if configTemplateVersion < 13 {
		t.Fatalf("config template version = %d; Ctrl+Win+Shift+S documentation requires version 13 or later", configTemplateVersion)
	}
	cfg := DefaultConfig()
	if cfg.HotkeysEnabled {
		t.Fatal("global hotkeys must remain opt-in by default")
	}
	if text := renderAnnotatedTOML(cfg); !strings.Contains(text, "Ctrl+Win+Shift+S") {
		t.Fatal("annotated configuration does not document the current sleep hotkey")
	}
}

func TestExampleConfigParsesAndValidates(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "IdleTrigger.example.toml"))
	if err != nil {
		t.Fatal(err)
	}
	cfg := DefaultConfig()
	if err := toml.Unmarshal(data, &cfg); err != nil {
		t.Fatalf("parse example config: %v", err)
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate example config: %v", err)
	}
	text := string(data)
	assertConfigFieldsPresent(t, text)
	assertConfigOrder(t, text)
}

func TestExampleConfigBodyMatchesGeneratedAnnotatedConfig(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "IdleTrigger.example.toml"))
	if err != nil {
		t.Fatal(err)
	}
	exampleBody, ok := annotatedConfigBody(string(data))
	if !ok {
		t.Fatalf("example config is missing annotated body:\n%s", string(data))
	}
	generated := renderAnnotatedTOML(DefaultConfig())
	generatedBody, ok := annotatedConfigBody(generated)
	if !ok {
		t.Fatalf("generated config is missing annotated body:\n%s", generated)
	}
	if exampleBody != generatedBody {
		t.Fatalf("example config body differs from generated annotated config\n\nexample:\n%s\n\ngenerated:\n%s", exampleBody, generatedBody)
	}
}

func TestValidateRejectsUnsafeValues(t *testing.T) {
	tests := []struct {
		name   string
		mutate func(*Config)
	}{
		{"action", func(c *Config) { c.IdleAction = "format" }},
		{"negative timeout", func(c *Config) { c.IdleTimeoutMinutes = -1 }},
		{"battery threshold", func(c *Config) { c.NoSleepBatteryThreshold = 101 }},
		{"theme time", func(c *Config) { c.ThemeDarkTime = "not-a-time" }},
		{"latitude", func(c *Config) { c.ThemeLatitude = 91 }},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cfg := DefaultConfig()
			tt.mutate(&cfg)
			if err := cfg.Validate(); err == nil {
				t.Fatal("expected validation error")
			}
		})
	}
}

func TestSaveToAtomicallyReplacesExistingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	if err := os.WriteFile(path, []byte("old content"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := saveTo(path, DefaultConfig()); err != nil {
		t.Fatalf("saveTo: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	if strings.Contains(text, "old content") ||
		strings.Contains(text, "autostart_enabled") ||
		!strings.Contains(text, configTemplateVersionMarker()) ||
		!strings.Contains(text, "idle_action = \"sleep\"") ||
		!strings.Contains(text, "# -- 空闲监测 / Idle Monitoring --") {
		t.Fatalf("unexpected saved config: %s", text)
	}
	matches, err := filepath.Glob(filepath.Join(filepath.Dir(path), ".IdleTrigger-*.tmp"))
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 0 {
		t.Fatalf("temporary files left behind: %v", matches)
	}
}

func TestSaveToLocalizesHeader(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	cfg := DefaultConfig()
	cfg.Language = "zh-CN"
	if err := saveTo(path, cfg); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	if !strings.Contains(text, "# IdleTrigger 配置") ||
		!strings.Contains(text, "运行 config:reload") {
		t.Fatalf("localized config header missing: %s", text)
	}
}

func TestSaveToWritesAnnotatedConfigThatParses(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	cfg := DefaultConfig()
	cfg.Language = "zh-CN"
	cfg.AutomationRules = []automation.Rule{{
		ID: "presentation-awake", Name: "Presentation", Enabled: true,
		Action: automation.ActionStayAwake, Trigger: automation.TriggerProcessRunning,
		ProcessLogic: automation.ProcessAny,
		Processes:    []automation.ProcessTarget{{Match: automation.MatchName, Executable: "powerpnt.exe"}},
		IdleMinutes:  automation.DefaultIdleMinutes,
	}}
	cfg.ThemeLatitude = 31.2304
	cfg.ThemeLongitude = 121.4737
	if err := saveTo(path, cfg); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	for _, want := range []string{
		configTemplateVersionMarker(),
		"# -- 保持唤醒 / Stay Awake --",
		"# -- 自动任务 / Automatic Tasks --",
		"# -- 空闲监测 / Idle Monitoring --",
		"# -- 昼夜主题 / Day/Night Theme --",
		"# -- 设置 / Settings --",
		"[[automation_rules]]",
		"executable = \"powerpnt.exe\"",
		"theme_latitude = 31.2304",
		"theme_longitude = 121.4737",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("saved config missing %q:\n%s", want, text)
		}
	}
	assertConfigFieldsPresent(t, text)
	assertConfigOrder(t, text)

	parsed := DefaultConfig()
	if err := toml.Unmarshal(data, &parsed); err != nil {
		t.Fatalf("parse saved config: %v", err)
	}
	if err := parsed.Validate(); err != nil {
		t.Fatalf("validate saved config: %v", err)
	}
	if len(parsed.AutomationRules) != 1 || parsed.AutomationRules[0].ID != "presentation-awake" || parsed.ThemeLatitude != 31.2304 {
		t.Fatalf("parsed config mismatch: %+v", parsed)
	}
}

func TestTOMLStringUsesTOMLEscapesForControlCharacters(t *testing.T) {
	want := "bell:\a vertical-tab:\v"
	encoded := "value = " + tomlString(want)
	if strings.Contains(encoded, `\a`) || strings.Contains(encoded, `\v`) {
		t.Fatalf("TOML contains Go-only escapes: %q", encoded)
	}
	var decoded struct {
		Value string `toml:"value"`
	}
	if err := toml.Unmarshal([]byte(encoded), &decoded); err != nil {
		t.Fatalf("parse encoded TOML: %v", err)
	}
	if decoded.Value != want {
		t.Fatalf("decoded value = %q, want %q", decoded.Value, want)
	}
}

func TestLoadFromRefreshesExistingPlainConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	plain := strings.Join([]string{
		"language = \"zh-CN\"",
		"idle_timeout_minutes = 5",
		"idle_action = \"lock\"",
		"nosleep_enabled = true",
		"theme_mode = \"fixed\"",
		"theme_light_time = \"08:30\"",
		"theme_dark_time = \"20:45\"",
	}, "\n")
	if err := os.WriteFile(path, []byte(plain), 0600); err != nil {
		t.Fatal(err)
	}

	cfg, err := loadFrom(path)
	if err != nil {
		t.Fatalf("loadFrom: %v", err)
	}
	if cfg.Language != "zh-CN" ||
		cfg.IdleTimeoutMinutes != 5 ||
		cfg.IdleAction != ActionLock ||
		!cfg.NoSleepEnabled ||
		len(cfg.AutomationRules) != 0 ||
		cfg.ThemeLightTime != "08:30" ||
		cfg.ThemeDarkTime != "20:45" {
		t.Fatalf("loaded config mismatch: %+v", cfg)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	stableTime := time.Now().Add(-time.Hour).Truncate(time.Second)
	if err := os.Chtimes(path, stableTime, stableTime); err != nil {
		t.Fatal(err)
	}
	infoAfterRefresh, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	assertConfigFieldsPresent(t, text)
	assertConfigOrder(t, text)
	if needsAnnotatedTOMLRefresh(data) {
		t.Fatalf("refreshed config should not need another refresh:\n%s", text)
	}
	for _, want := range []string{
		configTemplateVersionMarker(),
		"# -- 保持唤醒 / Stay Awake --",
		"idle_timeout_minutes = 5",
		"idle_action = \"lock\"",
		"nosleep_enabled = true",
		"automation_enabled = true",
		"theme_light_time = \"08:30\"",
		"theme_dark_time = \"20:45\"",
		"language = \"zh-CN\"",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("refreshed config missing %q:\n%s", want, text)
		}
	}

	if _, err := loadFrom(path); err != nil {
		t.Fatalf("second loadFrom: %v", err)
	}
	infoAfterSecondLoad, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if !os.SameFile(infoAfterRefresh, infoAfterSecondLoad) {
		t.Fatal("annotated config was rewritten on the second load")
	}
	if !infoAfterSecondLoad.ModTime().Equal(stableTime) {
		t.Fatal("annotated config modification time changed on the second load")
	}
}

func TestLoadFromBacksUpConfigBeforeTemplateRefresh(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	current := renderAnnotatedTOML(DefaultConfig())
	previousMarker := fmt.Sprintf("# config_template_version = %d", configTemplateVersion-1)
	previous := strings.Replace(current, configTemplateVersionMarker(), previousMarker, 1)
	if previous == current {
		t.Fatal("failed to prepare previous template version")
	}
	if err := os.WriteFile(path, []byte(previous), 0600); err != nil {
		t.Fatal(err)
	}

	if _, err := loadFrom(path); err != nil {
		t.Fatalf("loadFrom: %v", err)
	}
	backup, err := os.ReadFile(path + ".bak")
	if err != nil {
		t.Fatalf("read template refresh backup: %v", err)
	}
	if string(backup) != previous {
		t.Fatal("template refresh backup does not match the original config")
	}
	refreshed, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(refreshed), configTemplateVersionMarker()) {
		t.Fatal("template refresh did not write the current version marker")
	}
}

func TestLoadFromBlocksSaveWhenTemplateRefreshBackupFails(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	current := renderAnnotatedTOML(DefaultConfig())
	previousMarker := fmt.Sprintf("# config_template_version = %d", configTemplateVersion-1)
	previous := strings.Replace(current, configTemplateVersionMarker(), previousMarker, 1)
	if err := os.WriteFile(path, []byte(previous), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(path+".bak", 0700); err != nil {
		t.Fatal(err)
	}

	cfg, err := loadFrom(path)
	if err == nil {
		t.Fatal("loadFrom should report a failed template backup")
	}
	if cfg.LoadError == "" {
		t.Fatal("failed template refresh should retain a save-blocking load error")
	}
	if _, err := saveToAtRevision(path, cfg, cfg.SourceRevision); !errors.Is(err, ErrConfigRecoveryRequired) {
		t.Fatalf("saveToAtRevision error = %v, want ErrConfigRecoveryRequired", err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != previous {
		t.Fatal("failed template backup changed the original config")
	}
}

func TestLoadFromKeepsMalformedConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	broken := []byte("idle_timeout_minutes = [")
	if err := os.WriteFile(path, broken, 0600); err != nil {
		t.Fatal(err)
	}

	cfg, err := loadFrom(path)
	if err == nil {
		t.Fatal("loadFrom should report malformed TOML")
	}
	if cfg.LoadError == "" {
		t.Fatal("malformed config should retain a save-blocking load error")
	}
	if cfg.SourceRevision != configRevision(broken) {
		t.Fatalf("source revision = %q, want revision of malformed file", cfg.SourceRevision)
	}
	if _, err := saveToAtRevision(path, cfg, cfg.SourceRevision); !errors.Is(err, ErrConfigRecoveryRequired) {
		t.Fatalf("saveToAtRevision error = %v, want ErrConfigRecoveryRequired", err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(broken) {
		t.Fatalf("malformed config was modified: %q", got)
	}
}

func TestRepairingMalformedConfigClearsSaveProtection(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	if err := os.WriteFile(path, []byte("idle_timeout_minutes = ["), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadFrom(path); err == nil {
		t.Fatal("loadFrom should report malformed TOML")
	}

	repaired := renderAnnotatedTOML(DefaultConfig())
	if err := os.WriteFile(path, []byte(repaired), 0600); err != nil {
		t.Fatal(err)
	}
	cfg, err := loadFrom(path)
	if err != nil {
		t.Fatalf("load repaired config: %v", err)
	}
	if cfg.LoadError != "" {
		t.Fatalf("repaired config retained load error: %q", cfg.LoadError)
	}
	if _, err := saveToAtRevision(path, cfg, cfg.SourceRevision); err != nil {
		t.Fatalf("save repaired config: %v", err)
	}
}

func TestLoadFromPreservesInvalidAutomationRulesWithoutRewrite(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	data := []byte(`automation_enabled = true

[[automation_rules]]
id = "valid"
name = "Valid"
enabled = true
action = "stay_awake"
trigger = "process_running"

[[automation_rules.processes]]
match = "name"
executable = "render.exe"

[[automation_rules]]
id = "invalid"
name = "Invalid"
enabled = true
action = "lock"
trigger = "daily"
time = "12:00"
warning_seconds = 5
`)
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}

	cfg, err := loadFrom(path)
	if err != nil {
		t.Fatalf("loadFrom: %v", err)
	}
	if len(cfg.AutomationIssues) != 1 || cfg.AutomationIssues[0].Index != 1 {
		t.Fatalf("automation issues = %+v", cfg.AutomationIssues)
	}
	if err := cfg.Validate(); err == nil {
		t.Fatal("config with a loaded invalid automation rule became saveable without an explicit correction")
	}
	runtimeRules := automation.RuntimeRules(cfg.AutomationRules, cfg.AutomationIssues)
	if len(runtimeRules) != 2 || !runtimeRules[0].Enabled || runtimeRules[1].Enabled {
		t.Fatalf("runtime rules = %+v", runtimeRules)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(data) {
		t.Fatalf("invalid rule config was rewritten:\n%s", got)
	}
	if _, err := os.Stat(path + ".bak"); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("invalid rule load created an unexpected backup: %v", err)
	}
}

func TestSaveToAtRevisionRejectsExternalChange(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	if err := saveTo(path, DefaultConfig()); err != nil {
		t.Fatal(err)
	}
	original, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	external := append(append([]byte(nil), original...), []byte("\n# external edit\n")...)
	if err := os.WriteFile(path, external, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := saveToAtRevision(path, DefaultConfig(), configRevision(original)); !errors.Is(err, ErrConfigChanged) {
		t.Fatalf("saveToAtRevision error = %v, want ErrConfigChanged", err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(external) {
		t.Fatal("external config was overwritten by a stale revision")
	}
}

func TestConfigRevisionIsCompactAndContentSensitive(t *testing.T) {
	revision := configRevision([]byte("idletrigger"))
	if len(revision) > 16 || revision == "" {
		t.Fatalf("config revision = %q, want a compact 64-bit token", revision)
	}
	if revision != configRevision([]byte("idletrigger")) {
		t.Fatal("identical configuration content produced different revisions")
	}
	if revision == configRevision([]byte("IdleTrigger")) {
		t.Fatal("different configuration content produced the same revision")
	}
}

func TestNormalizeConfigUsesValidationBounds(t *testing.T) {
	cfg := DefaultConfig()
	cfg.IdleTimeoutMinutes = 7*24*60 + 1
	cfg.IdleWarningSeconds = 3600
	got := NormalizeConfig(cfg)
	if got.IdleTimeoutMinutes != DefaultConfig().IdleTimeoutMinutes {
		t.Fatalf("idle timeout = %d, want default %d", got.IdleTimeoutMinutes, DefaultConfig().IdleTimeoutMinutes)
	}
	if got.IdleWarningSeconds != 3600 {
		t.Fatalf("idle warning = %d, want 3600", got.IdleWarningSeconds)
	}
}

func TestLoadFromRefreshesOlderTemplateVersion(t *testing.T) {
	path := filepath.Join(t.TempDir(), "IdleTrigger.toml")
	oldAnnotated := renderAnnotatedTOML(DefaultConfig())
	oldAnnotated = strings.Replace(oldAnnotated, configTemplateVersionMarker(), "# config_template_version = 1", 1)
	oldAnnotated = strings.Replace(oldAnnotated, "idle_timeout_minutes = 30", "idle_timeout_minutes = 120", 1)
	oldAnnotated = strings.Replace(oldAnnotated,
		"# 增强空闲监测：适合系统睡眠也被固定间隔空闲刷新干扰的机器；默认关闭，普通键鼠操作仍会重置计时 / Enhanced idle monitoring for machines where system sleep is disturbed by fixed-interval idle refreshes; off by default, and normal keyboard or mouse input still resets idle time\nidle_enhanced_monitor = false",
		"# Ignore stable keepalive input\nidle_ignore_keepalive_input = true", 1)
	if err := os.WriteFile(path, []byte(oldAnnotated), 0600); err != nil {
		t.Fatal(err)
	}

	cfg, err := loadFrom(path)
	if err != nil {
		t.Fatalf("loadFrom: %v", err)
	}
	if cfg.IdleTimeoutMinutes != 120 {
		t.Fatalf("user value was not preserved: %+v", cfg)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	if !strings.Contains(text, configTemplateVersionMarker()) ||
		!strings.Contains(text, "idle_timeout_minutes = 120") ||
		!strings.Contains(text, "idle_enhanced_monitor = false") ||
		strings.Contains(text, "idle_ignore_keepalive_input") ||
		strings.Contains(text, "Ignore stable keepalive input") ||
		needsAnnotatedTOMLRefresh(data) {
		t.Fatalf("old template version was not refreshed correctly:\n%s", text)
	}
}

func assertConfigFieldsPresent(t *testing.T, text string) {
	t.Helper()
	for _, field := range []string{
		"config_template_version",
		"nosleep_enabled =",
		"keep_screen_on =",
		"nosleep_on_battery =",
		"nosleep_battery_threshold =",
		"automation_enabled =",
		"idle_timeout_minutes =",
		"idle_action =",
		"idle_warning_seconds =",
		"idle_enhanced_monitor =",
		"theme_switch_enabled =",
		"theme_mode =",
		"theme_light_time =",
		"theme_dark_time =",
		"theme_latitude =",
		"theme_longitude =",
		"theme_ip_location_enabled =",
		"theme_dark_on_battery =",
		"theme_skip_fullscreen =",
		"hotkeys_enabled =",
		"logging_enabled =",
		"language =",
	} {
		if !strings.Contains(text, field) {
			t.Fatalf("config missing field %q:\n%s", field, text)
		}
	}
	if strings.Contains(text, "autostart_enabled") {
		t.Fatalf("runtime-only autostart field should not be saved:\n%s", text)
	}
}

func assertConfigOrder(t *testing.T, text string) {
	t.Helper()
	needles := []string{
		"# -- 保持唤醒 / Stay Awake --",
		"nosleep_enabled =",
		"# -- 自动任务 / Automatic Tasks --",
		"automation_enabled =",
		"# -- 空闲监测 / Idle Monitoring --",
		"idle_timeout_minutes =",
		"# -- 昼夜主题 / Day/Night Theme --",
		"theme_switch_enabled =",
		"theme_ip_location_enabled =",
		"# -- 设置 / Settings --",
		"hotkeys_enabled =",
		"logging_enabled =",
		"language =",
	}
	last := -1
	for _, needle := range needles {
		pos := strings.Index(text, needle)
		if pos < 0 {
			t.Fatalf("config missing %q:\n%s", needle, text)
		}
		if pos <= last {
			t.Fatalf("config order mismatch at %q:\n%s", needle, text)
		}
		last = pos
	}
}

func annotatedConfigBody(text string) (string, bool) {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	marker := "# -- 保持唤醒 / Stay Awake --\n"
	idx := strings.Index(text, marker)
	if idx < 0 {
		return "", false
	}
	return text[idx:], true
}
