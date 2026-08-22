package darkmode

import (
	"testing"
)

func TestPreferredAppModeArgument(t *testing.T) {
	tests := []struct {
		name       string
		build      uint32
		preference processThemePreference
		want       uintptr
		supported  bool
	}{
		{name: "server 2016 is unsupported", build: 14393, preference: processThemeForceDark},
		{name: "1809 follows system through legacy allow", build: 17763, preference: processThemeFollowSystem, want: 1, supported: true},
		{name: "1809 enables dark through legacy allow", build: 17763, preference: processThemeForceDark, want: 1, supported: true},
		{name: "1809 disables dark through legacy allow", build: 17763, preference: processThemeForceLight, want: 0, supported: true},
		{name: "pre-1903 still uses legacy allow", build: 18361, preference: processThemeForceLight, want: 0, supported: true},
		{name: "1903 follows system", build: 18362, preference: processThemeFollowSystem, want: preferredAppModeAllowDark, supported: true},
		{name: "1903 forces dark", build: 18362, preference: processThemeForceDark, want: preferredAppModeForceDark, supported: true},
		{name: "1903 forces light", build: 18362, preference: processThemeForceLight, want: preferredAppModeForceLight, supported: true},
		{name: "current Windows forces light", build: 28020, preference: processThemeForceLight, want: preferredAppModeForceLight, supported: true},
		{name: "unknown preference is rejected", build: 28020, preference: processThemePreference(99)},
		{name: "unknown legacy preference is rejected", build: 17763, preference: processThemePreference(99)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, supported := preferredAppModeArgument(tt.build, tt.preference)
			if got != tt.want || supported != tt.supported {
				t.Fatalf("preferredAppModeArgument(%d, %d) = (%d, %t), want (%d, %t)", tt.build, tt.preference, got, supported, tt.want, tt.supported)
			}
		})
	}
}

// Enable must not panic on any Windows version — it's called unconditionally at startup.
func TestEnable_NoPanic(t *testing.T) {
	Enable()
}

// Enable must be idempotent.
func TestEnable_Idempotent(t *testing.T) {
	Enable()
	Enable()
	Enable()
}

func TestAppsUseDark_NoPanic(t *testing.T) {
	AppsUseDark()
}
