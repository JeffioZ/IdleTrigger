// Package processpicker implements the bounded native process-selection
// window used by automatic tasks. PIDs are display-time metadata only and are
// never returned to callers.
package processpicker

import (
	"fmt"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

type Options struct {
	Owner        windows.Handle
	Selected     []automation.ProcessTarget
	Descriptions map[string]string
	Chinese      bool
	Text         func(string) string
	OnConfirm    func([]automation.ProcessTarget, map[string]string)
}

type rect struct{ Left, Top, Right, Bottom int32 }
type logicalBounds struct{ X, Y, Width, Height int }
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

type item struct {
	target                   automation.ProcessTarget
	name, description, count string
	search                   string
}

type initCommonControlsEx struct {
	Size uint32
	ICC  uint32
}

type lvColumn struct {
	Mask    uint32
	Fmt     int32
	Cx      int32
	Text    *uint16
	TextMax int32
	SubItem int32
	Image   int32
	Order   int32
}

type lvItem struct {
	Mask      uint32
	Item      int32
	SubItem   int32
	State     uint32
	StateMask uint32
	Text      *uint16
	TextMax   int32
	Image     int32
	LParam    uintptr
}

type nmHeader struct {
	HwndFrom windows.Handle
	IDFrom   uintptr
	Code     int32
}

type nmListView struct {
	Header             nmHeader
	Item, SubItem      int32
	NewState, OldState uint32
	Changed            uint32
	Action             struct{ X, Y int32 }
	LParam             uintptr
}

type nmLVGetInfoTip struct {
	Header  nmHeader
	Flags   uint32
	Text    *uint16
	TextMax int32
	Item    int32
	SubItem int32
	LParam  uintptr
}

type lvHitTestInfo struct {
	Point         struct{ X, Y int32 }
	Flags         uint32
	Item, SubItem int32
	Group         int32
}

type openFileName struct {
	Size             uint32
	Owner            windows.Handle
	Instance         windows.Handle
	Filter           *uint16
	CustomFilter     *uint16
	MaxCustomFilter  uint32
	FilterIndex      uint32
	File             *uint16
	MaxFile          uint32
	FileTitle        *uint16
	MaxFileTitle     uint32
	InitialDirectory *uint16
	Title            *uint16
	Flags            uint32
	FileOffset       uint16
	FileExtension    uint16
	DefaultExtension *uint16
	CustomData       uintptr
	Hook             uintptr
	TemplateName     *uint16
	Reserved         unsafe.Pointer
	ReservedSize     uint32
	FlagsEx          uint32
}

type picker struct {
	hwnd             windows.Handle
	options          Options
	controls         map[uint16]windows.Handle
	bounds           map[uint16]logicalBounds
	labels           map[uint16]string
	surfaces         nativeform.ControlSurfaceSet
	font             windows.Handle
	windowBrush      windows.Handle
	surfaceBrush     windows.Handle
	palette          colors.Palette
	themeDark        bool
	icons            nativeform.WindowIcons
	style            uintptr
	exStyle          uintptr
	tooltip          windows.Handle
	tooltipText      [][]uint16
	items            []item
	visible          []item
	selected         map[string]automation.ProcessTarget
	descriptions     map[string]string
	descriptionTried map[string]struct{}
	sortColumn       int
	sortAscending    bool
	populating       bool
	captureHost      bool
	captureScale     float64
	themeOverride    *bool
	generation       uint64
	viewPhase        pickerViewPhase
	loadMode         processLoadMode
	loadInFlight     bool
	closed           atomic.Bool
	cleanupOnce      sync.Once
	workers          sync.WaitGroup
	interaction      nativeform.InteractionTracker
	stateImages      windows.Handle
	header           windows.Handle
	headerHover      int
	headerPressed    int
	scrollbar        *nativeform.Scrollbar
	previewScroll    *nativeform.ListboxScrollbar
	contentScroll    *nativeform.Scrollbar
	lastSnapshot     time.Time
	lastScanDuration time.Duration
	statusHoldUntil  time.Time
	pendingStatus    string
	listWheelDelta   int
	anchorTopKey     string
	anchorFocusKey   string
	ownerDisabled    bool
	createErr        error
	layoutErr        error
	dpiScale         float64
	contentOffset    int
	viewportWidth    int
	viewportHeight   int
	pendingSuggested *nativeform.Rect
	layoutBatch      uintptr
}

const (
	windowClass           = "IdleTriggerProcessPicker"
	windowWidth           = 700
	pickerTopPadding      = 16
	pickerBottomPadding   = 16
	pickerTextHeight      = 18
	pickerLabelGap        = 4
	pickerRelatedGap      = 8
	pickerTextOpticalBias = 1
	pickerFieldHeight     = 34
	pickerListHeight      = 180
	pickerPreviewHeight   = 60
	pickerHeadingY        = pickerTopPadding
	pickerHelperY         = pickerHeadingY + pickerTextHeight + pickerLabelGap
	pickerSearchY         = pickerHelperY + pickerTextHeight + pickerRelatedGap
	pickerListY           = pickerSearchY + pickerFieldHeight + pickerRelatedGap
	pickerStatusY         = pickerListY + pickerListHeight + pickerRelatedGap
	pickerPreviewTitleY   = pickerStatusY + pickerTextHeight + pickerRelatedGap
	pickerPreviewY        = pickerPreviewTitleY + pickerTextHeight + pickerLabelGap
	// GDI STATIC reserves slightly more descent below its visible glyphs. Shift
	// the text box up by 1 px while keeping the combined footer gaps at 16 px.
	pickerPrivacyY        = pickerPreviewY + pickerPreviewHeight + pickerRelatedGap - pickerTextOpticalBias
	pickerButtonsY        = pickerPrivacyY + pickerTextHeight + pickerRelatedGap + pickerTextOpticalBias
	windowHeight          = pickerButtonsY + nativeform.ButtonHeight + pickerBottomPadding
	processScrollbarLane  = 24
	idSearch              = 101
	idRefresh             = 102
	idList                = 103
	idStatus              = 104
	idPrivacy             = 105
	idCancel              = 106
	idConfirm             = 107
	idPreviewTitle        = 108
	idPreview             = 109
	idEmpty               = 110
	idBrowse              = 111
	idHeading             = 112
	idHelper              = 113
	idSearchSurface       = 114
	idListSurface         = 115
	idPreviewSurface      = 116
	wmDestroy             = 0x0002
	wmActivate            = 0x0006
	wmPaint               = 0x000F
	wmClose               = 0x0010
	wmEraseBkgnd          = 0x0014
	wmDrawItem            = 0x002B
	wmCommand             = 0x0111
	wmNotify              = 0x004E
	wmTimer               = 0x0113
	wmMouseWheel          = 0x020A
	wmSetRedraw           = 0x000B
	wmCtlColorStatic      = 0x0138
	wmCtlColorEdit        = 0x0133
	wmCtlColorList        = 0x0134
	wmSettingChange       = 0x001A
	wmSysColorChange      = 0x0015
	wmThemeChanged        = 0x031A
	wmDpiChanged          = 0x02E0
	wmSetFont             = 0x0030
	wsOverlapped          = 0x00000000
	wsPopup               = 0x80000000
	wsCaption             = 0x00C00000
	wsSysMenu             = 0x00080000
	wsThickFrame          = 0x00040000
	wsMinimizeBox         = 0x00020000
	wsMaximizeBox         = 0x00010000
	wsClipChildren        = 0x02000000
	wsClipSiblings        = 0x04000000
	wsChild               = 0x40000000
	wsVisible             = 0x10000000
	wsTabStop             = 0x00010000
	wsVScroll             = 0x00200000
	wsExTopmost           = 0x00000008
	wsExAppWindow         = 0x00040000
	esAutoHScroll         = 0x0080
	lbsNoSelection        = 0x4000
	lbsNoIntegralHeight   = 0x0100
	lvsReport             = 0x0001
	lvsSingleSel          = 0x0004
	lvsShowSelAlways      = 0x0008
	bsOwnerDraw           = 0x0000000B
	ssLeft                = 0
	ssOwnerDraw           = 0x0000000D
	formSurfaceStyle      = wsChild | wsClipSiblings | ssOwnerDraw
	lbAddString           = 0x0180
	lbResetContent        = 0x0184
	lvmFirst              = 0x1000
	lvmSetBackgroundColor = lvmFirst + 1
	lvmSetImageList       = lvmFirst + 3
	lvmDeleteItem         = lvmFirst + 8
	lvmDeleteAllItems     = lvmFirst + 9
	lvmRedrawItems        = lvmFirst + 21
	lvmGetHeader          = lvmFirst + 31
	lvmGetColumnWidth     = lvmFirst + 29
	lvmGetTopIndex        = lvmFirst + 39
	lvmGetCountPerPage    = lvmFirst + 40
	lvmGetItemRect        = lvmFirst + 14
	lvmScroll             = lvmFirst + 20
	lvmGetItemState       = lvmFirst + 44
	lvmSetItemState       = lvmFirst + 43
	lvmEnsureVisible      = lvmFirst + 19
	lvmSubItemHitTest     = lvmFirst + 57
	lvmInsertItemW        = lvmFirst + 77
	lvmGetStringWidthW    = lvmFirst + 87
	lvmSetColumnW         = lvmFirst + 96
	lvmSetItemTextW       = lvmFirst + 116
	lvmInsertColumnW      = lvmFirst + 97
	lvmSetColumnWidth     = lvmFirst + 30
	lvmSetExtendedStyle   = lvmFirst + 54
	lvmSetTextColor       = lvmFirst + 36
	lvmSetTextBackground  = lvmFirst + 38
	lvsExCheckboxes       = 0x00000004
	lvsExFullRowSelect    = 0x00000020
	lvsExInfoTip          = 0x00000400
	lvsExDoubleBuffer     = 0x00010000
	lvifText              = 0x00000001
	lvifState             = 0x00000008
	lvisStateImageMask    = 0x0000F000
	lvisFocused           = 0x00000001
	lvisSelected          = 0x00000002
	lvnItemChanged        = -101
	lvnColumnClick        = -108
	lvnGetInfoTipW        = -158
	lvnSetFocus           = -7
	nmClick               = -2
	lvhtOnItemLabel       = 0x0004
	lvhtOnItemStateIcon   = 0x0008
	wmGetTextLength       = 0x000E
	wmGetText             = 0x000D
	enChange              = 0x0300
	enSetFocus            = 0x0100
	bnClicked             = 0
	bnSetFocus            = 6
	ttfIDIsHwnd           = 0x0001
	ttfSubclass           = 0x0010
	ttmAddTool            = 0x0432
	ttmSetMaxTipWidth     = 0x0418
	ttsAlwaysTip          = 0x01
	ttsNoPrefix           = 0x02
	opaque                = 2
	sbVert                = 1
	odsSelected           = 0x0001
	odsFocus              = 0x0010
	odsDisabled           = 0x0004
	lvsilState            = 2
	ilcColor32            = 0x00000020
	swpNoZOrder           = 0x0004
	swpNoMove             = 0x0002
	swpNoSize             = 0x0001
	swpNoActivate         = 0x0010
	searchTimerID         = 1
	statusTimerID         = 2
	ofnHideReadOnly       = 0x00000004
	ofnNoChangeDir        = 0x00000008
	ofnPathMustExist      = 0x00000800
	ofnFileMustExist      = 0x00001000
	ofnExplorer           = 0x00080000
	ofnDontAddToRecent    = 0x02000000
)

const (
	processPickerAutoRefreshMinAge  = 15 * time.Second
	processPickerAutoRefreshMaxAge  = 60 * time.Second
	processPickerScanCostMultiplier = 250
	processPickerManualStatusHold   = 1500 * time.Millisecond
)

type processLoadMode uint8

const (
	processLoadInitial processLoadMode = iota
	processLoadAutomatic
	processLoadManual
)

type pickerViewPhase uint8

const (
	pickerViewInitial pickerViewPhase = iota
	pickerViewLoading
	pickerViewEnriching
	pickerViewReady
	pickerViewError
)

var (
	user32                = windows.NewLazySystemDLL("user32.dll")
	gdi32                 = windows.NewLazySystemDLL("gdi32.dll")
	pCreateWindowEx       = user32.NewProc("CreateWindowExW")
	pDestroyWindow        = user32.NewProc("DestroyWindow")
	pDefWindowProc        = user32.NewProc("DefWindowProcW")
	pRegisterClassEx      = user32.NewProc("RegisterClassExW")
	pSendMessage          = user32.NewProc("SendMessageW")
	pSetWindowText        = user32.NewProc("SetWindowTextW")
	pSetWindowPos         = user32.NewProc("SetWindowPos")
	pBeginDeferWindowPos  = user32.NewProc("BeginDeferWindowPos")
	pDeferWindowPos       = user32.NewProc("DeferWindowPos")
	pEndDeferWindowPos    = user32.NewProc("EndDeferWindowPos")
	pShowWindow           = user32.NewProc("ShowWindow")
	pSetForeground        = user32.NewProc("SetForegroundWindow")
	pEnableWindow         = user32.NewProc("EnableWindow")
	pIsWindow             = user32.NewProc("IsWindow")
	pIsWindowEnabled      = user32.NewProc("IsWindowEnabled")
	pGetDpiForWindow      = user32.NewProc("GetDpiForWindow")
	pGetWindowRect        = user32.NewProc("GetWindowRect")
	pShowScrollBar        = user32.NewProc("ShowScrollBar")
	pGetClientRect        = user32.NewProc("GetClientRect")
	pFillRect             = user32.NewProc("FillRect")
	pInvalidateRect       = user32.NewProc("InvalidateRect")
	pSetTimer             = user32.NewProc("SetTimer")
	pKillTimer            = user32.NewProc("KillTimer")
	pGetDC                = user32.NewProc("GetDC")
	pReleaseDC            = user32.NewProc("ReleaseDC")
	pSetTextColor         = gdi32.NewProc("SetTextColor")
	pSetBkColor           = gdi32.NewProc("SetBkColor")
	pSetBkMode            = gdi32.NewProc("SetBkMode")
	pCreateBrush          = gdi32.NewProc("CreateSolidBrush")
	pDeleteObject         = gdi32.NewProc("DeleteObject")
	pCreateCompatibleDC   = gdi32.NewProc("CreateCompatibleDC")
	pDeleteDC             = gdi32.NewProc("DeleteDC")
	pCreateBitmap         = gdi32.NewProc("CreateCompatibleBitmap")
	pSelectObject         = gdi32.NewProc("SelectObject")
	comdlg32              = windows.NewLazySystemDLL("comdlg32.dll")
	pGetOpenFileName      = comdlg32.NewProc("GetOpenFileNameW")
	kernel32              = windows.NewLazySystemDLL("kernel32.dll")
	pGetBinaryType        = kernel32.NewProc("GetBinaryTypeW")
	comctl                = windows.NewLazySystemDLL("comctl32.dll")
	pInitCommonControls   = comctl.NewProc("InitCommonControlsEx")
	pImageListCreate      = comctl.NewProc("ImageList_Create")
	pImageListAdd         = comctl.NewProc("ImageList_Add")
	pImageListDestroy     = comctl.NewProc("ImageList_Destroy")
	classOnce             sync.Once
	classErr              error
	activeMu              sync.Mutex
	active                *picker
	nextGeneration        atomic.Uint64
	wndCallback           = windows.NewCallback(wndProc)
	createWindowForPicker = func(exStyle uintptr, class, caption *uint16, style uintptr, x, y, width, height int, parent windows.Handle, id uintptr) (windows.Handle, error) {
		hwnd, _, callErr := pCreateWindowEx.Call(
			exStyle, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(caption)), style,
			uintptr(x), uintptr(y), uintptr(width), uintptr(height), uintptr(parent), id, 0, 0,
		)
		if hwnd == 0 {
			return 0, callErr
		}
		return windows.Handle(hwnd), nil
	}
	newFontForPicker            = font.New
	newCueBannerForPicker       = nativeform.NewCueBanner
	newScrollbarForPicker       = nativeform.NewScrollbar
	newListboxScrollForPicker   = nativeform.NewListboxScrollbar
	postPickerUI                = trayicon.Post
	snapshotNamesForPicker      = processcatalog.SnapshotNames
	enrichDescriptionsForPicker = processcatalog.EnrichDescriptions
	fileDescriptionForPicker    = processcatalog.FileDescription
	createBrushForPicker        = func(color uint32) windows.Handle {
		brush, _, _ := pCreateBrush.Call(uintptr(color))
		return windows.Handle(brush)
	}
)

func Show(options Options) error {
	activeMu.Lock()
	if active != nil && active.hwnd != 0 {
		activeMu.Unlock()
		Focus()
		return nil
	}
	activeMu.Unlock()
	if options.Text == nil {
		options.Text = func(key string) string { return key }
	}
	if err := ensureClass(); err != nil {
		return err
	}
	p := &picker{
		options: options, controls: make(map[uint16]windows.Handle), bounds: make(map[uint16]logicalBounds),
		labels:   make(map[uint16]string),
		selected: make(map[string]automation.ProcessTarget), descriptions: make(map[string]string), descriptionTried: make(map[string]struct{}), sortAscending: true,
	}
	for key, value := range options.Descriptions {
		p.descriptions[key] = value
	}
	for _, target := range automation.NormalizeTargets(options.Selected) {
		p.selected[target.Key()] = target
	}
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

// Focus brings the active picker to the foreground and reports whether one is
// open. The automation manager uses it to preserve nested modal behavior.
func Focus() bool {
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
		p.destroy()
	}
}

// Capture creates the real picker with deterministic process groups for
// devtools visual checks. It never scans the host process list.
func Capture(options Options, groups []processcatalog.Group, scale float64, dark bool, capture func(windows.Handle) error) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	if options.Text == nil {
		options.Text = func(key string) string { return key }
	}
	if err := ensureClass(); err != nil {
		return err
	}
	options.Owner = 0
	p := &picker{
		options: options, controls: make(map[uint16]windows.Handle), bounds: make(map[uint16]logicalBounds),
		labels:   make(map[uint16]string),
		selected: make(map[string]automation.ProcessTarget), descriptions: make(map[string]string), descriptionTried: make(map[string]struct{}), sortAscending: true,
		captureHost: true, captureScale: scale, themeOverride: &dark,
	}
	for key, value := range options.Descriptions {
		p.descriptions[key] = value
	}
	for _, target := range automation.NormalizeTargets(options.Selected) {
		p.selected[target.Key()] = target
	}
	activeMu.Lock()
	if active != nil {
		activeMu.Unlock()
		return fmt.Errorf("process picker already active")
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
			p.destroy()
		}
	}()
	p.items = buildItems(groups, p.selected, options.Text)
	p.viewPhase = pickerViewReady
	p.applyFilter()
	if capture == nil {
		return nil
	}
	p.repaint()
	return capture(p.hwnd)
}

func (p *picker) repaint() {
	if p.hwnd == 0 {
		return
	}
	p.surfaces.PrepareCues()
	nativeform.PresentFrame(p.hwnd, p.frameControls()...)
}

func (p *picker) frameControls() []windows.Handle {
	controls := make([]windows.Handle, 0, len(p.controls))
	for _, control := range p.controls {
		if control != 0 {
			controls = append(controls, control)
		}
	}
	if p.header != 0 {
		controls = append(controls, p.header)
	}
	return controls
}

func (p *picker) abortCreate() {
	p.closed.Store(true)
	p.generation = nextGeneration.Add(1)
	if p.hwnd != 0 {
		if destroyed, _, _ := pDestroyWindow.Call(uintptr(p.hwnd)); destroyed != 0 {
			return
		}
	}
	p.releaseResources()
}

func (p *picker) destroy() {
	if p == nil {
		return
	}
	p.closed.Store(true)
	p.generation = nextGeneration.Add(1)
	if p.hwnd != 0 {
		if destroyed, _, _ := pDestroyWindow.Call(uintptr(p.hwnd)); destroyed != 0 {
			return
		}
	}
	p.releaseResources()
}

func (p *picker) releaseResources() {
	p.cleanupOnce.Do(func() {
		p.closed.Store(true)
		p.generation = nextGeneration.Add(1)
		hwnd := p.hwnd
		if hwnd != 0 {
			pKillTimer.Call(uintptr(hwnd), searchTimerID)
			pKillTimer.Call(uintptr(hwnd), statusTimerID)
		}
		p.releaseHeaderRenderer()
		p.surfaces.Close()
		p.interaction.Release()
		if p.scrollbar != nil {
			p.scrollbar.Close()
			p.scrollbar = nil
		}
		if p.previewScroll != nil {
			p.previewScroll.Close()
			p.previewScroll = nil
		}
		if p.contentScroll != nil {
			p.contentScroll.Close()
			p.contentScroll = nil
		}
		p.releaseStateImages()
		if p.tooltip != 0 {
			pDestroyWindow.Call(uintptr(p.tooltip))
			p.tooltip = 0
		}
		if hwnd != 0 {
			trayicon.ClearTabNavigationWindow(hwnd)
		}
		if p.font != 0 {
			pDeleteObject.Call(uintptr(p.font))
			p.font = 0
		}
		p.releaseBrushes()
		p.icons.Release()
		if p.ownerDisabled && p.options.Owner != 0 {
			if valid, _, _ := pIsWindow.Call(uintptr(p.options.Owner)); valid != 0 {
				pEnableWindow.Call(uintptr(p.options.Owner), 1)
				pSetForeground.Call(uintptr(p.options.Owner))
			}
			p.ownerDisabled = false
		}
		activeMu.Lock()
		if active == p {
			active = nil
		}
		activeMu.Unlock()
		p.hwnd = 0
		p.controls = nil
		p.bounds = nil
		p.tooltipText = nil
	})
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
			classErr = fmt.Errorf("register process picker: %w", callErr)
		}
	})
	return classErr
}

func (p *picker) create() (err error) {
	defer func() {
		if err != nil {
			p.abortCreate()
		}
	}()
	class, err := windows.UTF16PtrFromString(windowClass)
	if err != nil {
		return err
	}
	title, err := windows.UTF16PtrFromString(p.text("process_picker_title"))
	if err != nil {
		return err
	}
	p.style = wsPopup | wsCaption | wsSysMenu | wsClipChildren
	// Native EDIT and Header controls must remain on their own normal paint
	// path. Top-level WS_EX_COMPOSITED can discard their text on the first DWM
	// frame; list/preview updates are committed explicitly by PresentControl.
	p.exStyle = wsExTopmost
	if p.captureHost {
		p.style = wsOverlapped | wsCaption | wsSysMenu | wsThickFrame | wsMinimizeBox | wsMaximizeBox | wsClipChildren
		p.exStyle = wsExAppWindow
	}
	creationX, creationY := nativeform.InitialWindowPoint(p.options.Owner)
	hwnd, callErr := createWindowForPicker(p.exStyle, class, title, p.style, creationX, creationY, 1, 1, p.options.Owner, 0)
	if callErr != nil {
		return fmt.Errorf("create process picker: %w", callErr)
	}
	p.hwnd = hwnd
	firstFrame := nativeform.BeginFirstFrame(p.hwnd)
	p.dpiScale = p.windowScale()
	scale := p.scale()
	p.font, _ = newFontForPicker(int32(14*scale+0.5), 400, p.options.Chinese)
	if p.font == 0 {
		return fmt.Errorf("create process picker font")
	}
	p.applyTheme()
	if p.windowBrush == 0 || p.surfaceBrush == 0 {
		return fmt.Errorf("create process picker brushes")
	}
	init := initCommonControlsEx{Size: uint32(unsafe.Sizeof(initCommonControlsEx{})), ICC: 0x00000001}
	if ok, _, callErr := pInitCommonControls.Call(uintptr(unsafe.Pointer(&init))); ok == 0 {
		return fmt.Errorf("initialize process picker controls: %w", callErr)
	}
	p.child("STATIC", p.text("process_picker_heading"), wsChild|wsVisible|ssLeft, 18, pickerHeadingY, 664, pickerTextHeight, idHeading)
	p.child("STATIC", p.text("process_picker_helper"), wsChild|wsVisible|ssLeft, 18, pickerHelperY, 664, pickerTextHeight, idHelper)
	searchSurface := p.child("STATIC", "", formSurfaceStyle|wsVisible, 18, pickerSearchY, 370, pickerFieldHeight, idSearchSurface)
	pSetWindowPos.Call(uintptr(searchSurface), 1, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
	search := p.child("EDIT", "", wsChild|wsVisible|wsTabStop|wsClipSiblings|esAutoHScroll, 20, pickerSearchY+7, 366, 20, idSearch)
	if p.createErr != nil {
		return p.createErr
	}
	if _, err := p.surfaces.Add(nativeform.ControlSurfaceOptions{
		ControlID: idSearch, SurfaceID: idSearchSurface, Control: search, Surface: searchSurface,
		CueText: p.text("process_picker_search_hint"), CueColor: p.palette.MutedText, Scale: p.scale(),
		Tracker: &p.interaction, NewCue: newCueBannerForPicker,
	}); err != nil {
		return err
	}
	p.child("BUTTON", p.text("process_picker_refresh"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 396, pickerSearchY, 132, pickerFieldHeight, idRefresh)
	p.child("BUTTON", p.text("process_picker_browse"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 536, pickerSearchY, 146, pickerFieldHeight, idBrowse)
	listSurface := p.child("STATIC", "", formSurfaceStyle|wsVisible, 18, pickerListY, 664, pickerListHeight, idListSurface)
	pSetWindowPos.Call(uintptr(listSurface), 1, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
	list := p.child("SysListView32", "", wsChild|wsVisible|wsTabStop|wsClipChildren|wsClipSiblings|lvsReport|lvsSingleSel|lvsShowSelAlways, 20, pickerListY+2, 660, pickerListHeight-4, idList)
	if p.createErr != nil {
		return p.createErr
	}
	if _, err := p.surfaces.Add(nativeform.ControlSurfaceOptions{
		ControlID: idList, SurfaceID: idListSurface, Control: list, Surface: listSurface, Tracker: &p.interaction,
	}); err != nil {
		return err
	}
	pSendMessage.Call(uintptr(list), lvmSetExtendedStyle, 0, lvsExCheckboxes|lvsExFullRowSelect|lvsExInfoTip|lvsExDoubleBuffer)
	p.createColumns()
	p.resizeColumns()
	p.updateHeaderLabels()
	if err := p.installHeaderRenderer(); err != nil {
		return err
	}
	p.applyListTheme()
	if err := p.applyStateImages(); err != nil {
		return err
	}
	scrollbar, scrollErr := newScrollbarForPicker(nativeform.ScrollbarOptions{
		Parent: p.hwnd, Palette: p.palette, Background: p.palette.Surface, Scale: p.scale(),
		OnChange: p.scrollListTo,
	})
	if scrollErr != nil {
		return scrollErr
	}
	p.scrollbar = scrollbar
	p.syncListScrollbarBounds()
	p.syncListScrollbar()
	p.resizeColumns()
	p.child("STATIC", p.text("process_picker_loading"), wsChild|ssLeft, 42, pickerListY+(pickerListHeight-pickerTextHeight)/2, 616, pickerTextHeight, idEmpty)
	p.child("STATIC", p.text("process_picker_loading"), wsChild|wsVisible|ssLeft, 18, pickerStatusY, 664, pickerTextHeight, idStatus)
	p.child("STATIC", p.text("process_picker_selection_title"), wsChild|wsVisible|ssLeft, 18, pickerPreviewTitleY, 664, pickerTextHeight, idPreviewTitle)
	previewSurface := p.child("STATIC", "", formSurfaceStyle|wsVisible, 18, pickerPreviewY, 664, pickerPreviewHeight, idPreviewSurface)
	pSetWindowPos.Call(uintptr(previewSurface), 1, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
	preview := p.child("LISTBOX", "", wsChild|wsVisible|wsVScroll|wsClipSiblings|lbsNoSelection|lbsNoIntegralHeight, 20, pickerPreviewY+2, 660, pickerPreviewHeight-4, idPreview)
	if p.createErr != nil {
		return p.createErr
	}
	if _, err := p.surfaces.Add(nativeform.ControlSurfaceOptions{
		ControlID: idPreview, SurfaceID: idPreviewSurface, Control: preview, Surface: previewSurface,
	}); err != nil {
		return err
	}
	previewScroll, previewErr := newListboxScrollForPicker(nativeform.ListboxScrollbarOptions{
		Parent: p.hwnd, Listbox: preview, Palette: p.palette, Background: p.palette.Surface, Scale: p.scale(),
	})
	if previewErr != nil {
		return previewErr
	}
	p.previewScroll = previewScroll
	p.syncPreviewScrollbarBounds()
	p.child("STATIC", p.text("process_picker_privacy"), wsChild|wsVisible|ssLeft, 18, pickerPrivacyY, 664, pickerTextHeight, idPrivacy)
	p.child("BUTTON", p.text("common_cancel"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 462, pickerButtonsY, 106, nativeform.ButtonHeight, idCancel)
	p.child("BUTTON", p.text("process_picker_confirm"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 576, pickerButtonsY, 106, nativeform.ButtonHeight, idConfirm)
	if p.createErr != nil {
		return p.createErr
	}
	contentScroll, contentScrollErr := newScrollbarForPicker(nativeform.ScrollbarOptions{
		Parent: p.hwnd, Palette: p.palette, Background: p.palette.WindowBackground, Scale: p.scale(),
		OnChange: p.scrollContentTo,
	})
	if contentScrollErr != nil {
		return contentScrollErr
	}
	p.contentScroll = contentScroll
	for _, id := range []uint16{idSearch, idList, idPreview} {
		pSetWindowPos.Call(uintptr(p.controls[id]), 0, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
	}
	p.updatePreview()
	if err := p.createTooltips(); err != nil {
		return err
	}
	p.positionInWorkArea(nil)
	if p.layoutErr != nil {
		return p.layoutErr
	}
	if p.options.Owner != 0 {
		if enabled, _, _ := pIsWindowEnabled.Call(uintptr(p.options.Owner)); enabled != 0 {
			pEnableWindow.Call(uintptr(p.options.Owner), 0)
			p.ownerDisabled = true
		}
	}
	if !p.captureHost {
		// Establish the loading state before the first visible frame so the
		// picker never flashes an uninitialized empty list.
		p.startLoad(processLoadInitial)
	}
	p.surfaces.PrepareCues()
	if err := firstFrame.Reveal(nativeform.FirstFrameOptions{
		RepeatShow: p.captureHost,
		Controls:   p.frameControls(),
	}); err != nil {
		return err
	}
	if !p.captureHost {
		pSetForeground.Call(uintptr(hwnd))
		trayicon.SetTabNavigationWindow(p.hwnd, func() { p.interaction.SetFocusVisible(true) })
	}
	return nil
}

func (p *picker) createColumns() {
	columns := []struct {
		key   string
		width int
	}{
		{"process_picker_column_process", 250},
		{"process_picker_column_description", 310},
		{"process_picker_column_instances", 82},
	}
	for index, column := range columns {
		text, _ := windows.UTF16PtrFromString(p.text(column.key))
		value := lvColumn{Mask: 0x0001 | 0x0002 | 0x0004, Cx: int32(float64(column.width) * p.scale()), Text: text, SubItem: int32(index)}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmInsertColumnW, uintptr(index), uintptr(unsafe.Pointer(&value)))
	}
}

func (p *picker) resizeColumns() {
	list := p.controls[idList]
	if list == 0 {
		return
	}
	// Hide the native lane before measuring. At some fractional DPI values the
	// ListView reports a client width that already excludes its transient
	// system scrollbar, while the header still spans the full control width.
	// Measuring that narrower client would reserve both lanes in the header.
	pShowScrollBar.Call(uintptr(list), sbVert, 0)
	var client rect
	pGetClientRect.Call(uintptr(list), uintptr(unsafe.Pointer(&client)))
	widths := processColumnWidths(int(client.Right-client.Left), p.scale())
	for index, width := range widths {
		pSendMessage.Call(uintptr(list), lvmSetColumnWidth, uintptr(index), uintptr(width))
	}
	// The shared overlay owns the visible vertical affordance. Keep the native
	// vertical bar hidden without forcibly toggling the horizontal bar: it must
	// remain available when the user intentionally widens a column.
	pShowScrollBar.Call(uintptr(list), sbVert, 0)
}

func processColumnWidths(clientWidth int, scale float64) [3]int {
	// Reserve a stable lane for the themed vertical scrollbar. ListView can
	// recreate its native vertical scroll mechanics after the first wheel or
	// item update; filling the complete client width makes that transition
	// create a horizontal scrollbar even though the default columns fit.
	reserve := max(1, int(float64(processScrollbarLane)*scale+0.5))
	available := max(0, clientWidth-reserve)
	last := min(available, int(82*scale+0.5))
	remaining := available - last
	minimumFirst := int(190*scale + 0.5)
	minimumSecond := int(180*scale + 0.5)
	desiredSecond := int(310*scale + 0.5)
	first, second := remaining, 0
	if remaining >= minimumFirst+minimumSecond {
		second = min(desiredSecond, remaining-minimumFirst)
		first = remaining - second
	}
	return [3]int{first, second, last}
}

func (p *picker) syncListScrollbarBounds() {
	if p.scrollbar == nil {
		return
	}
	scale := p.scale()
	bounds := p.bounds[idList]
	headerHeight := int(24*scale + 0.5)
	if p.header != 0 {
		var headerBounds rect
		if ok, _, _ := pGetWindowRect.Call(uintptr(p.header), uintptr(unsafe.Pointer(&headerBounds))); ok != 0 && headerBounds.Bottom > headerBounds.Top {
			headerHeight = int(headerBounds.Bottom - headerBounds.Top)
		}
	}
	width := int(float64(nativeform.ScrollbarWidth)*scale + 0.5)
	rightInset := int(2*scale + 0.5)
	x := int(float64(bounds.X+bounds.Width)*scale+0.5) - width - rightInset
	y := int(float64(bounds.Y-p.contentOffset)*scale+0.5) + headerHeight
	height := int(float64(bounds.Height)*scale+0.5) - headerHeight
	p.scrollbar.SetBounds(x, y, width, max(1, height))
}

func (p *picker) syncListScrollbar() {
	list := p.controls[idList]
	if list == 0 || p.scrollbar == nil {
		return
	}
	// Native non-client scrollbars cannot be colored reliably on every
	// supported Windows build. The list keeps its native scroll mechanics while
	// the shared themed scrollbar provides the visible mouse affordance.
	pShowScrollBar.Call(uintptr(list), sbVert, 0)
	top, _, _ := pSendMessage.Call(uintptr(list), lvmGetTopIndex, 0, 0)
	page, _, _ := pSendMessage.Call(uintptr(list), lvmGetCountPerPage, 0, 0)
	p.scrollbar.SetMetrics(len(p.visible), int(page), int(top))
}

func (p *picker) syncPreviewScrollbarBounds() {
	if p.previewScroll == nil {
		return
	}
	scale := p.scale()
	bounds := p.bounds[idPreview]
	width := int(float64(nativeform.ScrollbarWidth)*scale + 0.5)
	inset := max(1, int(2*scale+0.5))
	x := int(float64(bounds.X+bounds.Width)*scale+0.5) - width - inset
	y := int(float64(bounds.Y-p.contentOffset)*scale+0.5) + inset
	height := int(float64(bounds.Height)*scale+0.5) - 2*inset
	p.previewScroll.SetBounds(x, y, width, max(1, height))
}

func (p *picker) scrollListTo(position int) {
	list := p.controls[idList]
	if list == 0 || len(p.visible) == 0 {
		return
	}
	current, _, _ := pSendMessage.Call(uintptr(list), lvmGetTopIndex, 0, 0)
	if int(current) == position {
		return
	}
	itemBounds := rect{}
	if ok, _, _ := pSendMessage.Call(uintptr(list), lvmGetItemRect, 0, uintptr(unsafe.Pointer(&itemBounds))); ok == 0 {
		return
	}
	rowHeight := max(1, int(itemBounds.Bottom-itemBounds.Top))
	delta := (position - int(current)) * rowHeight
	pSendMessage.Call(uintptr(list), lvmScroll, 0, uintptr(delta))
	p.syncListScrollbar()
}

func (p *picker) scrollListWheel(wParam uintptr) bool {
	if len(p.visible) == 0 {
		return false
	}
	p.listWheelDelta += int(int16(wParam >> 16))
	steps := p.listWheelDelta / 120
	p.listWheelDelta %= 120
	if steps == 0 {
		return true
	}
	top, _, _ := pSendMessage.Call(uintptr(p.controls[idList]), lvmGetTopIndex, 0, 0)
	// Three rows follows the Windows default while keeping high-resolution
	// wheel deltas accumulated until one complete notch is available.
	p.scrollListTo(int(top) - steps*3)
	return true
}

func (p *picker) updateHeaderLabels() {
	for index := range 3 {
		caption := p.headerCaption(index)
		text, _ := windows.UTF16PtrFromString(caption)
		column := lvColumn{Mask: 0x0004, Text: text}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmSetColumnW, uintptr(index), uintptr(unsafe.Pointer(&column)))
	}
}

func (p *picker) headerCaption(index int) string {
	keys := []string{"process_picker_column_process", "process_picker_column_description", "process_picker_column_instances"}
	if index < 0 || index >= len(keys) {
		return ""
	}
	caption := p.text(keys[index])
	if p.sortColumn == index {
		if p.sortAscending {
			caption += "  ↑"
		} else {
			caption += "  ↓"
		}
	}
	return caption
}

func (p *picker) child(className, value string, style uintptr, x, y, width, height int, id uint16) windows.Handle {
	if p.createErr != nil {
		return 0
	}
	class, err := windows.UTF16PtrFromString(className)
	if err != nil {
		p.createErr = err
		return 0
	}
	caption, err := windows.UTF16PtrFromString(value)
	if err != nil {
		p.createErr = err
		return 0
	}
	scale := p.scale()
	hwnd, err := createWindowForPicker(0, class, caption, style,
		int(float64(x)*scale), int(float64(y)*scale), int(float64(width)*scale), int(float64(height)*scale),
		p.hwnd, uintptr(id))
	if err != nil {
		p.createErr = fmt.Errorf("create process picker control %d: %w", id, err)
		return 0
	}
	if p.font != 0 {
		pSendMessage.Call(uintptr(hwnd), wmSetFont, uintptr(p.font), 1)
	}
	nativeform.ApplyControl(hwnd, p.themeDark)
	p.controls[id] = hwnd
	p.bounds[id] = logicalBounds{X: x, Y: y, Width: width, Height: height}
	p.labels[id] = value
	if strings.EqualFold(className, "BUTTON") && style&bsOwnerDraw != 0 {
		p.interaction.Track(hwnd, 0)
	}
	return hwnd
}

func (p *picker) createTooltips() error {
	class, err := windows.UTF16PtrFromString("tooltips_class32")
	if err != nil {
		return err
	}
	hwnd, err := createWindowForPicker(0, class, nil, wsPopup|ttsAlwaysTip|ttsNoPrefix, 0, 0, 0, 0, p.hwnd, 0)
	if err != nil {
		return fmt.Errorf("create process picker tooltip: %w", err)
	}
	p.tooltip = hwnd
	nativeform.ApplyTooltip(p.tooltip, p.themeDark, p.palette)
	pSendMessage.Call(uintptr(hwnd), ttmSetMaxTipWidth, 0, uintptr(int(360*p.scale())))
	for id, key := range map[uint16]string{
		idSearch: "tip_process_search", idRefresh: "tip_process_refresh", idBrowse: "tip_process_browse",
		idCancel: "tip_process_cancel", idConfirm: "tip_process_confirm",
	} {
		if err := p.addTooltip(p.controls[id], p.text(key)); err != nil {
			return err
		}
	}
	return nil
}

func (p *picker) addTooltip(control windows.Handle, value string) error {
	if p.tooltip == 0 || control == 0 || value == "" {
		return nil
	}
	text, err := windows.UTF16FromString(value)
	if err != nil {
		return err
	}
	p.tooltipText = append(p.tooltipText, text)
	info := toolInfo{Size: uint32(unsafe.Sizeof(toolInfo{})), Flags: ttfIDIsHwnd | ttfSubclass, Hwnd: p.hwnd, ID: uintptr(control), Text: &p.tooltipText[len(p.tooltipText)-1][0]}
	// Some common-controls builds return zero here even though the tooltip is
	// registered and displayed. HWND creation is the reliable lifecycle gate;
	// the tooltip remains optional if an individual tool registration is refused.
	pSendMessage.Call(uintptr(p.tooltip), ttmAddTool, 0, uintptr(unsafe.Pointer(&info)))
	return nil
}
