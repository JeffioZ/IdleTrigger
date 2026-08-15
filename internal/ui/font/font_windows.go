// Package font resolves the native Win32 UI font used by IdleTrigger.
package font

import (
	"strings"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	spiGetNonClientMetrics = 0x0029
	lfFaceSize             = 32
)

// Choice records the font selected for one UI surface. It is deliberately
// small so callers can log it once when a window is created or rebuilt.
type Choice struct {
	Face           string
	Reason         string
	UILanguage     string
	SystemLanguage string
	SystemLocale   string
}

type logFont struct {
	Height, Width, Escapement, Orientation, Weight       int32
	Italic, Underline, StrikeOut, CharSet                byte
	OutPrecision, ClipPrecision, Quality, PitchAndFamily byte
	FaceName                                             [lfFaceSize]uint16
}

type nonClientMetrics struct {
	Size                                   uint32
	BorderWidth, ScrollWidth, ScrollHeight int32
	CaptionWidth, CaptionHeight            int32
	CaptionFont                            logFont
	SmallCaptionWidth, SmallCaptionHeight  int32
	SmallCaptionFont                       logFont
	MenuWidth, MenuHeight                  int32
	MenuFont, StatusFont, MessageFont      logFont
	PaddedBorderWidth                      int32
}

var (
	gdi32    = windows.NewLazySystemDLL("gdi32.dll")
	user32   = windows.NewLazySystemDLL("user32.dll")
	kernel32 = windows.NewLazySystemDLL("kernel32.dll")

	pCreateFontIndirect = gdi32.NewProc("CreateFontIndirectW")
	pCreateFont         = gdi32.NewProc("CreateFontW")
	pGetTextFace        = gdi32.NewProc("GetTextFaceW")
	pSelectObject       = gdi32.NewProc("SelectObject")
	pDeleteObject       = gdi32.NewProc("DeleteObject")
	pCreateCompatibleDC = gdi32.NewProc("CreateCompatibleDC")
	pDeleteDC           = gdi32.NewProc("DeleteDC")
	pSystemParameters   = user32.NewProc("SystemParametersInfoW")
	pUserUILanguage     = kernel32.NewProc("GetUserDefaultUILanguage")
	pSystemUILanguage   = kernel32.NewProc("GetSystemDefaultUILanguage")
	pUserLocaleName     = kernel32.NewProc("GetUserDefaultLocaleName")
)

// New creates a font at the requested logical point-like height. It chooses
// an installed system UI family for the current UI language and always has a
// system-message-font fallback. Callers own the returned HFONT.
func New(size, weight int32, chinese bool) (windows.Handle, Choice) {
	size = scaleRequestedSize(size)
	choice := resolve(chinese)
	for _, face := range candidates(chinese) {
		if font := createNamed(size, weight, face); font != 0 && sameFace(fontFace(font), face) {
			choice.Face, choice.Reason = face, "installed preferred UI font"
			return font, choice
		} else if font != 0 {
			pDeleteObject.Call(uintptr(font))
		}
	}
	if font := messageFont(size, weight); font != 0 {
		choice.Face, choice.Reason = fontFace(font), "system message font fallback"
		return font, choice
	}
	// CreateFontW with an empty family asks the GDI font mapper for its default;
	// it is the final safety net if metrics retrieval unexpectedly fails.
	font := createNamed(size, weight, "")
	choice.Face, choice.Reason = fontFace(font), "GDI default font fallback"
	return font, choice
}

// SystemLanguageIsChinese reports the current Windows UI language, independent
// of an application override. It is useful for UI that has no language state.
func SystemLanguageIsChinese() bool {
	id, _, _ := pUserUILanguage.Call()
	return uint16(id)&0x3ff == 0x04
}

func candidates(chinese bool) []string {
	if chinese {
		return []string{"Microsoft YaHei UI", "Microsoft YaHei", "Segoe UI Variable Text", "Segoe UI"}
	}
	return []string{"Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI"}
}

func resolve(chinese bool) Choice {
	ui := "en"
	if SystemLanguageIsChinese() {
		ui = "zh-CN"
	}
	if chinese {
		ui = "zh-CN (app)"
	}
	return Choice{UILanguage: ui, SystemLanguage: languageName(pSystemUILanguage), SystemLocale: localeName()}
}

func languageName(proc *windows.LazyProc) string {
	id, _, _ := proc.Call()
	if uint16(id)&0x3ff == 0x04 {
		return "zh-CN"
	}
	if id == 0 {
		return "unknown"
	}
	return "non-Chinese"
}

func localeName() string {
	buf := make([]uint16, 85)
	if n, _, _ := pUserLocaleName.Call(uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf))); n > 0 {
		return windows.UTF16ToString(buf)
	}
	return "unknown"
}

func createNamed(size, weight int32, face string) windows.Handle {
	ptr, _ := windows.UTF16PtrFromString(face)
	font, _, _ := pCreateFont.Call(uintptr(-size), 0, 0, 0, uintptr(weight), 0, 0, 0, 1, 0, 0, 5, 0, uintptr(unsafe.Pointer(ptr)))
	return windows.Handle(font)
}

func messageFont(size, weight int32) windows.Handle {
	ncm := nonClientMetrics{Size: uint32(unsafe.Sizeof(nonClientMetrics{}))}
	if ok, _, _ := pSystemParameters.Call(spiGetNonClientMetrics, uintptr(ncm.Size), uintptr(unsafe.Pointer(&ncm)), 0); ok == 0 {
		return 0
	}
	lf := ncm.MessageFont
	lf.Height = -size
	lf.Weight = weight
	return windows.Handle(callCreateFontIndirect(&lf))
}

func callCreateFontIndirect(lf *logFont) uintptr {
	font, _, _ := pCreateFontIndirect.Call(uintptr(unsafe.Pointer(lf)))
	return font
}

func fontFace(font windows.Handle) string {
	if font == 0 {
		return "unknown"
	}
	dc, _, _ := pCreateCompatibleDC.Call(0)
	if dc == 0 {
		return "unknown"
	}
	defer pDeleteDC.Call(dc)
	old, _, _ := pSelectObject.Call(dc, uintptr(font))
	defer pSelectObject.Call(dc, old)
	buf := make([]uint16, lfFaceSize)
	pGetTextFace.Call(dc, uintptr(len(buf)), uintptr(unsafe.Pointer(&buf[0])))
	return windows.UTF16ToString(buf)
}

func sameFace(a, b string) bool { return strings.EqualFold(strings.TrimSpace(a), strings.TrimSpace(b)) }
