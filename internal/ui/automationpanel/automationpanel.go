// Package automationpanel implements the native automatic-task manager and
// editor. It uses the existing compact palette and system UI fonts; business
// state is returned to the app through one save callback.
package automationpanel

import (
	"fmt"
	"runtime"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/processpicker"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

type State struct {
	Rules    []automation.Rule
	Issues   []automation.RuleIssue
	Revision string
	Chinese  bool
	Owner    windows.Handle
	NextText string
}

type SaveRequest struct {
	BaseRevision string
	Rules        []automation.Rule
}

type SaveResult struct {
	State State
	Error string
}

type OnSave func(SaveRequest) SaveResult
type TextFunc func(string) string

type rect struct{ Left, Top, Right, Bottom int32 }
type point struct{ X, Y int32 }
type wndClassEx struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}
type toolInfo struct {
	Size     uint32
	Flags    uint32
	Hwnd     windows.Handle
	ID       uintptr
	Rect     rect
	Instance windows.Handle
	Text     *uint16
	LParam   uintptr
	Reserved uintptr
}
type drawItem struct {
	CtlType, CtlID, ItemID, ItemAction, ItemState uint32
	HwndItem, HDC                                 windows.Handle
	Rect                                          rect
	ItemData                                      uintptr
}

type view uint8

const (
	managerView view = iota
	editorView
)

type panel struct {
	hwnd                windows.Handle
	state               State
	onSave              OnSave
	text                TextFunc
	view                view
	controls            map[uint16]windows.Handle
	anonymous           []windows.Handle
	tooltip             windows.Handle
	tooltipText         map[uint16][]uint16
	font                windows.Handle
	sectionFont         windows.Handle
	windowBrush         windows.Handle
	surfaceBrush        windows.Handle
	disabledBrush       windows.Handle
	palette             colors.Palette
	themeDark           bool
	style, exStyle      uintptr
	ownerDisabled       bool
	captureHost         bool
	captureScale        float64
	themeOverride       *bool
	icons               nativeform.WindowIcons
	rules               []automation.Rule
	editing             int
	draft               automation.Rule
	originalDraft       automation.Rule
	processDescriptions map[string]string
	selectedRuleID      string
	listTopIndex        int
	triggerOptions      []automation.Trigger
	labels              map[uint16]string
	bounds              map[uint16]logicalBounds
	surfaces            nativeform.ControlSurfaceSet
	choices             map[uint16]*choiceField
	checks              map[uint16]bool
	choiceOpen          uint16
	choicePopup         *nativeform.ChoicePopup
	interaction         nativeform.InteractionTracker
	clientWidth         int
	clientHeight        int
	viewportWidth       int
	viewportHeight      int
	contentOffset       int
	dpiScale            float64
	pendingSuggested    *nativeform.Rect
	layoutWorkArea      *nativeform.Rect
	layoutErr           error
	layoutingEditor     bool
	editorStatusError   bool
	managerReady        bool
	editorReady         bool
	rebuildSuspended    bool
	rebuildVisibility   map[uint16]bool
	firstFramePending   bool
	managerScroll       *nativeform.ListboxScrollbar
	contentScroll       *nativeform.Scrollbar
	pendingState        *State
	managerNotice       string
	layoutBatch         uintptr
	layoutVisibility    map[uint16]bool
}

type logicalBounds struct{ X, Y, Width, Height int }

type choiceField struct {
	labels   []string
	selected int
}

const (
	windowClass         = "IdleTriggerAutomationPanel"
	managerWidth        = 600
	managerHeight       = 380
	editorWidth         = 680
	editorHeight        = 520
	idTitle             = 100
	idList              = 102
	idNext              = 103
	idNew               = 104
	idEdit              = 105
	idToggle            = 106
	idDelete            = 107
	idEmptyTitle        = 108
	idEmptyBody         = 109
	idListSurface       = 110
	idName              = 200
	idAction            = 201
	idTrigger           = 202
	idDate              = 203
	idTime              = 204
	idEndTime           = 205
	idProcessLogic      = 207
	idChooseProcesses   = 208
	idProcessSummary    = 209
	idKeepScreen        = 210
	idIdleMinutes       = 211
	idWarningSeconds    = 212
	idBlockedPolicy     = 213
	idMaxWait           = 214
	idSave              = 215
	idCancel            = 216
	idBasicsTitle       = 217
	idTriggerTitle      = 218
	idOptionsTitle      = 219
	idNameLabel         = 220
	idActionLabel       = 221
	idTriggerLabel      = 222
	idDateLabel         = 223
	idTimeLabel         = 224
	idEndTimeLabel      = 225
	idDaysLabel         = 226
	idProcessLogicLabel = 227
	idIdleMinutesLabel  = 228
	idWarningLabel      = 229
	idBlockedLabel      = 230
	idMaxWaitLabel      = 231
	idNoOptions         = 232
	idNameHint          = 233
	idRuntimeNote       = 234
	idValidation        = 235
	idProcessInfo       = 236
	idDaysWorkdays      = 237
	idDaysEveryday      = 238
	idWeekdayBase       = 240
	idFieldSurfaceBase  = 500
	formTextHeight      = 18
	formLabelGap        = 4
	formRelatedGap      = 8
	formContentGap      = 12
	formSectionGap      = 16
	formEdgePadding     = 16
	checkboxRowHeight   = 28
	processSummaryRowH  = 22
	wmDestroy           = 0x0002
	wmPaint             = 0x000F
	wmClose             = 0x0010
	wmEraseBkgnd        = 0x0014
	wmDrawItem          = 0x002B
	wmCommand           = 0x0111
	wmCtlColorStatic    = 0x0138
	wmCtlColorEdit      = 0x0133
	wmCtlColorList      = 0x0134
	wmCtlColorButton    = 0x0135
	wmSettingChange     = 0x001A
	wmSysColorChange    = 0x0015
	wmThemeChanged      = 0x031A
	wmDpiChanged        = 0x02E0
	wmSetFont           = 0x0030
	wmLButtonDown       = 0x0201
	wmMouseWheel        = 0x020A
	wmKeyDown           = 0x0100
	wmNCDestroy         = 0x0082
	wmOpenChoice        = 0x8001
	wmPrewarmEditor     = 0x8002
	wsOverlapped        = 0
	wsPopup             = 0x80000000
	wsCaption           = 0x00C00000
	wsSysMenu           = 0x00080000
	wsThickFrame        = 0x00040000
	wsMinimizeBox       = 0x00020000
	wsMaximizeBox       = 0x00010000
	wsClipChildren      = 0x02000000
	wsClipSiblings      = 0x04000000
	wsChild             = 0x40000000
	wsVisible           = 0x10000000
	wsTabStop           = 0x00010000
	wsVScroll           = 0x00200000
	wsExTopmost         = 0x00000008
	wsExAppWindow       = 0x00040000
	esAutoHScroll       = 0x0080
	esNumber            = 0x2000
	lbsNotify           = 0x0001
	lbsNoIntegralHeight = 0x0100
	bsOwnerDraw         = 0x0000000B
	ssLeft              = 0
	ssOwnerDraw         = 0x0000000D
	formSurfaceStyle    = wsChild | wsClipSiblings | ssOwnerDraw
	lbAddString         = 0x0180
	lbResetContent      = 0x0184
	lbGetCurSel         = 0x0188
	lbSetCurSel         = 0x0186
	lbGetTopIndex       = 0x018E
	lbSetTopIndex       = 0x0197
	wmGetTextLength     = 0x000E
	wmGetText           = 0x000D
	emSetMargins        = 0x00D3
	emSetSel            = 0x00B1
	bnClicked           = 0
	lbnDblClk           = 2
	lbnSelChange        = 1
	ttfIDIsHwnd         = 0x0001
	ttfSubclass         = 0x0010
	ttmAddTool          = 0x0432
	ttmDelTool          = 0x0433
	ttmSetMaxTipWidth   = 0x0418
	ttsAlwaysTip        = 0x01
	ttsNoPrefix         = 0x02
	bnSetFocus          = 6
	enSetFocus          = 0x0100
	enChange            = 0x0300
	lbnSetFocus         = 4
	transparent         = 1
	opaque              = 2
	swpNoZOrder         = 0x0004
	swpNoSize           = 0x0001
	swpNoMove           = 0x0002
	swpNoActivate       = 0x0010
	odsSelected         = 0x0001
	odsDisabled         = 0x0004
	odsFocus            = 0x0010
	vkLeft              = 0x25
	vkRight             = 0x27
	vkHome              = 0x24
	vkEnd               = 0x23
	weekdaySubclassID   = 0x49545744
)

var (
	user32               = windows.NewLazySystemDLL("user32.dll")
	gdi32                = windows.NewLazySystemDLL("gdi32.dll")
	pCreateWindowEx      = user32.NewProc("CreateWindowExW")
	pDestroyWindow       = user32.NewProc("DestroyWindow")
	pDefWindowProc       = user32.NewProc("DefWindowProcW")
	pRegisterClassEx     = user32.NewProc("RegisterClassExW")
	pSendMessage         = user32.NewProc("SendMessageW")
	pSetWindowText       = user32.NewProc("SetWindowTextW")
	pPostMessage         = user32.NewProc("PostMessageW")
	pSetWindowPos        = user32.NewProc("SetWindowPos")
	pBeginDeferWindowPos = user32.NewProc("BeginDeferWindowPos")
	pDeferWindowPos      = user32.NewProc("DeferWindowPos")
	pEndDeferWindowPos   = user32.NewProc("EndDeferWindowPos")
	pShowWindow          = user32.NewProc("ShowWindow")
	pEnableWindow        = user32.NewProc("EnableWindow")
	pIsWindow            = user32.NewProc("IsWindow")
	pIsWindowEnabled     = user32.NewProc("IsWindowEnabled")
	pIsWindowVisible     = user32.NewProc("IsWindowVisible")
	pSetForeground       = user32.NewProc("SetForegroundWindow")
	pSetFocus            = user32.NewProc("SetFocus")
	pGetClientRect       = user32.NewProc("GetClientRect")
	pFillRect            = user32.NewProc("FillRect")
	pInvalidateRect      = user32.NewProc("InvalidateRect")
	pGetDpiForWindow     = user32.NewProc("GetDpiForWindow")
	pSetTextColor        = gdi32.NewProc("SetTextColor")
	pSetBkColor          = gdi32.NewProc("SetBkColor")
	pSetBkMode           = gdi32.NewProc("SetBkMode")
	pCreateBrush         = gdi32.NewProc("CreateSolidBrush")
	pDeleteObject        = gdi32.NewProc("DeleteObject")
	pSetWindowSubclass   = windows.NewLazySystemDLL("comctl32.dll").NewProc("SetWindowSubclass")
	pDefSubclassProc     = windows.NewLazySystemDLL("comctl32.dll").NewProc("DefSubclassProc")
	classOnce            sync.Once
	classErr             error
	activeMu             sync.Mutex
	active               *panel
	wndCallback          = windows.NewCallback(wndProc)
	weekdayCallback      = windows.NewCallback(weekdayButtonProc)
)

func Show(state State, onSave OnSave, text TextFunc) error {
	activeMu.Lock()
	if active != nil && active.hwnd != 0 {
		activeMu.Unlock()
		Focus()
		return nil
	}
	activeMu.Unlock()
	if text == nil {
		text = func(key string) string { return key }
	}
	if err := ensureClass(); err != nil {
		return err
	}
	state = cloneState(state)
	p := &panel{state: state, onSave: onSave, text: text, controls: make(map[uint16]windows.Handle), rules: append([]automation.Rule(nil), state.Rules...), editing: -1}
	activeMu.Lock()
	active = p
	activeMu.Unlock()
	if err := p.create(); err != nil {
		activeMu.Lock()
		if active == p {
			active = nil
		}
		activeMu.Unlock()
		return err
	}
	return nil
}

// Focus brings the deepest automatic-task dialog to the foreground. It lets a
// tray click preserve modal ownership instead of closing the disabled parent.
func Focus() bool {
	if processpicker.Focus() {
		return true
	}
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd == 0 {
		return false
	}
	pSetForeground.Call(uintptr(p.hwnd))
	return true
}

func Hide() {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p != nil && p.hwnd != 0 {
		pDestroyWindow.Call(uintptr(p.hwnd))
	}
}

// Update publishes the latest authoritative rule state to an open manager.
// The caller must run on the tray UI thread. Unsaved editor input is preserved
// and marked stale instead of being overwritten.
func Update(state State) {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd == 0 {
		return
	}
	p.updateState(cloneState(state))
}

func cloneState(state State) State {
	state.Rules = append([]automation.Rule(nil), state.Rules...)
	state.Issues = append([]automation.RuleIssue(nil), state.Issues...)
	return state
}

// Capture hosts the real manager or editor for deterministic devtools visual
// checks without starting the tray or reading user configuration.
func Capture(state State, text TextFunc, scale float64, dark, editor bool, capture func(windows.Handle) error) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restoreTextScale := font.OverrideTextScaleFactor(1)
	defer restoreTextScale()
	if text == nil {
		text = func(key string) string { return key }
	}
	if err := ensureClass(); err != nil {
		return err
	}
	state.Owner = 0
	p := &panel{
		state: state, text: text, controls: make(map[uint16]windows.Handle),
		rules: append([]automation.Rule(nil), state.Rules...), editing: -1,
		captureHost: true, captureScale: scale, themeOverride: &dark,
	}
	activeMu.Lock()
	if active != nil {
		activeMu.Unlock()
		return fmt.Errorf("automation panel already active")
	}
	active = p
	activeMu.Unlock()
	if err := p.create(); err != nil {
		activeMu.Lock()
		if active == p {
			active = nil
		}
		activeMu.Unlock()
		return err
	}
	defer func() {
		if p.hwnd != 0 {
			pDestroyWindow.Call(uintptr(p.hwnd))
		}
	}()
	if editor {
		index := -1
		if len(p.rules) > 0 {
			index = 0
		}
		p.showEditor(index)
		// A hidden screenshot host may report the window as not visible during
		// the immediate manager-to-editor transition. Restore the explicit
		// capture state after the transition so PrintWindow never receives an
		// otherwise valid but hidden editor frame.
		pShowWindow.Call(uintptr(p.hwnd), 5)
	}
	if !editor {
		for _, id := range []uint16{idNew, idEdit, idToggle, idDelete} {
			control := p.controls[id]
			if control == 0 {
				return fmt.Errorf("automation capture control %d was not created", id)
			}
			if visible, _, _ := pIsWindowVisible.Call(uintptr(control)); visible == 0 {
				return fmt.Errorf("automation capture control %d is not visible", id)
			}
		}
	}
	if capture == nil {
		return nil
	}
	p.repaint()
	return capture(p.hwnd)
}

func ensureClass() error {
	classOnce.Do(func() {
		name, err := windows.UTF16PtrFromString(windowClass)
		if err != nil {
			classErr = err
			return
		}
		wc := wndClassEx{Size: uint32(unsafe.Sizeof(wndClassEx{})), WndProc: wndCallback, ClassName: name}
		result, _, callErr := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc)))
		if result == 0 && callErr != windows.ERROR_CLASS_ALREADY_EXISTS {
			classErr = fmt.Errorf("register automation panel: %w", callErr)
		}
	})
	return classErr
}

func (p *panel) create() error {
	p.initializeControlState()
	class, _ := windows.UTF16PtrFromString(windowClass)
	title, _ := windows.UTF16PtrFromString(p.t("automation_title"))
	p.style = wsPopup | wsCaption | wsSysMenu | wsClipChildren
	// Keep native EDIT controls on their normal paint path. The first-frame gate
	// provides atomic presentation without top-level descendant compositing.
	p.exStyle = wsExTopmost
	if p.captureHost {
		p.style = wsOverlapped | wsCaption | wsSysMenu | wsThickFrame | wsMinimizeBox | wsMaximizeBox | wsClipChildren
		p.exStyle = wsExAppWindow
	}
	creationX, creationY := nativeform.InitialWindowPoint(p.state.Owner)
	hwnd, _, callErr := pCreateWindowEx.Call(
		p.exStyle, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(title)), p.style,
		uintptr(creationX), uintptr(creationY), 1, 1, uintptr(p.state.Owner), 0, 0, 0,
	)
	if hwnd == 0 {
		return fmt.Errorf("create automation panel: %w", callErr)
	}
	p.hwnd = windows.Handle(hwnd)
	firstFrame := nativeform.BeginFirstFrame(p.hwnd)
	p.dpiScale = p.windowScale()
	scale := p.scale()
	p.font, _ = font.New(int32(14*scale+0.5), 400, p.state.Chinese)
	p.sectionFont, _ = font.New(int32(14*scale+0.5), 600, p.state.Chinese)
	p.applyTheme()
	contentScroll, scrollErr := nativeform.NewScrollbar(nativeform.ScrollbarOptions{
		Parent: p.hwnd, Palette: p.palette, Background: p.palette.WindowBackground, Scale: scale,
		OnChange: p.scrollContentTo,
	})
	if scrollErr != nil {
		pDestroyWindow.Call(hwnd)
		return scrollErr
	}
	p.contentScroll = contentScroll
	p.firstFramePending = true
	p.showManager()
	if p.layoutErr != nil {
		err := p.layoutErr
		pDestroyWindow.Call(hwnd)
		return err
	}
	p.firstFramePending = false
	if !p.captureHost && p.state.Owner != 0 {
		if enabled, _, _ := pIsWindowEnabled.Call(uintptr(p.state.Owner)); enabled != 0 {
			pEnableWindow.Call(uintptr(p.state.Owner), 0)
			p.ownerDisabled = true
		}
	}
	p.surfaces.PrepareCues()
	if err := firstFrame.Reveal(nativeform.FirstFrameOptions{
		RepeatShow: p.captureHost,
		Controls:   p.frameControls(),
	}); err != nil {
		pDestroyWindow.Call(hwnd)
		return err
	}
	if !p.captureHost {
		pSetForeground.Call(hwnd)
		trayicon.SetTabNavigationWindow(p.hwnd, func() { p.interaction.SetFocusVisible(true) })
		// Let the manager commit its first frame before constructing the hidden
		// editor controls. The queued prewarm removes the first-New delay without
		// making the manager itself wait for a second page of controls.
		pPostMessage.Call(hwnd, wmPrewarmEditor, 0, 0)
	}
	return nil
}

func (p *panel) initializeControlState() {
	p.controls = make(map[uint16]windows.Handle)
	p.anonymous = nil
	p.tooltip = 0
	p.tooltipText = nil
	p.labels = make(map[uint16]string)
	p.bounds = make(map[uint16]logicalBounds)
	p.surfaces = nativeform.ControlSurfaceSet{}
	p.choices = make(map[uint16]*choiceField)
	p.checks = make(map[uint16]bool)
	p.choiceOpen = 0
	p.managerReady = false
	p.editorReady = false
}

func (p *panel) updateState(state State) {
	if state.Revision == p.state.Revision {
		p.state.NextText = state.NextText
		p.state.Issues = append([]automation.RuleIssue(nil), state.Issues...)
		if p.view == managerView {
			p.setText(idNext, p.managerStatusText())
			p.populateRules()
		}
		return
	}

	selectedID := p.selectedRuleID
	if p.view == editorView {
		p.syncDraft()
		selectedID = p.draft.ID
		if editorHasAnyChanges(p.originalDraft, p.draft) {
			pending := cloneState(state)
			p.pendingState = &pending
			p.setEditorError(idSave, p.t("automation_changed_external"))
			return
		}
	}

	p.managerNotice = ""
	p.acceptState(state)
	if p.view == managerView {
		p.showManager()
		return
	}
	for index := range p.rules {
		if p.rules[index].ID == selectedID {
			p.showEditor(index)
			return
		}
	}
	p.showManager()
}

func (p *panel) acceptState(state State) {
	p.state = cloneState(state)
	p.rules = append([]automation.Rule(nil), state.Rules...)
	p.pendingState = nil
}

func (p *panel) issueForRule(index int) (automation.RuleIssue, bool) {
	for _, issue := range p.state.Issues {
		if issue.Index == index {
			return issue, true
		}
	}
	return automation.RuleIssue{}, false
}

func (p *panel) managerStatusText() string {
	if p.managerNotice != "" {
		return p.managerNotice
	}
	return p.state.NextText
}
