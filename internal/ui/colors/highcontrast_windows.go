package colors

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	spiGetHighContrast = 0x0042
	hcfHighContrastOn  = 0x00000001

	colorWindow         = 5
	colorWindowFrame    = 6
	colorWindowText     = 8
	colorHighlight      = 13
	colorHighlightText  = 14
	colorButtonFace     = 15
	colorGrayText       = 17
	colorButtonText     = 18
	colorInfoText       = 23
	colorInfoBackground = 24
)

type highContrast struct {
	Size          uint32
	Flags         uint32
	DefaultScheme *uint16
}

var (
	colorUser32                 = windows.NewLazySystemDLL("user32.dll")
	pSystemParametersInfoColors = colorUser32.NewProc("SystemParametersInfoW")
	pGetSystemColorColors       = colorUser32.NewProc("GetSysColor")
	queryHighContrastForColors  = queryHighContrast
	getSystemColorForColors     = func(index int) uint32 {
		value, _, _ := pGetSystemColorColors.Call(uintptr(index))
		return uint32(value)
	}
)

func queryHighContrast() bool {
	state := highContrast{Size: uint32(unsafe.Sizeof(highContrast{}))}
	ok, _, _ := pSystemParametersInfoColors.Call(
		spiGetHighContrast,
		uintptr(state.Size),
		uintptr(unsafe.Pointer(&state)),
		0,
	)
	return ok != 0 && state.Flags&hcfHighContrastOn != 0
}

func systemHighContrastPalette() (Palette, bool) {
	if !queryHighContrastForColors() {
		return Palette{}, false
	}
	system := getSystemColorForColors
	window := system(colorWindow)
	windowText := system(colorWindowText)
	highlight := system(colorHighlight)
	highlightText := system(colorHighlightText)
	buttonFace := system(colorButtonFace)
	buttonText := system(colorButtonText)
	grayText := system(colorGrayText)
	frame := system(colorWindowFrame)
	infoBackground := system(colorInfoBackground)
	infoText := system(colorInfoText)
	// Shared controls keep window text during hover, so the hover fill remains
	// COLOR_WINDOW and COLOR_HIGHLIGHT is used by their border. Active and
	// pressed states switch both fill and text to the highlight color pair.
	return Palette{
		WindowBackground:    window,
		Surface:             window,
		ElevatedSurface:     window,
		HoverSurface:        window,
		Border:              windowText,
		SubtleBorder:        frame,
		PrimaryText:         windowText,
		SecondaryText:       windowText,
		MutedText:           windowText,
		DisabledText:        grayText,
		DisabledSurface:     buttonFace,
		Accent:              highlight,
		AccentHover:         highlight,
		AccentPressed:       highlight,
		Selected:            highlight,
		SelectedHover:       highlight,
		AccentText:          highlightText,
		Focus:               windowText,
		FocusOnAccent:       highlightText,
		DangerBackground:    highlight,
		DangerHover:         highlight,
		DangerPressed:       highlight,
		DangerBorder:        windowText,
		DangerHoverBorder:   windowText,
		DangerPressedBorder: windowText,
		DangerText:          highlightText,
		DangerSurfaceText:   windowText,
		DangerFocus:         highlightText,
		CloseText:           buttonText,
		CloseHover:          highlight,
		ClosePressed:        highlight,
		CloseActiveText:     highlightText,
		TooltipBackground:   infoBackground,
		TooltipText:         infoText,
	}, true
}
