package settingspanel

import (
	"testing"

	"github.com/JeffioZ/idletrigger/internal/i18n"
)

func validDraft() settingsDraft {
	return settingsDraft{BaseRevision: "rev", BatteryThreshold: "20", IdleTimeout: "30", WarningSeconds: "30",
		LightTime: "07:00", DarkTime: "19:00"}
}

func TestParseSettingsDraftMapsLocationSources(t *testing.T) {
	for _, test := range []struct {
		name, mode string
		source     int
		ip         bool
	}{
		{"automatic", "fixed", 0, false},
		{"ip", "sunrise", 1, true},
	} {
		t.Run(test.name, func(t *testing.T) {
			draft := validDraft()
			draft.LocationSource = test.source
			if test.mode == "sunrise" {
				draft.ThemeMode = 1
			}
			request, id, key := parseSettingsDraft(draft)
			if id != 0 || key != "" || request.ThemeMode != test.mode || request.ThemeIPLocationEnabled != test.ip {
				t.Fatalf("request=%+v id=%d key=%q", request, id, key)
			}
		})
	}
}

func TestParseSettingsDraftMapsRestart(t *testing.T) {
	draft := validDraft()
	draft.IdleAction = 4
	request, id, key := parseSettingsDraft(draft)
	if id != 0 || key != "" || request.IdleAction != "restart" || idleActionIndex(request.IdleAction) != 4 {
		t.Fatalf("request=%+v id=%d key=%q", request, id, key)
	}
}

func TestIdleActionChoicesMatchSystemControlOrder(t *testing.T) {
	want := []string{"lock", "sleep", "hibernate", "shutdown", "restart"}
	if len(idleActionValues) != len(want) {
		t.Fatalf("idle action values = %v", idleActionValues)
	}
	for index := range want {
		if idleActionValues[index] != want[index] {
			t.Fatalf("idle action %d = %q, want %q", index, idleActionValues[index], want[index])
		}
	}
	if idleActionIndex("sleep") != 1 {
		t.Fatalf("sleep index = %d, want 1", idleActionIndex("sleep"))
	}
}

func TestParseSettingsDraftRejectsInvalidFields(t *testing.T) {
	for _, test := range []struct {
		name, value, key string
		id               uint16
		apply            func(*settingsDraft, string)
	}{
		{"battery", "101", "settings_error_battery_threshold", idBatteryThreshold, func(d *settingsDraft, v string) { d.BatteryThreshold = v }},
		{"idle", "0", "settings_error_idle_timeout", idIdleTimeout, func(d *settingsDraft, v string) { d.IdleTimeout = v }},
		{"warning", "-1", "settings_error_warning_seconds", idWarningSeconds, func(d *settingsDraft, v string) { d.WarningSeconds = v }},
		{"light time", "7:00", "settings_error_light_time", idLightTime, func(d *settingsDraft, v string) { d.LightTime = v }},
		{"dark time", "24:00", "settings_error_dark_time", idDarkTime, func(d *settingsDraft, v string) { d.DarkTime = v }},
	} {
		t.Run(test.name, func(t *testing.T) {
			draft := validDraft()
			test.apply(&draft, test.value)
			_, id, key := parseSettingsDraft(draft)
			if id != test.id || key != test.key {
				t.Fatalf("id=%d key=%q, want id=%d key=%q", id, key, test.id, test.key)
			}
		})
	}
}

func TestNumericInputFiltersMatchFieldSemantics(t *testing.T) {
	if got := filterDigits("2a0.5", 3); got != "205" {
		t.Fatalf("integer filter = %q", got)
	}
	if got := filterTime("07点:3x0"); got != "07:30" {
		t.Fatalf("time filter = %q", got)
	}
	if got := filterTime("1930"); got != "19:30" {
		t.Fatalf("pasted time filter = %q", got)
	}
}

func TestParseSettingsDraftRejectsIdleTimeoutOutsideRange(t *testing.T) {
	draft := validDraft()
	draft.IdleTimeout = "10081"
	_, id, key := parseSettingsDraft(draft)
	if id != idIdleTimeout || key != "settings_error_idle_timeout" {
		t.Fatalf("id=%d key=%q", id, key)
	}
}

func TestValidationFocusSelectsOwningPage(t *testing.T) {
	for _, test := range []struct {
		id   uint16
		page int
	}{
		{idBatteryThreshold, 0}, {idLocationSource, 1}, {idLanguage, 2},
	} {
		if got := pageForControl(test.id); got != test.page {
			t.Fatalf("control %d page=%d, want %d", test.id, got, test.page)
		}
	}
}

func TestEverySettingsControlHasLocalizedHelp(t *testing.T) {
	want := map[uint16]bool{
		idKeepScreen: true, idBatteryAllowed: true, idBatteryThreshold: true,
		idIdleTimeout: true, idIdleAction: true, idWarningSeconds: true, idIdleEnhanced: true,
		idThemeMode: true, idLightTime: true, idDarkTime: true, idLocationSource: true,
		idThemeBattery: true, idThemeFullscreen: true,
		idLanguage: true, idHotkeys: true, idAutostart: true,
		idLogging: true, idProjectHome: true, idCancel: true, idSave: true,
	}
	for _, binding := range settingsTooltipBindings() {
		for _, language := range []string{"en", "zh-CN"} {
			if value := i18n.T(language, binding.key); value == "" || value == binding.key {
				t.Errorf("tooltip %q is missing for %s", binding.key, language)
			}
		}
		for _, id := range binding.ids {
			delete(want, id)
		}
	}
	if len(want) != 0 {
		t.Fatalf("settings controls without tooltip coverage: %v", want)
	}
}

func TestChoiceControlsExposeTheirVisibleLabelsToAccessibility(t *testing.T) {
	for id, want := range map[uint16]uint16{
		idIdleAction: idIdleActionLabel, idThemeMode: idThemeModeLabel,
		idLocationSource: idLocationLabel, idLanguage: idLanguageLabel,
	} {
		if got := choiceLabelID(id); got != want {
			t.Errorf("choice %d label = %d, want %d", id, got, want)
		}
	}
}
