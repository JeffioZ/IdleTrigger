package actionwarning

import (
	"fmt"
	"runtime"
	"testing"
	"unsafe"

	"github.com/JeffioZ/idletrigger/internal/i18n"
	"golang.org/x/sys/windows"
)

func TestOverflowingWarningBodyRemainsReadable(t *testing.T) {
	if testing.Short() {
		t.Skip("native window test")
	}
	for _, language := range []string{"en", "zh-CN"} {
		t.Run(language, func(t *testing.T) {
			runtime.LockOSThread()
			defer runtime.UnlockOSThread()
			class := windows.StringToUTF16Ptr("STATIC")
			hwnd, _, err := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), 0, wsPopup|wsClipChildren, 0, 0, 800, 560, 0, 0, 0, 0)
			if hwnd == 0 {
				t.Fatal(err)
			}
			active = windows.Handle(hwnd)
			defer hideNow()
			dpiScale, textScale = 4.5, 1
			options := Options{Seconds: 10, CancelText: "Cancel", ExecuteText: "Execute", Body: func(seconds int) string {
				return fmt.Sprintf(i18n.T(language, "automation_warning_body"), "Scheduled action", "Hibernate", seconds)
			}}
			buildControls(options)
			_, controls := warningLayout(800, 560, 4.5, 630)
			b := controls[0]
			bodyWidth = b.Right - b.Left - 80
			pSetWindowPos.Call(uintptr(bodyControl), 0, uintptr(b.Left), uintptr(b.Top), uintptr(b.Right-b.Left), uintptr(b.Bottom-b.Top), swpNoZOrder|swpNoActivate)
			setBodyText()
			count, _, _ := pSendMessage.Call(uintptr(bodyControl), 0x00BA, 0, 0) // EM_GETLINECOUNT
			if count < 2 {
				t.Fatal("test text did not wrap")
			}
			pSendMessage.Call(uintptr(bodyControl), emLineScroll, 0, 10000)
			first, _, _ := pSendMessage.Call(uintptr(bodyControl), emGetFirstVisibleLine, 0, 0)
			if first == 0 {
				t.Fatal("overflowing body could not scroll")
			}
			last, _, _ := pSendMessage.Call(uintptr(bodyControl), 0x00BB, count-1, 0) // EM_LINEINDEX
			pos, _, _ := pSendMessage.Call(uintptr(bodyControl), 0x00D6, last, 0)     // EM_POSFROMCHAR
			y := int32(int16(pos >> 16))
			if pos == ^uintptr(0) || y < 0 || y >= b.Bottom-b.Top {
				t.Fatal("last line is still outside viewport")
			}
			bodyText = options.Body(9)
			setBodyText()
			after, _, _ := pSendMessage.Call(uintptr(bodyControl), emGetFirstVisibleLine, 0, 0)
			if after != first {
				t.Fatalf("countdown reset reading position: %d -> %d", first, after)
			}
		})
	}
}
