package settingspanel

import (
	"errors"
	"fmt"
	"runtime"
	"strings"
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"
)

func TestBuildFailureReleasesPartialSettingsWindow(t *testing.T) {
	if testing.Short() {
		t.Skip("native window test")
	}
	for _, failID := range []uint16{idTitle, idBatteryThreshold, idSave} {
		t.Run(fmt.Sprint(failID), func(t *testing.T) {
			original := createSettingsControl
			t.Cleanup(func() { createSettingsControl = original })
			injected := errors.New("injected child creation failure")
			var created []windows.Handle
			var fonts []windows.Handle
			var owner windows.Handle
			failed := false
			createSettingsControl = func(className, value string, style, x, y, width, height, parent, id uintptr) (uintptr, uintptr, error) {
				runtime.GC()
				if failed {
					t.Error("creation continued after first failure")
				}
				if uint16(id) == failID {
					failed = true
					owner = active.hwnd
					fonts = []windows.Handle{active.font, active.sectionFont, active.titleFont}
					return 0, 0, injected
				}
				h, r, err := original(className, value, style, x, y, width, height, parent, id)
				if h != 0 {
					created = append(created, windows.Handle(h))
					buffer := make([]uint16, len(windows.StringToUTF16(value))+1)
					pSendMessage.Call(h, wmGetText, uintptr(len(buffer)), uintptr(unsafe.Pointer(&buffer[0])))
					if got := windows.UTF16ToString(buffer); got != value {
						t.Errorf("control text after GC = %q, want %q", got, value)
					}
				}
				return h, r, err
			}
			err := CapturePage(State{}, nil, 1, false, 0, func(windows.Handle) error {
				t.Error("incomplete settings window was presented")
				return nil
			})
			if !errors.Is(err, injected) || !strings.Contains(err.Error(), fmt.Sprint(failID)) {
				t.Fatalf("missing original error/control ID: %v", err)
			}
			if active != nil {
				t.Fatal("failed panel remains active")
			}
			for _, hwnd := range append(created, owner) {
				if ok, _, _ := pIsWindow.Call(uintptr(hwnd)); ok != 0 {
					t.Fatalf("window %x leaked", hwnd)
				}
			}
			getObject := windows.NewLazySystemDLL("gdi32.dll").NewProc("GetObjectW")
			for _, font := range fonts {
				var data [92]byte
				if n, _, _ := getObject.Call(uintptr(font), uintptr(len(data)), uintptr(unsafe.Pointer(&data[0]))); n != 0 {
					t.Fatalf("font %x leaked", font)
				}
			}
		})
	}
}
