//go:build devtools

package controlpanel

import (
	"fmt"

	"golang.org/x/sys/windows"
)

// OpenCapturePopup opens one deterministic popup on a panel created by
// Capture. It is compiled only into devtools builds and never enters Release.
func OpenCapturePopup(panelWindow windows.Handle, surface string) (windows.Handle, error) {
	p := panelFor(panelWindow)
	if p == nil {
		return 0, fmt.Errorf("capture panel is not active")
	}
	switch surface {
	case "popup-system":
		p.openQuickMenu()
	default:
		return 0, fmt.Errorf("unknown capture popup %q", surface)
	}
	if p.choice.popup == nil || !p.choice.popup.IsOpen() {
		return 0, fmt.Errorf("capture popup %q did not open", surface)
	}
	return p.choice.popup.Window(), nil
}
