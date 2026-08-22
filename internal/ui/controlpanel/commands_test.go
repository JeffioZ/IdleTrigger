package controlpanel

import "testing"

func TestActionClosesPanel(t *testing.T) {
	wantClosed := map[Action]bool{
		ActSleep:            true,
		ActHibernate:        true,
		ActShutdown:         true,
		ActLock:             true,
		ActRestart:          true,
		ActExit:             true,
		ActNoSleepToggle:    false,
		ActAutomationOpen:   false,
		ActAutomationToggle: false,
		ActIdleToggle:       false,
		ActThemeToggle:      false,
		ActSwitchTheme:      false,
		ActRepairTheme:      false,
		ActSettingsOpen:     false,
	}
	for action, want := range wantClosed {
		if got := actionClosesPanel(action); got != want {
			t.Errorf("actionClosesPanel(%d) = %v, want %v", action, got, want)
		}
	}
}
