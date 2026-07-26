package trayicon

import (
	"golang.org/x/sys/windows"
	"strings"
	"unsafe"
)

// Loads an icon resource embedded in the executable and shows it in the tray.
// Shell_NotifyIcon: https://msdn.microsoft.com/en-us/library/windows/desktop/bb762159(v=vs.85).aspx
func (t *winTray) setIcon(resourceID uint16) error {
	if !t.uiAvailable() {
		return errTrayUnavailable
	}
	side := t.desiredTrayIconSide()
	width, height := uintptr(side), uintptr(side)
	if err := t.setIconAtSize(resourceID, width, height); err != nil {
		return err
	}
	t.requestTrayIconConvergence()
	return nil
}

func (t *winTray) desiredTrayIconSide() uint32 {
	if iconRect, ok := t.notificationIconRect(); ok {
		if side := trayIconSide(iconRect); side != 0 {
			return side
		}
	}
	if side := taskbarSmallIconSide(); side != 0 {
		return side
	}
	return 64 // largest supplied tray frame: downscale safely, never upscale a small frame
}

func (t *winTray) setIconAtSize(resourceID uint16, width, height uintptr) error {
	t.muIconLifecycle.Lock()
	defer t.muIconLifecycle.Unlock()
	return t.setIconAtSizeLocked(resourceID, width, height)
}

func (t *winTray) setIconAtSizeLocked(resourceID uint16, width, height uintptr) error {
	const NIF_ICON = 0x00000002
	if !t.uiAvailable() {
		return errTrayUnavailable
	}
	key := loadedImageKey{resourceID: resourceID, width: uint32(width), height: uint32(height)}
	if t.trayIconKey == key {
		return nil
	}
	h, err := t.loadIconResource(key)
	if err != nil {
		return err
	}

	t.muNID.Lock()
	defer t.muNID.Unlock()
	if t.nid == nil {
		return errTrayUnavailable
	}
	t.nid.Icon = h
	t.nid.Flags |= NIF_ICON
	t.nid.Size = uint32(unsafe.Sizeof(*t.nid))

	if err := t.nid.modify(); err != nil {
		return err
	}
	t.trayIconKey = key
	return nil
}

func (t *winTray) notificationIconRect() (rect, bool) {
	t.muNID.RLock()
	if t.nid == nil {
		t.muNID.RUnlock()
		return rect{}, false
	}
	identifier := notifyIconIdentifier{Wnd: t.nid.Wnd, ID: t.nid.ID}
	t.muNID.RUnlock()
	identifier.Size = uint32(unsafe.Sizeof(identifier))
	var iconRect rect
	hr, _, _ := pShellNotifyIconGetRect.Call(
		uintptr(unsafe.Pointer(&identifier)),
		uintptr(unsafe.Pointer(&iconRect)),
	)
	return iconRect, int32(hr) == 0
}

func trayIconSide(iconRect rect) uint32 {
	width := iconRect.Right - iconRect.Left
	height := iconRect.Bottom - iconRect.Top
	if width < 16 || height < 16 || width > 256 || height > 256 {
		return 0
	}
	if width < height {
		return uint32(width)
	}
	return uint32(height)
}

func taskbarSmallIconSide() uint32 {
	const (
		abmGetTaskbarPos        = 0x00000005
		monitorDefaultToNearest = 0x00000002
	)
	data := appBarData{}
	data.Size = uint32(unsafe.Sizeof(data))
	result, _, _ := pSHAppBarMessage.Call(abmGetTaskbarPos, uintptr(unsafe.Pointer(&data)))
	if result == 0 {
		return 0
	}
	monitor, _, _ := pMonitorFromRect.Call(uintptr(unsafe.Pointer(&data.Rect)), monitorDefaultToNearest)
	if monitor == 0 {
		return 0
	}
	var scale uint32
	hr, _, _ := pGetScaleFactorForMonitor.Call(
		monitor,
		uintptr(unsafe.Pointer(&scale)),
	)
	if int32(hr) != 0 {
		return 0
	}
	return smallIconSideForScale(scale)
}

func smallIconSideForScale(scale uint32) uint32 {
	if scale < 100 || scale > 500 {
		return 0
	}
	return (16*scale + 50) / 100
}

func (t *winTray) refreshTrayIconFromShell() {
	side := t.desiredTrayIconSide()
	t.muIconLifecycle.Lock()
	resourceID := t.trayIconKey.resourceID
	if resourceID == 0 {
		t.muIconLifecycle.Unlock()
		return
	}
	err := t.setIconAtSizeLocked(resourceID, uintptr(side), uintptr(side))
	t.muIconLifecycle.Unlock()
	if err != nil {
		reportError("Unable to refresh tray icon: %v", err)
	}
}

const (
	trayIconProbeTimerID  = 0x51A1
	trayIconProbeInterval = 125
	trayIconProbeLimit    = 16 // two seconds; no persistent polling
)

func (t *winTray) requestTrayIconConvergence() {
	t.muUITasks.Lock()
	window := t.window
	closing := t.uiClosing
	t.muUITasks.Unlock()
	if window == 0 || closing {
		return
	}
	pPostMessage.Call(uintptr(window), wmStartTrayIconConvergence, 0, 0)
}

func (t *winTray) startTrayIconConvergence() {
	t.muUITasks.Lock()
	window := t.window
	closing := t.uiClosing
	t.muUITasks.Unlock()
	if window == 0 || closing {
		return
	}
	t.trayIconProbeAttempt = 0
	t.refreshTrayIconFromShell()
	if timer, _, _ := pSetTimer.Call(uintptr(window), trayIconProbeTimerID, trayIconProbeInterval, 0); timer == 0 {
		reportError("Unable to start tray icon size probe")
	}
}

func (t *winTray) continueTrayIconConvergence() {
	t.trayIconProbeAttempt++
	t.refreshTrayIconFromShell()
	if t.trayIconProbeAttempt >= trayIconProbeLimit {
		t.stopTrayIconConvergence()
	}
}

func (t *winTray) stopTrayIconConvergence() {
	t.muUITasks.Lock()
	window := t.window
	t.muUITasks.Unlock()
	if window != 0 {
		pKillTimer.Call(uintptr(window), trayIconProbeTimerID)
	}
	t.trayIconProbeAttempt = 0
}

// releaseLoadedIcons destroys only the HICON values created by LoadImageW in
// loadIconResource. It runs after NIM_DELETE (or destruction of the owner window),
// when Shell_NotifyIcon can no longer use the notification icon handle.
func (t *winTray) releaseLoadedIcons() {
	for _, icon := range t.takeLoadedIconsForRelease() {
		pDestroyIcon.Call(uintptr(icon))
	}
}

// takeLoadedIconsForRelease transfers the owned handles exactly once. Clearing
// the map before DestroyIcon prevents both a second release and later reuse of
// an invalid handle during shutdown.
func (t *winTray) takeLoadedIconsForRelease() []windows.Handle {
	t.muLoadedImages.Lock()
	defer t.muLoadedImages.Unlock()
	if t.iconsReleased {
		return nil
	}
	t.iconsReleased = true
	icons := make([]windows.Handle, 0, len(t.loadedImages))
	for _, icon := range t.loadedImages {
		if icon != 0 {
			icons = append(icons, icon)
		}
	}
	t.loadedImages = nil
	return icons
}

// removeNotificationIcon releases the Shell registration before destroying
// application-owned HICON values. WM_ENDSESSION can precede WM_DESTROY, so this
// operation is intentionally idempotent.
func (t *winTray) removeNotificationIcon() {
	t.muIconLifecycle.Lock()
	defer t.muIconLifecycle.Unlock()
	t.trayIconKey = loadedImageKey{}
	t.muNID.Lock()
	nid := t.nid
	t.nid = nil
	t.muNID.Unlock()
	if nid != nil {
		_ = nid.delete()
	}
	t.releaseLoadedIcons()
}

// Sets tooltip on icon.
// Shell_NotifyIcon: https://msdn.microsoft.com/en-us/library/windows/desktop/bb762159(v=vs.85).aspx
func (t *winTray) setTooltip(src string) error {
	const NIF_TIP = 0x00000004
	if !t.uiAvailable() {
		return errTrayUnavailable
	}
	src = fitTooltip(src, len(notifyIconData{}.Tip)-1)
	b, err := windows.UTF16FromString(src)
	if err != nil {
		return err
	}

	t.muNID.Lock()
	defer t.muNID.Unlock()
	if t.nid == nil {
		return errTrayUnavailable
	}
	for i := range t.nid.Tip {
		t.nid.Tip[i] = 0
	}
	if len(b) > len(t.nid.Tip) {
		b = b[:len(t.nid.Tip)]
	}
	if len(b) > 0 {
		copy(t.nid.Tip[:], b[:len(b)-1])
	}
	t.nid.Flags |= NIF_TIP
	t.nid.Size = uint32(unsafe.Sizeof(*t.nid))

	return t.nid.modify()
}

func fitTooltip(src string, maxUTF16 int) string {
	src = strings.TrimSpace(src)
	if maxUTF16 <= 0 {
		return ""
	}
	units := 0
	var out []rune
	for _, r := range src {
		width := 1
		if r > 0xFFFF {
			width = 2
		}
		if units+width > maxUTF16 {
			break
		}
		out = append(out, r)
		units += width
	}
	return trimTooltipSuffix(string(out))
}

func trimTooltipSuffix(src string) string {
	return strings.TrimRight(strings.TrimSpace(src), " \t\r\n·:：/-")
}

var wt winTray

const (
	wmRunUITask                = 0x8001
	wmStartTrayIconConvergence = 0x8002
)
