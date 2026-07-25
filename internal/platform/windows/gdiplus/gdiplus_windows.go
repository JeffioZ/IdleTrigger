//go:build windows

// Package gdiplus provides the small anti-aliased vector primitive used by
// native owner-drawn controls. It never owns an HDC or renders text.
package gdiplus

import (
	"sync"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	FillModeAlternate = 0
	FillModeWinding   = 1

	smoothingModeAntiAlias8x8 = 5
	pixelOffsetModeHalf       = 4
)

type Point struct{ X, Y int32 }

type roundedRect struct{ left, top, right, bottom int32 }

// DrawResult distinguishes a setup failure, for which GDI can safely draw on
// the existing HDC, from a final GDI+ call that may have changed pixels.
type DrawResult uint8

const (
	DrawNotStarted DrawResult = iota
	DrawCompleted
	DrawMayBeDirty
)

type startupInput struct {
	Version                  uint32
	DebugEventCallback       uintptr
	SuppressBackgroundThread uint32
	SuppressExternalCodecs   uint32
}

type startupOutput struct {
	NotificationHook   uintptr
	NotificationUnhook uintptr
}

type lifecycleSession struct {
	gdiplusToken       uintptr
	notificationToken  uintptr
	notificationUnhook uintptr
}

type lifecycleState uint8

const (
	stateNew lifecycleState = iota
	stateReady
	stateClosing
	stateClosed
	stateFailed
)

type lifecycleManager struct {
	mu       sync.Mutex
	changed  *sync.Cond
	state    lifecycleState
	session  lifecycleSession
	active   int
	startup  func() (lifecycleSession, bool)
	shutdown func(lifecycleSession)
}

func newLifecycle(startup func() (lifecycleSession, bool), shutdown func(lifecycleSession)) *lifecycleManager {
	m := &lifecycleManager{startup: startup, shutdown: shutdown}
	m.changed = sync.NewCond(&m.mu)
	return m
}

func (m *lifecycleManager) start() bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.state == stateReady {
		return true
	}
	if m.state != stateNew {
		return false
	}
	session, ok := m.startup()
	if !ok || session.gdiplusToken == 0 {
		m.state = stateFailed
		return false
	}
	m.session, m.state = session, stateReady
	return true
}

func (m *lifecycleManager) startDrawing() (func(), bool) {
	m.mu.Lock()
	if m.state != stateReady {
		m.mu.Unlock()
		return nil, false
	}
	m.active++
	m.mu.Unlock()
	var once sync.Once
	return func() {
		once.Do(func() {
			m.mu.Lock()
			m.active--
			if m.state == stateClosing && m.active == 0 {
				m.changed.Broadcast()
			}
			m.mu.Unlock()
		})
	}, true
}

func (m *lifecycleManager) close() {
	m.mu.Lock()
	for m.state == stateClosing {
		m.changed.Wait()
	}
	if m.state == stateClosed || m.state == stateFailed {
		m.mu.Unlock()
		return
	}
	if m.state == stateNew {
		m.state = stateClosed
		m.mu.Unlock()
		return
	}
	m.state = stateClosing
	for m.active != 0 {
		m.changed.Wait()
	}
	session := m.session
	m.session = lifecycleSession{}
	m.mu.Unlock()

	m.shutdown(session)

	m.mu.Lock()
	m.state = stateClosed
	m.changed.Broadcast()
	m.mu.Unlock()
}

type gdiAPI struct {
	createFromHDC      func(windows.Handle) (uintptr, bool)
	deleteGraphics     func(uintptr)
	setSmoothingMode   func(uintptr) bool
	setPixelOffsetMode func(uintptr) bool
	createSolidFill    func(uint32) (uintptr, bool)
	deleteBrush        func(uintptr)
	fillPolygon        func(uintptr, uintptr, []Point, int) bool
	createPath         func(int) (uintptr, bool)
	deletePath         func(uintptr)
	startPathFigure    func(uintptr) bool
	closePathFigure    func(uintptr) bool
	addPathLine        func(uintptr, int32, int32, int32, int32) bool
	addPathBezier      func(uintptr, int32, int32, int32, int32, int32, int32, int32, int32) bool
	fillPath           func(uintptr, uintptr, uintptr) bool
}

var (
	dll = windows.NewLazySystemDLL("gdiplus.dll")

	pStartup            = dll.NewProc("GdiplusStartup")
	pShutdown           = dll.NewProc("GdiplusShutdown")
	pCreateFromHDC      = dll.NewProc("GdipCreateFromHDC")
	pDeleteGraphics     = dll.NewProc("GdipDeleteGraphics")
	pSetSmoothingMode   = dll.NewProc("GdipSetSmoothingMode")
	pSetPixelOffsetMode = dll.NewProc("GdipSetPixelOffsetMode")
	pCreateSolidFill    = dll.NewProc("GdipCreateSolidFill")
	pDeleteBrush        = dll.NewProc("GdipDeleteBrush")
	pFillPolygonI       = dll.NewProc("GdipFillPolygonI")
	pCreatePath         = dll.NewProc("GdipCreatePath")
	pDeletePath         = dll.NewProc("GdipDeletePath")
	pStartPathFigure    = dll.NewProc("GdipStartPathFigure")
	pClosePathFigure    = dll.NewProc("GdipClosePathFigure")
	pAddPathLineI       = dll.NewProc("GdipAddPathLineI")
	pAddPathBezierI     = dll.NewProc("GdipAddPathBezierI")
	pFillPath           = dll.NewProc("GdipFillPath")

	lifecycle = newLifecycle(startup, shutdown)
	api       = gdiAPI{
		createFromHDC:      createFromHDC,
		deleteGraphics:     deleteGraphics,
		setSmoothingMode:   setSmoothingMode,
		setPixelOffsetMode: setPixelOffsetMode,
		createSolidFill:    createSolidFill,
		deleteBrush:        deleteBrush,
		fillPolygon:        fillPolygon,
		createPath:         createPath,
		deletePath:         deletePath,
		startPathFigure:    startPathFigure,
		closePathFigure:    closePathFigure,
		addPathLine:        addPathLine,
		addPathBezier:      addPathBezier,
		fillPath:           fillPath,
	}
)

func Start() bool { return lifecycle.start() }

func Shutdown() { lifecycle.close() }

// FillPolygon draws one compact anti-aliased vector icon. A caller must fully
// repaint its control in GDI when DrawMayBeDirty is returned.
func FillPolygon(hdc windows.Handle, points []Point, color uint32) DrawResult {
	return fillPolygonWith(lifecycle, api, hdc, points, color)
}

// DrawCheck preserves the existing checkbox footprint while rendering the
// diagonal mark as a filled anti-aliased polygon.
func DrawCheck(hdc windows.Handle, left, top, right, bottom int32, color uint32, width int32) DrawResult {
	w, h := right-left, bottom-top
	if w <= 0 || h <= 0 || width <= 0 {
		return DrawNotStarted
	}
	x1, y1 := left+w*24/100, top+h*51/100
	x2, y2 := left+w*43/100, top+h*70/100
	x3, y3 := left+w*77/100, top+h*30/100
	return FillPolygon(hdc, []Point{{X: x1 - width/2, Y: y1}, {X: x2, Y: y2 + width/2}, {X: x3 + width/2, Y: y3}, {X: x3, Y: y3 - width/2}, {X: x2, Y: y2 - width/2}, {X: x1 + width/2, Y: y1 - width/2}}, color)
}

// FillRoundedRect renders an anti-aliased rounded card using an integer Bezier
// path. border is retained by filling the outer path first, then a path inset
// by one physical pixel on each edge. The caller must fully repaint the card
// in GDI when DrawMayBeDirty is returned.
func FillRoundedRect(hdc windows.Handle, left, top, right, bottom, radius int32, fill, border uint32) DrawResult {
	return fillRoundedRectWith(lifecycle, api, hdc, roundedRect{left, top, right, bottom}, radius, fill, border)
}

func fillPolygonWith(m *lifecycleManager, a gdiAPI, hdc windows.Handle, points []Point, color uint32) DrawResult {
	if hdc == 0 || len(points) < 3 {
		return DrawNotStarted
	}
	end, ok := m.startDrawing()
	if !ok {
		return DrawNotStarted
	}
	defer end()
	graphics, ok := a.createFromHDC(hdc)
	if !ok {
		return DrawNotStarted
	}
	defer a.deleteGraphics(graphics)
	if !a.setSmoothingMode(graphics) || !a.setPixelOffsetMode(graphics) {
		return DrawNotStarted
	}
	brush, ok := a.createSolidFill(argb(color))
	if !ok {
		return DrawNotStarted
	}
	defer a.deleteBrush(brush)
	if !a.fillPolygon(graphics, brush, points, FillModeWinding) {
		return DrawMayBeDirty
	}
	return DrawCompleted
}

func fillRoundedRectWith(m *lifecycleManager, a gdiAPI, hdc windows.Handle, bounds roundedRect, radius int32, fill, border uint32) DrawResult {
	if hdc == 0 || bounds.right-bounds.left <= 2 || bounds.bottom-bounds.top <= 2 {
		return DrawNotStarted
	}
	end, ok := m.startDrawing()
	if !ok {
		return DrawNotStarted
	}
	defer end()
	graphics, ok := a.createFromHDC(hdc)
	if !ok {
		return DrawNotStarted
	}
	defer a.deleteGraphics(graphics)
	if !a.setSmoothingMode(graphics) || !a.setPixelOffsetMode(graphics) {
		return DrawNotStarted
	}
	borderBrush, ok := a.createSolidFill(argb(border))
	if !ok {
		return DrawNotStarted
	}
	defer a.deleteBrush(borderBrush)
	fillBrush, ok := a.createSolidFill(argb(fill))
	if !ok {
		return DrawNotStarted
	}
	defer a.deleteBrush(fillBrush)
	outerPath, ok := roundedRectPath(a, bounds, radius)
	if !ok {
		return DrawNotStarted
	}
	defer a.deletePath(outerPath)
	inner := roundedRect{bounds.left + 1, bounds.top + 1, bounds.right - 1, bounds.bottom - 1}
	if inner.right <= inner.left || inner.bottom <= inner.top {
		return DrawNotStarted
	}
	innerPath, ok := roundedRectPath(a, inner, radius-1)
	if !ok {
		return DrawNotStarted
	}
	defer a.deletePath(innerPath)
	if !a.fillPath(graphics, borderBrush, outerPath) || !a.fillPath(graphics, fillBrush, innerPath) {
		return DrawMayBeDirty
	}
	return DrawCompleted
}

func roundedRectPath(a gdiAPI, bounds roundedRect, radius int32) (uintptr, bool) {
	width, height := bounds.right-bounds.left, bounds.bottom-bounds.top
	if width <= 0 || height <= 0 {
		return 0, false
	}
	if radius < 0 {
		radius = 0
	}
	if radius > width/2 {
		radius = width / 2
	}
	if radius > height/2 {
		radius = height / 2
	}
	path, ok := a.createPath(FillModeWinding)
	if !ok {
		return 0, false
	}
	fail := func() (uintptr, bool) { a.deletePath(path); return 0, false }
	if !a.startPathFigure(path) {
		return fail()
	}
	if radius == 0 {
		if !a.addPathLine(path, bounds.left, bounds.top, bounds.right, bounds.top) ||
			!a.addPathLine(path, bounds.right, bounds.top, bounds.right, bounds.bottom) ||
			!a.addPathLine(path, bounds.right, bounds.bottom, bounds.left, bounds.bottom) {
			return fail()
		}
	} else {
		// 0.552 is the standard cubic approximation of a quarter circle. All
		// coordinates remain integer device pixels; no float syscall arguments
		// or GdipAddPathArc calls are used.
		offset := (radius*552 + 500) / 1000
		if !a.addPathLine(path, bounds.left+radius, bounds.top, bounds.right-radius, bounds.top) ||
			!a.addPathBezier(path, bounds.right-radius, bounds.top, bounds.right-radius+offset, bounds.top, bounds.right, bounds.top+radius-offset, bounds.right, bounds.top+radius) ||
			!a.addPathLine(path, bounds.right, bounds.top+radius, bounds.right, bounds.bottom-radius) ||
			!a.addPathBezier(path, bounds.right, bounds.bottom-radius, bounds.right, bounds.bottom-radius+offset, bounds.right-radius+offset, bounds.bottom, bounds.right-radius, bounds.bottom) ||
			!a.addPathLine(path, bounds.right-radius, bounds.bottom, bounds.left+radius, bounds.bottom) ||
			!a.addPathBezier(path, bounds.left+radius, bounds.bottom, bounds.left+radius-offset, bounds.bottom, bounds.left, bounds.bottom-radius+offset, bounds.left, bounds.bottom-radius) ||
			!a.addPathLine(path, bounds.left, bounds.bottom-radius, bounds.left, bounds.top+radius) ||
			!a.addPathBezier(path, bounds.left, bounds.top+radius, bounds.left, bounds.top+radius-offset, bounds.left+radius-offset, bounds.top, bounds.left+radius, bounds.top) {
			return fail()
		}
	}
	if !a.closePathFigure(path) {
		return fail()
	}
	return path, true
}

func startup() (lifecycleSession, bool) {
	if !loadRequiredAPI(
		dll.Load,
		pStartup.Find,
		pShutdown.Find,
		pCreateFromHDC.Find,
		pDeleteGraphics.Find,
		pSetSmoothingMode.Find,
		pSetPixelOffsetMode.Find,
		pCreateSolidFill.Find,
		pDeleteBrush.Find,
		pFillPolygonI.Find,
		pCreatePath.Find,
		pDeletePath.Find,
		pStartPathFigure.Find,
		pClosePathFigure.Find,
		pAddPathLineI.Find,
		pAddPathBezierI.Find,
		pFillPath.Find,
	) {
		return lifecycleSession{}, false
	}
	// The default GDI+ background thread owns a hidden "GDI+ Hook Window" at
	// (0,0). IdleTrigger already has one lifetime-long Win32 message loop, so use
	// the documented notification hook path instead and avoid creating that
	// auxiliary top-level window altogether.
	input := startupInput{Version: 1, SuppressBackgroundThread: 1}
	var output startupOutput
	var token uintptr
	status, _, _ := pStartup.Call(uintptr(unsafe.Pointer(&token)), uintptr(unsafe.Pointer(&input)), uintptr(unsafe.Pointer(&output)))
	if status != 0 || token == 0 || output.NotificationHook == 0 || output.NotificationUnhook == 0 {
		if token != 0 {
			_, _, _ = pShutdown.Call(token)
		}
		return lifecycleSession{}, false
	}
	var notificationToken uintptr
	status, _, _ = syscall.SyscallN(output.NotificationHook, uintptr(unsafe.Pointer(&notificationToken)))
	if status != 0 {
		_, _, _ = pShutdown.Call(token)
		return lifecycleSession{}, false
	}
	return lifecycleSession{
		gdiplusToken:       token,
		notificationToken:  notificationToken,
		notificationUnhook: output.NotificationUnhook,
	}, true
}

func loadRequiredAPI(load func() error, finders ...func() error) bool {
	if err := load(); err != nil {
		return false
	}
	for _, find := range finders {
		if err := find(); err != nil {
			return false
		}
	}
	return true
}

func shutdown(session lifecycleSession) {
	if session.notificationUnhook != 0 {
		_, _, _ = syscall.SyscallN(session.notificationUnhook, session.notificationToken)
	}
	if session.gdiplusToken != 0 {
		_, _, _ = pShutdown.Call(session.gdiplusToken)
	}
}

func createFromHDC(hdc windows.Handle) (uintptr, bool) {
	var graphics uintptr
	status, _, _ := pCreateFromHDC.Call(uintptr(hdc), uintptr(unsafe.Pointer(&graphics)))
	return graphics, status == 0 && graphics != 0
}

func deleteGraphics(graphics uintptr) { _, _, _ = pDeleteGraphics.Call(graphics) }

func setSmoothingMode(graphics uintptr) bool {
	status, _, _ := pSetSmoothingMode.Call(graphics, smoothingModeAntiAlias8x8)
	return status == 0
}

func setPixelOffsetMode(graphics uintptr) bool {
	status, _, _ := pSetPixelOffsetMode.Call(graphics, pixelOffsetModeHalf)
	return status == 0
}

func createSolidFill(color uint32) (uintptr, bool) {
	var brush uintptr
	status, _, _ := pCreateSolidFill.Call(uintptr(color), uintptr(unsafe.Pointer(&brush)))
	return brush, status == 0 && brush != 0
}

func deleteBrush(brush uintptr) { _, _, _ = pDeleteBrush.Call(brush) }

func fillPolygon(graphics, brush uintptr, points []Point, fillMode int) bool {
	status, _, _ := pFillPolygonI.Call(graphics, brush, uintptr(unsafe.Pointer(&points[0])), uintptr(len(points)), uintptr(fillMode))
	return status == 0
}

func createPath(fillMode int) (uintptr, bool) {
	var path uintptr
	status, _, _ := pCreatePath.Call(uintptr(fillMode), uintptr(unsafe.Pointer(&path)))
	return path, status == 0 && path != 0
}

func deletePath(path uintptr) { _, _, _ = pDeletePath.Call(path) }

func startPathFigure(path uintptr) bool {
	status, _, _ := pStartPathFigure.Call(path)
	return status == 0
}

func closePathFigure(path uintptr) bool {
	status, _, _ := pClosePathFigure.Call(path)
	return status == 0
}

func addPathLine(path uintptr, x1, y1, x2, y2 int32) bool {
	status, _, _ := pAddPathLineI.Call(path, uintptr(x1), uintptr(y1), uintptr(x2), uintptr(y2))
	return status == 0
}

func addPathBezier(path uintptr, x1, y1, x2, y2, x3, y3, x4, y4 int32) bool {
	status, _, _ := pAddPathBezierI.Call(path, uintptr(x1), uintptr(y1), uintptr(x2), uintptr(y2), uintptr(x3), uintptr(y3), uintptr(x4), uintptr(y4))
	return status == 0
}

func fillPath(graphics, brush, path uintptr) bool {
	status, _, _ := pFillPath.Call(graphics, brush, path)
	return status == 0
}

func argb(color uint32) uint32 {
	return 0xff000000 | ((color & 0x000000ff) << 16) | (color & 0x0000ff00) | ((color & 0x00ff0000) >> 16)
}
