package nativeform

import (
	"fmt"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

type ChoicePopupItem struct {
	Label  string
	Value  int
	Header bool
	Danger bool
}

type ChoicePopupOptions struct {
	Owner, Anchor windows.Handle
	Font          windows.Handle
	SelectedFont  windows.Handle
	Palette       colors.Palette
	Dark          bool
	Scale         float64
	Selected      int
	MaxVisible    int
	Items         []ChoicePopupItem
	// KeepOpenOnReselect preserves selectors where the currently applied row is
	// an intentional no-op. RestoreAnchorOnCancel returns keyboard focus after
	// Escape/F4 without stealing it when the popup closes through mouse focus.
	KeepOpenOnReselect    bool
	RestoreAnchorOnCancel bool
	PreferAbove           bool
	FocusVisible          bool
	OnSelect              func(int)
	OnClose               func()
}

type ChoicePopup struct {
	hwnd         windows.Handle
	options      ChoicePopupOptions
	first        int
	hover        int
	pressed      int
	focus        int
	rowHeight    int32
	inset        int32
	radius       int32
	rowGap       int32
	closed       bool
	focusVisible bool
	scrollbar    *Scrollbar
}

type popupRect struct{ Left, Top, Right, Bottom int32 }
type popupMonitorInfo struct {
	Size          uint32
	Monitor, Work popupRect
	Flags         uint32
}
type popupWndClass struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}
type popupPaint struct {
	DC                         windows.Handle
	Erase                      int32
	Paint                      popupRect
	Restore, IncrementalUpdate int32
	Reserved                   [32]byte
}
type popupTrackMouse struct {
	Size      uint32
	Flags     uint32
	Track     windows.Handle
	HoverTime uint32
}

const (
	choicePopupClass = "IdleTriggerNativeChoicePopup"
	cpWMDestroy      = 0x0002
	cpWMKillFocus    = 0x0008
	cpWMPaint        = 0x000F
	cpWMEraseBkgnd   = 0x0014
	cpWMPrint        = 0x0317
	cpWMPrintClient  = 0x0318
	cpWMKeyDown      = 0x0100
	cpWMMouseMove    = 0x0200
	cpWMLButtonDown  = 0x0201
	cpWMLButtonUp    = 0x0202
	cpWMMouseWheel   = 0x020A
	cpWMMouseLeave   = 0x02A3
	cpWSPopup        = 0x80000000
	cpWSClipChildren = 0x02000000
	cpWSExToolWindow = 0x00000080
	cpSWPShowWindow  = 0x0040
	cpMonitorNearest = 2
	cpTMELeave       = 0x00000002
	cpVKReturn       = 0x0D
	cpVKSpace        = 0x20
	cpVKEnd          = 0x23
	cpVKHome         = 0x24
	cpVKEscape       = 0x1B
	cpVKUp           = 0x26
	cpVKDown         = 0x28
	cpVKF4           = 0x73
)

var (
	cpUser32         = windows.NewLazySystemDLL("user32.dll")
	cpCreateWindow   = cpUser32.NewProc("CreateWindowExW")
	cpDestroyWindow  = cpUser32.NewProc("DestroyWindow")
	cpDefWindowProc  = cpUser32.NewProc("DefWindowProcW")
	cpRegisterClass  = cpUser32.NewProc("RegisterClassExW")
	cpGetWindowRect  = cpUser32.NewProc("GetWindowRect")
	cpGetClientRect  = cpUser32.NewProc("GetClientRect")
	cpMonitorFromWnd = cpUser32.NewProc("MonitorFromWindow")
	cpGetMonitorInfo = cpUser32.NewProc("GetMonitorInfoW")
	cpSetWindowPos   = cpUser32.NewProc("SetWindowPos")
	cpSetActiveWnd   = cpUser32.NewProc("SetActiveWindow")
	cpSetFocus       = cpUser32.NewProc("SetFocus")
	cpIsChild        = cpUser32.NewProc("IsChild")
	cpSetCapture     = cpUser32.NewProc("SetCapture")
	cpReleaseCapture = cpUser32.NewProc("ReleaseCapture")
	cpTrackMouse     = cpUser32.NewProc("TrackMouseEvent")
	cpInvalidateRect = cpUser32.NewProc("InvalidateRect")
	cpBeginPaint     = cpUser32.NewProc("BeginPaint")
	cpEndPaint       = cpUser32.NewProc("EndPaint")
	cpLoadCursor     = cpUser32.NewProc("LoadCursorW")
	cpClassOnce      sync.Once
	cpClassErr       error
	cpCallback       = windows.NewCallback(choicePopupWndProc)
	cpMu             sync.Mutex
	cpWindows        = make(map[windows.Handle]*ChoicePopup)
)

func ShowChoicePopup(options ChoicePopupOptions) (*ChoicePopup, error) {
	if options.Owner == 0 || options.Anchor == 0 || len(options.Items) == 0 {
		return nil, fmt.Errorf("invalid choice popup options")
	}
	cpClassOnce.Do(func() {
		name, _ := windows.UTF16PtrFromString(choicePopupClass)
		cursor, _, _ := cpLoadCursor.Call(0, 32512)
		wc := popupWndClass{Size: uint32(unsafe.Sizeof(popupWndClass{})), WndProc: cpCallback, Cursor: windows.Handle(cursor), ClassName: name}
		registered, _, err := cpRegisterClass.Call(uintptr(unsafe.Pointer(&wc)))
		if registered == 0 && err != windows.ERROR_CLASS_ALREADY_EXISTS {
			cpClassErr = fmt.Errorf("register choice popup: %w", err)
		}
	})
	if cpClassErr != nil {
		return nil, cpClassErr
	}
	if options.Scale <= 0 {
		options.Scale = 1
	}
	if options.MaxVisible <= 0 {
		options.MaxVisible = 6
	}
	visible := min(options.MaxVisible, len(options.Items))
	popup := &ChoicePopup{
		options: options, hover: -1, pressed: -1, focus: -1,
		rowHeight: int32(MenuRowHeight*options.Scale + 0.5), rowGap: int32(MenuRowGap*options.Scale + 0.5),
		inset: int32(MenuSurfaceInset*options.Scale + 0.5), radius: int32(CornerRadius*options.Scale + 0.5),
		focusVisible: options.FocusVisible,
	}
	for index, item := range options.Items {
		if !item.Header && item.Value == options.Selected {
			popup.focus = index
			break
		}
	}
	if popup.focus < 0 {
		popup.focus = popup.nextSelectable(-1, 1)
	}
	class, _ := windows.UTF16PtrFromString(choicePopupClass)
	style := uintptr(cpWSPopup | cpWSClipChildren)
	hwnd, _, err := cpCreateWindow.Call(cpWSExToolWindow, uintptr(unsafe.Pointer(class)), 0, style, 0, 0, 1, 1, uintptr(options.Owner), 0, 0, 0)
	if hwnd == 0 {
		return nil, fmt.Errorf("create choice popup: %w", err)
	}
	popup.hwnd = windows.Handle(hwnd)
	cpMu.Lock()
	cpWindows[popup.hwnd] = popup
	cpMu.Unlock()
	ApplyFrame(popup.hwnd, options.Dark)
	var anchor popupRect
	cpGetWindowRect.Call(uintptr(options.Anchor), uintptr(unsafe.Pointer(&anchor)))
	monitor, _, _ := cpMonitorFromWnd.Call(uintptr(options.Owner), cpMonitorNearest)
	info := popupMonitorInfo{Size: uint32(unsafe.Sizeof(popupMonitorInfo{}))}
	if ok, _, _ := cpGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); ok == 0 {
		cpDestroyWindow.Call(hwnd)
		return nil, fmt.Errorf("read choice popup work area")
	}
	visible = choicePopupVisibleRows(visible, info.Work.Bottom-info.Work.Top, popup.rowHeight, popup.rowGap, popup.inset)
	popup.options.MaxVisible = visible
	popup.ensureVisible(popup.focus, visible)
	width := anchor.Right - anchor.Left
	width = min(width, info.Work.Right-info.Work.Left)
	height := 2*popup.inset + int32(visible)*popup.rowHeight + int32(max(0, visible-1))*popup.rowGap
	if len(options.Items) > visible {
		scrollbar, scrollErr := NewScrollbar(ScrollbarOptions{
			Parent: popup.hwnd, Palette: options.Palette, Background: options.Palette.ElevatedSurface, Scale: options.Scale,
			OnChange: func(position int) {
				popup.first = position
				popup.hover, popup.pressed = -1, -1
				cpInvalidateRect.Call(uintptr(popup.hwnd), 0, 0)
			},
		})
		if scrollErr != nil {
			cpDestroyWindow.Call(uintptr(popup.hwnd))
			return nil, scrollErr
		}
		popup.scrollbar = scrollbar
		scrollWidth := int(float64(ScrollbarWidth)*options.Scale + 0.5)
		scrollbar.SetBounds(int(width)-scrollWidth-popup.logicalInset(), popup.logicalInset(), scrollWidth, int(height)-2*popup.logicalInset())
	}
	anchorGap := int32(MenuAnchorGap*options.Scale + 0.5)
	x := anchor.Left
	y := choicePopupVerticalPosition(anchor, info.Work, height, anchorGap, options.PreferAbove)
	if x+width > info.Work.Right {
		x = info.Work.Right - width
	}
	if x < info.Work.Left {
		x = info.Work.Left
	}
	// The popup owns keyboard navigation, so it must become the active owned
	// window before receiving focus. Showing it with SWP_NOACTIVATE and then
	// calling SetFocus causes Windows to restore focus to the owner immediately
	// on some builds, which closes the popup through WM_KILLFOCUS.
	cpSetWindowPos.Call(hwnd, ^uintptr(0), uintptr(x), uintptr(y), uintptr(width), uintptr(height), cpSWPShowWindow)
	popup.updateScroll(visible)
	cpSetActiveWnd.Call(hwnd)
	cpSetFocus.Call(hwnd)
	if !popup.IsOpen() {
		return nil, fmt.Errorf("choice popup closed during activation")
	}
	return popup, nil
}

func choicePopupVisibleRows(requested int, height, row, gap, inset int32) int {
	return max(1, min(requested, int((height-2*inset+gap)/max(1, row+gap))))
}

func choicePopupVerticalPosition(anchor, work popupRect, height, gap int32, preferAbove bool) int32 {
	above := anchor.Top - height - gap
	below := anchor.Bottom + gap
	if preferAbove {
		if above >= work.Top {
			return above
		}
		if below+height <= work.Bottom {
			return below
		}
	} else {
		if below+height <= work.Bottom {
			return below
		}
		if above >= work.Top {
			return above
		}
	}
	return max(work.Top, work.Bottom-height)
}

func (p *ChoicePopup) Close() {
	p.close(false)
}

func (p *ChoicePopup) close(restoreAnchor bool) {
	if p != nil && p.hwnd != 0 && !p.closed {
		cpDestroyWindow.Call(uintptr(p.hwnd))
		if restoreAnchor && p.options.RestoreAnchorOnCancel && p.options.Anchor != 0 {
			cpSetFocus.Call(uintptr(p.options.Anchor))
		}
	}
}

func (p *ChoicePopup) IsOpen() bool { return p != nil && p.hwnd != 0 && !p.closed }

func (p *ChoicePopup) Window() windows.Handle {
	if p == nil || p.closed {
		return 0
	}
	return p.hwnd
}

// IsChoicePopupOwnedBy is safe during ShowChoicePopup re-entrancy. Windows can
// deactivate the owner while the popup is being shown, before ShowChoicePopup
// has returned its handle to the caller; the registry is already authoritative
// at that point.
func IsChoicePopupOwnedBy(window, owner windows.Handle) bool {
	if window == 0 || owner == 0 {
		return false
	}
	cpMu.Lock()
	popup := cpWindows[window]
	owned := popup != nil && !popup.closed && popup.options.Owner == owner
	cpMu.Unlock()
	return owned
}

func (p *ChoicePopup) nextSelectable(start, delta int) int {
	for index := start + delta; index >= 0 && index < len(p.options.Items); index += delta {
		if !p.options.Items[index].Header {
			return index
		}
	}
	return start
}

func (p *ChoicePopup) visibleRows() int {
	return min(p.options.MaxVisible, len(p.options.Items))
}

func (p *ChoicePopup) logicalInset() int { return int(p.inset) }

func (p *ChoicePopup) ensureVisible(index, visible int) {
	if index < p.first {
		p.first = index
	} else if index >= p.first+visible {
		p.first = index - visible + 1
	}
	p.first = max(0, min(p.first, max(0, len(p.options.Items)-visible)))
}

func (p *ChoicePopup) updateScroll(visible int) {
	if p.scrollbar != nil {
		p.scrollbar.SetMetrics(len(p.options.Items), visible, p.first)
	}
}

func (p *ChoicePopup) scroll(delta int) {
	visible := p.visibleRows()
	p.first = max(0, min(p.first+delta, max(0, len(p.options.Items)-visible)))
	p.updateScroll(visible)
	cpInvalidateRect.Call(uintptr(p.hwnd), 0, 0)
}

func (p *ChoicePopup) rowAt(y int32) int {
	if y < p.inset {
		return -1
	}
	stride := p.rowHeight + p.rowGap
	if stride <= 0 {
		return -1
	}
	offset := y - p.inset
	if offset%stride >= p.rowHeight {
		return -1
	}
	row := p.first + int(offset/stride)
	if row < p.first || row >= min(len(p.options.Items), p.first+p.visibleRows()) || p.options.Items[row].Header {
		return -1
	}
	return row
}

func (p *ChoicePopup) apply(index int) {
	if index < 0 || index >= len(p.options.Items) || p.options.Items[index].Header {
		return
	}
	value, callback := p.options.Items[index].Value, p.options.OnSelect
	if choicePopupKeepsReselectionOpen(p.options.Selected, value, p.options.KeepOpenOnReselect) {
		cpInvalidateRect.Call(uintptr(p.hwnd), 0, 0)
		return
	}
	p.Close()
	if callback != nil {
		callback(value)
	}
}

func (p *ChoicePopup) draw(target windows.Handle, bounds Rect) {
	paint := func(dc windows.Handle, local Rect) {
		DrawSurface(dc, local, p.options.Palette, p.options.Palette.WindowBackground, p.options.Palette.ElevatedSurface, p.options.Palette.SubtleBorder, p.radius)
		contentRight := local.Right - p.inset
		if p.scrollbar != nil {
			contentRight -= int32(float64(ScrollbarWidth+2)*p.options.Scale + 0.5)
		}
		end := min(len(p.options.Items), p.first+p.visibleRows())
		for index := p.first; index < end; index++ {
			top := p.inset + int32(index-p.first)*(p.rowHeight+p.rowGap)
			row := Rect{Left: p.inset, Top: top, Right: contentRight, Bottom: top + p.rowHeight}
			item := p.options.Items[index]
			if item.Header {
				DrawPopupHeader(dc, row, p.options.Font, item.Label, p.options.Palette, p.options.Palette.ElevatedSurface, max(1, int32(p.options.Scale+0.5)))
				continue
			}
			state := ControlState{Hovered: index == p.hover, Pressed: index == p.pressed, Focused: p.focusVisible && index == p.focus}
			DrawMenuOption(dc, row, p.options.Font, p.options.SelectedFont, item.Label, p.options.Palette, p.options.Palette.ElevatedSurface, state, item.Value == p.options.Selected, item.Danger, p.radius, max(1, int32(p.options.Scale+0.5)))
		}
	}
	if !DrawBuffered(target, bounds, paint) {
		paint(target, bounds)
	}
}

func choicePopupWndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	cpMu.Lock()
	p := cpWindows[hwnd]
	cpMu.Unlock()
	if p == nil {
		result, _, _ := cpDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
		return result
	}
	switch message {
	case cpWMPaint:
		var paint popupPaint
		dc, _, _ := cpBeginPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&paint)))
		var client popupRect
		cpGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		clientBounds := Rect(client)
		p.draw(windows.Handle(dc), clientBounds)
		cpEndPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&paint)))
		return 0
	case cpWMPrint, cpWMPrintClient:
		var client popupRect
		cpGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		p.draw(windows.Handle(wParam), Rect(client))
		return 0
	case cpWMEraseBkgnd:
		return 1
	case cpWMMouseMove:
		y := int32(int16(lParam >> 16))
		row := p.rowAt(y)
		if row != p.hover || p.focusVisible {
			p.hover, p.focus = row, row
			p.focusVisible = false
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		}
		track := popupTrackMouse{Size: uint32(unsafe.Sizeof(popupTrackMouse{})), Flags: cpTMELeave, Track: hwnd}
		cpTrackMouse.Call(uintptr(unsafe.Pointer(&track)))
		return 0
	case cpWMMouseLeave:
		p.hover, p.pressed = -1, -1
		cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		return 0
	case cpWMLButtonDown:
		y := int32(int16(lParam >> 16))
		p.pressed = p.rowAt(y)
		cpSetCapture.Call(uintptr(hwnd))
		cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		return 0
	case cpWMLButtonUp:
		y := int32(int16(lParam >> 16))
		row := p.rowAt(y)
		pressed := p.pressed
		p.pressed = -1
		cpReleaseCapture.Call()
		if row >= 0 && row == pressed {
			p.apply(row)
		} else {
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		}
		return 0
	case cpWMMouseWheel:
		delta := int16(wParam >> 16)
		if delta > 0 {
			p.scroll(-1)
		} else if delta < 0 {
			p.scroll(1)
		}
		return 0
	case cpWMKeyDown:
		p.focusVisible = true
		switch wParam {
		case cpVKEscape, cpVKF4:
			p.close(true)
		case cpVKHome:
			p.focus = p.nextSelectable(-1, 1)
			p.ensureVisible(p.focus, p.visibleRows())
			p.updateScroll(p.visibleRows())
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		case cpVKEnd:
			p.focus = p.nextSelectable(len(p.options.Items), -1)
			p.ensureVisible(p.focus, p.visibleRows())
			p.updateScroll(p.visibleRows())
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		case cpVKUp:
			p.focus = p.nextSelectable(p.focus, -1)
			p.ensureVisible(p.focus, p.visibleRows())
			p.updateScroll(p.visibleRows())
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		case cpVKDown:
			p.focus = p.nextSelectable(p.focus, 1)
			p.ensureVisible(p.focus, p.visibleRows())
			p.updateScroll(p.visibleRows())
			cpInvalidateRect.Call(uintptr(hwnd), 0, 0)
		case cpVKReturn, cpVKSpace:
			p.apply(p.focus)
		}
		return 0
	case cpWMKillFocus:
		// Clicking an owner control can move focus through the owner HWND before
		// it reaches the actual child and emits BN_CLICKED. Keep the popup alive
		// throughout that internal transfer; the owner's command closes or toggles
		// it. Closing here would make the deferred anchor click reopen the popup.
		if choicePopupFocusStaysWithinOwner(p.options.Owner, p.options.Anchor, windows.Handle(wParam)) {
			return 0
		}
		p.Close()
		return 0
	case cpWMDestroy:
		if p.scrollbar != nil {
			p.scrollbar.Close()
			p.scrollbar = nil
		}
		p.closed = true
		p.hwnd = 0
		cpMu.Lock()
		delete(cpWindows, hwnd)
		cpMu.Unlock()
		if p.options.OnClose != nil {
			p.options.OnClose()
		}
		return 0
	}
	result, _, _ := cpDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}

func choicePopupFocusStaysWithinOwner(owner, anchor, next windows.Handle) bool {
	if next == 0 {
		return false
	}
	if anchor != 0 && next == anchor {
		return true
	}
	if owner == 0 {
		return false
	}
	if next == owner {
		return true
	}
	isChild, _, _ := cpIsChild.Call(uintptr(owner), uintptr(next))
	return isChild != 0
}

func choicePopupKeepsReselectionOpen(selected, value int, keepOpen bool) bool {
	return keepOpen && selected == value
}
