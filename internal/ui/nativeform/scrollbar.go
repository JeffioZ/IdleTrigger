package nativeform

import (
	"fmt"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

type ScrollbarOptions struct {
	Horizontal bool
	Parent     windows.Handle
	Palette    colors.Palette
	Background uint32
	Scale      float64
	OnChange   func(int)
}

// Scrollbar is a compact themed vertical scrollbar used where native
// non-client scrollbars cannot be colored consistently across Windows builds.
// The owning list or popup retains keyboard/wheel behavior and supplies the
// logical total/page/position values.
type Scrollbar struct {
	horizontal            bool
	hwnd                  windows.Handle
	palette               colors.Palette
	background            uint32
	scale                 float64
	total, page, position int
	onChange              func(int)
	hovered, thumbHovered bool
	pressed               bool
	dragging              bool
	dragOffset            int32
	visible               bool
	closed                bool
}

type scrollbarRect struct{ Left, Top, Right, Bottom int32 }
type scrollbarPoint struct{ X, Y int32 }
type scrollbarPaint struct {
	DC                         windows.Handle
	Erase                      int32
	Paint                      scrollbarRect
	Restore, IncrementalUpdate int32
	Reserved                   [32]byte
}
type scrollbarTrackMouse struct {
	Size      uint32
	Flags     uint32
	Track     windows.Handle
	HoverTime uint32
}
type scrollbarWndClass struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}

const (
	scrollbarClass     = "IdleTriggerThemedScrollbar"
	sbWMDestroy        = 0x0002
	sbWMCancelMode     = 0x001F
	sbWMPaint          = 0x000F
	sbWMEraseBkgnd     = 0x0014
	sbWMPrintClient    = 0x0318
	sbWMMouseMove      = 0x0200
	sbWMLButtonDown    = 0x0201
	sbWMLButtonUp      = 0x0202
	sbWMMouseWheel     = 0x020A
	sbWMMouseLeave     = 0x02A3
	sbWMCaptureChanged = 0x0215
	sbWSChild          = 0x40000000
	sbWSClipSiblings   = 0x04000000
	sbSWShow           = 5
	sbSWHide           = 0
	sbSWPNoSize        = 0x0001
	sbSWPNoMove        = 0x0002
	sbSWPNoActivate    = 0x0010
	sbTMELeave         = 0x00000002
	sbCursorArrow      = 32512
)

var (
	scrollbarUser32   = windows.NewLazySystemDLL("user32.dll")
	sbCreateWindow    = scrollbarUser32.NewProc("CreateWindowExW")
	sbDestroyWindow   = scrollbarUser32.NewProc("DestroyWindow")
	sbDefWindowProc   = scrollbarUser32.NewProc("DefWindowProcW")
	sbRegisterClass   = scrollbarUser32.NewProc("RegisterClassExW")
	sbSetWindowPos    = scrollbarUser32.NewProc("SetWindowPos")
	sbShowWindow      = scrollbarUser32.NewProc("ShowWindow")
	sbInvalidateRect  = scrollbarUser32.NewProc("InvalidateRect")
	sbGetClientRect   = scrollbarUser32.NewProc("GetClientRect")
	sbBeginPaint      = scrollbarUser32.NewProc("BeginPaint")
	sbEndPaint        = scrollbarUser32.NewProc("EndPaint")
	sbSetCapture      = scrollbarUser32.NewProc("SetCapture")
	sbReleaseCapture  = scrollbarUser32.NewProc("ReleaseCapture")
	sbTrackMouse      = scrollbarUser32.NewProc("TrackMouseEvent")
	sbLoadCursor      = scrollbarUser32.NewProc("LoadCursorW")
	scrollbarOnce     sync.Once
	scrollbarErr      error
	scrollbarCallback = windows.NewCallback(scrollbarWndProc)
	scrollbarMu       sync.Mutex
	scrollbars        = make(map[windows.Handle]*Scrollbar)
)

func NewScrollbar(options ScrollbarOptions) (*Scrollbar, error) {
	if options.Parent == 0 {
		return nil, fmt.Errorf("scrollbar parent is required")
	}
	if options.Scale <= 0 {
		options.Scale = 1
	}
	scrollbarOnce.Do(func() {
		name, _ := windows.UTF16PtrFromString(scrollbarClass)
		cursor, _, _ := sbLoadCursor.Call(0, sbCursorArrow)
		class := scrollbarWndClass{Size: uint32(unsafe.Sizeof(scrollbarWndClass{})), WndProc: scrollbarCallback, Cursor: windows.Handle(cursor), ClassName: name}
		registered, _, err := sbRegisterClass.Call(uintptr(unsafe.Pointer(&class)))
		if registered == 0 && err != windows.ERROR_CLASS_ALREADY_EXISTS {
			scrollbarErr = fmt.Errorf("register themed scrollbar: %w", err)
		}
	})
	if scrollbarErr != nil {
		return nil, scrollbarErr
	}
	name, _ := windows.UTF16PtrFromString(scrollbarClass)
	hwnd, _, err := sbCreateWindow.Call(0, uintptr(unsafe.Pointer(name)), 0, sbWSChild|sbWSClipSiblings, 0, 0, 1, 1, uintptr(options.Parent), 0, 0, 0)
	if hwnd == 0 {
		return nil, fmt.Errorf("create themed scrollbar: %w", err)
	}
	bar := &Scrollbar{
		hwnd: windows.Handle(hwnd), palette: options.Palette, background: options.Background,
		scale: options.Scale, onChange: options.OnChange, horizontal: options.Horizontal,
	}
	scrollbarMu.Lock()
	scrollbars[bar.hwnd] = bar
	scrollbarMu.Unlock()
	return bar, nil
}

func (s *Scrollbar) Window() windows.Handle {
	if s == nil {
		return 0
	}
	return s.hwnd
}

func (s *Scrollbar) Close() {
	if s != nil && s.hwnd != 0 && !s.closed {
		sbDestroyWindow.Call(uintptr(s.hwnd))
	}
}

func (s *Scrollbar) SetBounds(x, y, width, height int) {
	if s == nil || s.hwnd == 0 {
		return
	}
	// Keep the overlay above the native list or popup content. Some common
	// controls repaint outside sibling bounds unless the z-order is explicit.
	sbSetWindowPos.Call(uintptr(s.hwnd), 0, uintptr(x), uintptr(y), uintptr(max(1, width)), uintptr(max(1, height)), sbSWPNoActivate)
}

func (s *Scrollbar) SetTheme(palette colors.Palette, background uint32) {
	if s == nil || (s.palette == palette && s.background == background) {
		return
	}
	s.palette, s.background = palette, background
	if s.hwnd != 0 {
		sbInvalidateRect.Call(uintptr(s.hwnd), 0, 0)
	}
}

func (s *Scrollbar) SetScale(scale float64) {
	if s == nil {
		return
	}
	if scale <= 0 {
		scale = 1
	}
	if s.scale == scale {
		return
	}
	s.scale = scale
	if s.hwnd != 0 {
		sbInvalidateRect.Call(uintptr(s.hwnd), 0, 0)
	}
}

func (s *Scrollbar) SetMetrics(total, page, position int) {
	if s == nil || s.hwnd == 0 {
		return
	}
	s.total = max(0, total)
	s.page = max(0, min(page, s.total))
	s.position = max(0, min(position, s.maxPosition()))
	visible := s.total > s.page && s.page > 0
	if visible != s.visible {
		s.visible = visible
		command := uintptr(sbSWHide)
		if visible {
			command = sbSWShow
		}
		sbShowWindow.Call(uintptr(s.hwnd), command)
	}
	if visible {
		// Lists can be raised after their overlay is constructed (for example
		// when a rounded surface and its native child are finalized). Reassert
		// the overlay z-order whenever metrics are synchronized so a valid thumb
		// cannot remain hidden behind the list until the user interacts with it.
		sbSetWindowPos.Call(uintptr(s.hwnd), 0, 0, 0, 0, 0, sbSWPNoMove|sbSWPNoSize|sbSWPNoActivate)
		sbInvalidateRect.Call(uintptr(s.hwnd), 0, 0)
	}
}

func (s *Scrollbar) maxPosition() int { return max(0, s.total-s.page) }

func scrollbarThumb(total, page, position int, trackTop, trackBottom, minimum int32) (int32, int32) {
	height := trackBottom - trackTop
	if height <= 0 {
		return trackTop, trackTop
	}
	if total <= 0 || page <= 0 || total <= page {
		return trackTop, trackBottom
	}
	thumbHeight := int32(int64(height) * int64(page) / int64(total))
	thumbHeight = max(minimum, min(height, thumbHeight))
	maxPosition := total - page
	position = max(0, min(position, maxPosition))
	travel := height - thumbHeight
	top := trackTop
	if travel > 0 && maxPosition > 0 {
		top += int32(int64(travel) * int64(position) / int64(maxPosition))
	}
	return top, top + thumbHeight
}

func (s *Scrollbar) geometry(height int32) (track, thumb Rect) {
	inset := scaledPixels(2, s.scale)
	track = Rect{Left: inset, Top: inset, Right: scaledPixels(ScrollbarWidth, s.scale) - inset, Bottom: height - inset}
	if track.Right <= track.Left {
		track.Right = track.Left + 2
	}
	minimum := scaledPixels(ScrollbarMinThumb, s.scale)
	thumb.Top, thumb.Bottom = scrollbarThumb(s.total, s.page, s.position, track.Top, track.Bottom, minimum)
	thumb.Left, thumb.Right = track.Left, track.Right
	return track, thumb
}

func (s *Scrollbar) setPosition(position int) {
	position = max(0, min(position, s.maxPosition()))
	if position == s.position {
		return
	}
	s.position = position
	sbInvalidateRect.Call(uintptr(s.hwnd), 0, 0)
	if s.onChange != nil {
		s.onChange(position)
	}
}

func (s *Scrollbar) positionFromThumb(y int32, height int32) int {
	track, thumb := s.geometry(height)
	travel := (track.Bottom - track.Top) - (thumb.Bottom - thumb.Top)
	if travel <= 0 || s.maxPosition() == 0 {
		return 0
	}
	top := max(track.Top, min(y-s.dragOffset, track.Top+travel))
	return int(int64(top-track.Top) * int64(s.maxPosition()) / int64(travel))
}

func (s *Scrollbar) draw(target windows.Handle, bounds Rect) {
	paint := func(dc windows.Handle, local Rect) {
		fillRect(dc, local, s.background)
		length := local.Bottom
		if s.horizontal {
			length = local.Right
		}
		track, thumb := s.geometry(length)
		radius := (track.Right - track.Left) / 2
		if s.horizontal {
			track = transposeScrollRect(track)
			thumb = transposeScrollRect(thumb)
		}
		trackColor := s.palette.DisabledSurface
		thumbColor := s.palette.Border
		if s.hovered {
			trackColor = s.palette.HoverSurface
		}
		if s.thumbHovered {
			thumbColor = s.palette.SecondaryText
		}
		if s.pressed || s.dragging {
			thumbColor = s.palette.AccentPressed
		}
		DrawSurface(dc, track, s.palette, s.background, trackColor, trackColor, radius)
		DrawSurface(dc, thumb, s.palette, s.background, thumbColor, thumbColor, radius)
	}
	if !DrawBuffered(target, bounds, paint) {
		paint(target, bounds)
	}
}

func scrollbarWndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	scrollbarMu.Lock()
	s := scrollbars[hwnd]
	scrollbarMu.Unlock()
	if s == nil {
		result, _, _ := sbDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
		return result
	}
	point := scrollbarPoint{X: int32(int16(lParam)), Y: int32(int16(lParam >> 16))}
	if s.horizontal {
		point.X, point.Y = point.Y, point.X
	}
	switch message {
	case sbWMPaint:
		var paint scrollbarPaint
		dc, _, _ := sbBeginPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&paint)))
		var client scrollbarRect
		sbGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		bounds := Rect(client)
		s.draw(windows.Handle(dc), bounds)
		sbEndPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&paint)))
		return 0
	case sbWMPrintClient:
		var client scrollbarRect
		sbGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		s.draw(windows.Handle(wParam), Rect(client))
		return 0
	case sbWMEraseBkgnd:
		return 1
	case sbWMMouseMove:
		var client scrollbarRect
		sbGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		if s.dragging {
			s.setPosition(s.positionFromThumb(point.Y, s.trackLength(client)))
		} else {
			_, thumb := s.geometry(s.trackLength(client))
			hovered := true
			thumbHovered := point.Y >= thumb.Top && point.Y < thumb.Bottom
			if hovered != s.hovered || thumbHovered != s.thumbHovered {
				s.hovered, s.thumbHovered = hovered, thumbHovered
				sbInvalidateRect.Call(uintptr(hwnd), 0, 0)
			}
			track := scrollbarTrackMouse{Size: uint32(unsafe.Sizeof(scrollbarTrackMouse{})), Flags: sbTMELeave, Track: hwnd}
			sbTrackMouse.Call(uintptr(unsafe.Pointer(&track)))
		}
		return 0
	case sbWMMouseLeave:
		if !s.dragging && (s.hovered || s.thumbHovered || s.pressed) {
			s.hovered, s.thumbHovered, s.pressed = false, false, false
			sbInvalidateRect.Call(uintptr(hwnd), 0, 0)
		}
		return 0
	case sbWMMouseWheel:
		delta := int16(wParam >> 16)
		if delta > 0 {
			s.setPosition(s.position - 1)
		} else if delta < 0 {
			s.setPosition(s.position + 1)
		}
		return 0
	case sbWMLButtonDown:
		var client scrollbarRect
		sbGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		_, thumb := s.geometry(s.trackLength(client))
		s.pressed = true
		if point.Y >= thumb.Top && point.Y < thumb.Bottom {
			s.dragging = true
			s.dragOffset = point.Y - thumb.Top
			sbSetCapture.Call(uintptr(hwnd))
		} else if point.Y < thumb.Top {
			s.setPosition(s.position - max(1, s.page))
		} else {
			s.setPosition(s.position + max(1, s.page))
		}
		sbInvalidateRect.Call(uintptr(hwnd), 0, 0)
		return 0
	case sbWMLButtonUp, sbWMCancelMode, sbWMCaptureChanged:
		wasCaptured := s.dragging
		s.dragging, s.pressed = false, false
		if wasCaptured && message == sbWMLButtonUp {
			sbReleaseCapture.Call()
		}
		sbInvalidateRect.Call(uintptr(hwnd), 0, 0)
		return 0
	case sbWMDestroy:
		s.closed, s.visible, s.hwnd = true, false, 0
		scrollbarMu.Lock()
		delete(scrollbars, hwnd)
		scrollbarMu.Unlock()
		return 0
	}
	result, _, _ := sbDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}

func transposeScrollRect(r Rect) Rect {
	return Rect{Left: r.Top, Top: r.Left, Right: r.Bottom, Bottom: r.Right}
}
func (s *Scrollbar) trackLength(r scrollbarRect) int32 {
	if s.horizontal {
		return r.Right
	}
	return r.Bottom
}
