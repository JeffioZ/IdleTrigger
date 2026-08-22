package processpicker

import (
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

const (
	headerListSubclassID = 0x49544844
	headerSubclassID     = 0x49544845
	headerMouseMove      = 0x0200
	headerLButtonDown    = 0x0201
	headerLButtonUp      = 0x0202
	headerMouseWheel     = 0x020A
	headerMouseLeave     = 0x02A3
	headerCancelMode     = 0x001F
	headerCaptureChanged = 0x0215
	headerVScroll        = 0x0115
	headerKeyDown        = 0x0100
	headerNMCustomDraw   = -12
	headerDrawPrepaint   = 0x00000001
	headerDrawItem       = 0x00010000
	headerDrawItemPaint  = headerDrawItem | headerDrawPrepaint
	headerNotifyItemDraw = 0x00000020
	headerSkipDefault    = 0x00000004
	headerTrackLeave     = 0x00000002
	hdmFirst             = 0x1200
	hdmGetItemRect       = hdmFirst + 7

	// processColumnCount is the number of list-view columns rendered by the
	// picker: process (0), description (1), instances (2). Every column loop and
	// boundary check must use this instead of a raw literal so the layout and
	// the header captions cannot drift apart.
	processColumnCount = 3
)

type headerTrackMouseEvent struct {
	Size      uint32
	Flags     uint32
	Track     windows.Handle
	HoverTime uint32
}

type headerCustomDraw struct {
	Header    nmHeader
	DrawStage uint32
	HDC       windows.Handle
	Rect      rect
	ItemSpec  uintptr
	ItemState uint32
	ItemParam uintptr
}

var (
	headerComctl32         = windows.NewLazySystemDLL("comctl32.dll")
	headerUser32           = windows.NewLazySystemDLL("user32.dll")
	pSetWindowSubclass     = headerComctl32.NewProc("SetWindowSubclass")
	pRemoveWindowSubclass  = headerComctl32.NewProc("RemoveWindowSubclass")
	pDefSubclassProc       = headerComctl32.NewProc("DefSubclassProc")
	pHeaderTrackMouseEvent = headerUser32.NewProc("TrackMouseEvent")
	headerSubclassCallback = windows.NewCallback(headerListSubclassProc)
)

// installHeaderRenderer keeps the native Header control and handles only its
// NM_CUSTOMDRAW colors through the ListView parent. This retains native hit
// testing and state notifications without replacing the first-frame WM_PAINT
// path that previously made captions depend on later mouse invalidation.
func (p *picker) installHeaderRenderer() error {
	list := p.controls[idList]
	if list == 0 {
		return fmt.Errorf("process picker list is required")
	}
	if ok, _, callErr := pSetWindowSubclass.Call(uintptr(list), headerSubclassCallback, headerListSubclassID, 0); ok == 0 {
		return fmt.Errorf("subclass process picker list: %w", callErr)
	}
	header, _, _ := pSendMessage.Call(uintptr(list), lvmGetHeader, 0, 0)
	if header == 0 {
		pRemoveWindowSubclass.Call(uintptr(list), headerSubclassCallback, headerListSubclassID)
		return fmt.Errorf("resolve process picker header")
	}
	p.header = windows.Handle(header)
	if ok, _, callErr := pSetWindowSubclass.Call(header, headerSubclassCallback, headerSubclassID, 0); ok == 0 {
		pRemoveWindowSubclass.Call(uintptr(list), headerSubclassCallback, headerListSubclassID)
		p.header = 0
		return fmt.Errorf("subclass process picker header: %w", callErr)
	}
	nativeform.ApplyControl(p.header, p.themeDark)
	nativeform.PresentControl(p.header, true)
	return nil
}

func (p *picker) releaseHeaderRenderer() {
	if p.header != 0 {
		pRemoveWindowSubclass.Call(uintptr(p.header), headerSubclassCallback, headerSubclassID)
	}
	if p.controls != nil && p.controls[idList] != 0 {
		pRemoveWindowSubclass.Call(uintptr(p.controls[idList]), headerSubclassCallback, headerListSubclassID)
	}
	p.header = 0
	p.headerHover = 0
	p.headerPressed = 0
}

func (p *picker) headerColumnAt(lParam uintptr) int {
	if p.header == 0 {
		return 0
	}
	x := int32(int16(uint16(lParam)))
	y := int32(int16(uint16(lParam >> 16)))
	for column := range processColumnCount {
		var bounds rect
		if ok, _, _ := pSendMessage.Call(uintptr(p.header), hdmGetItemRect, uintptr(column), uintptr(unsafe.Pointer(&bounds))); ok == 0 {
			continue
		}
		if x >= bounds.Left && x < bounds.Right && y >= bounds.Top && y < bounds.Bottom {
			return column + 1
		}
	}
	return 0
}

func (p *picker) trackHeaderMouse() {
	if p.header == 0 {
		return
	}
	value := headerTrackMouseEvent{
		Size:  uint32(unsafe.Sizeof(headerTrackMouseEvent{})),
		Flags: headerTrackLeave,
		Track: p.header,
	}
	pHeaderTrackMouseEvent.Call(uintptr(unsafe.Pointer(&value)))
}

func (p *picker) updateHeaderState(hover, pressed int) {
	if p.headerHover == hover && p.headerPressed == pressed {
		return
	}
	p.headerHover = hover
	p.headerPressed = pressed
	if p.header != 0 {
		pInvalidateRect.Call(uintptr(p.header), 0, 0)
	}
}

func (p *picker) paintHeaderRemainder() {
	if p.header == 0 {
		return
	}
	var client, last rect
	if ok, _, _ := pGetClientRect.Call(uintptr(p.header), uintptr(unsafe.Pointer(&client))); ok == 0 {
		return
	}
	if ok, _, _ := pSendMessage.Call(uintptr(p.header), hdmGetItemRect, 2, uintptr(unsafe.Pointer(&last))); ok == 0 || last.Right >= client.Right {
		return
	}
	dc, _, _ := pGetDC.Call(uintptr(p.header))
	if dc == 0 {
		return
	}
	nativeform.DrawTableHeaderCell(windows.Handle(dc), nativeform.Rect{Left: last.Right, Top: client.Top, Right: client.Right, Bottom: client.Bottom}, p.font, "", p.palette, nativeform.ControlState{}, p.scale())
	pReleaseDC.Call(uintptr(p.header), dc)
}

func (p *picker) drawHeaderCustom(value *headerCustomDraw) (uintptr, bool) {
	if value == nil || value.Header.HwndFrom != p.header || value.Header.Code != headerNMCustomDraw || value.HDC == 0 {
		return 0, false
	}
	bounds := nativeform.Rect{Left: value.Rect.Left, Top: value.Rect.Top, Right: value.Rect.Right, Bottom: value.Rect.Bottom}
	switch value.DrawStage {
	case headerDrawPrepaint:
		var client rect
		if ok, _, _ := pGetClientRect.Call(uintptr(p.header), uintptr(unsafe.Pointer(&client))); ok != 0 {
			bounds = nativeform.Rect{Left: client.Left, Top: client.Top, Right: client.Right, Bottom: client.Bottom}
		}
		nativeform.DrawTableHeaderCell(value.HDC, bounds, p.font, "", p.palette, nativeform.ControlState{}, p.scale())
		return headerNotifyItemDraw, true
	case headerDrawItemPaint:
		column := int(value.ItemSpec)
		if column < 0 || column >= processColumnCount {
			return headerSkipDefault, true
		}
		state := nativeform.ControlState{
			Hovered: p.headerHover == column+1,
			Pressed: p.headerPressed == column+1,
		}
		nativeform.DrawTableHeaderCell(value.HDC, bounds, p.font, p.headerCaption(column), p.palette, state, p.scale())
		return headerSkipDefault, true
	default:
		return 0, false
	}
}

func headerListSubclassProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr, subclassID, refData uintptr) uintptr {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p != nil && hwnd == p.header {
		switch message {
		case headerMouseMove:
			p.updateHeaderState(p.headerColumnAt(lParam), p.headerPressed)
			p.trackHeaderMouse()
		case headerMouseLeave:
			p.updateHeaderState(0, 0)
		case headerLButtonDown:
			column := p.headerColumnAt(lParam)
			p.updateHeaderState(column, column)
		case headerLButtonUp:
			result, _, _ := pDefSubclassProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
			p.updateHeaderState(p.headerColumnAt(lParam), 0)
			return result
		case headerCancelMode, headerCaptureChanged:
			p.updateHeaderState(p.headerHover, 0)
		case wmPaint:
			result, _, _ := pDefSubclassProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
			p.paintHeaderRemainder()
			return result
		}
	}
	if message == wmNotify && p != nil && hwnd == p.controls[idList] {
		if result, handled := p.drawHeaderCustom((*headerCustomDraw)(nativeform.MessagePointer(lParam))); handled {
			return result
		}
	}
	if message == headerMouseWheel {
		if p != nil && hwnd == p.controls[idList] && p.scrollListWheel(wParam) {
			return 0
		}
	}
	result, _, _ := pDefSubclassProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	switch message {
	case headerVScroll, headerKeyDown:
		if p != nil && hwnd == p.controls[idList] {
			p.syncListScrollbar()
		}
	}
	return result
}
