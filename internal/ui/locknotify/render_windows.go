package locknotify

import (
	"fmt"
	"math"
	"unsafe"

	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
)

var (
	createDC           = gdi32.NewProc("CreateCompatibleDC")
	deleteDC           = gdi32.NewProc("DeleteDC")
	createDIB          = gdi32.NewProc("CreateDIBSection")
	getFontObject      = gdi32.NewProc("GetObjectW")
	createFontIndirect = gdi32.NewProc("CreateFontIndirectW")
	flushGDI           = gdi32.NewProc("GdiFlush")
	updateLayered      = user32.NewProc("UpdateLayeredWindow")
)

const shadowInset = 10

type point struct{ X, Y int32 }
type surface struct {
	dc, bitmap, previous uintptr
	size                 point
	inset                int32
	pixels               []byte // Premultiplied BGRA, owned by bitmap.
}

func (s *surface) close() {
	if s == nil {
		return
	}
	selectObject.Call(s.dc, s.previous)
	deleteObject.Call(s.bitmap)
	deleteDC.Call(s.dc)
}

// Keep the existing installed-font/fallback and accessibility sizing policy,
// but use grayscale AA locally: subpixel color fringes do not survive alpha
// animation over arbitrary desktop content.
func notificationFont(size, weight int32, chinese bool) windows.Handle {
	original, _ := font.New(size, weight, chinese)
	if original == 0 {
		return 0
	}
	var lf [92]byte // LOGFONTW, identical on x86 and x64.
	if n, _, _ := getFontObject.Call(uintptr(original), uintptr(len(lf)), uintptr(unsafe.Pointer(&lf))); n != uintptr(len(lf)) {
		return original
	}
	lf[26] = 4 // ANTIALIASED_QUALITY
	replacement, _, _ := createFontIndirect.Call(uintptr(unsafe.Pointer(&lf)))
	if replacement == 0 {
		return original
	}
	deleteObject.Call(uintptr(original))
	return windows.Handle(replacement)
}

func textBounds(dc uintptr, f windows.Handle, text string) rect {
	old, _, _ := selectObject.Call(dc, uintptr(f))
	defer selectObject.Call(dc, old)
	var bounds rect
	value := windows.StringToUTF16Ptr(text)
	drawText.Call(dc, uintptr(unsafe.Pointer(value)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), 0x420) // CALCRECT | SINGLELINE
	return bounds
}

func renderSurface(dpi uint32, dark, on bool, language, symbol, text string) (*surface, error) {
	scale := func(v int32) int32 { return max(1, int32(math.Round(float64(v)*float64(dpi)/96))) }
	fonts := [2]windows.Handle{
		notificationFont(scale(28), 500, false),
		notificationFont(scale(15), 400, i18n.ResolveLanguage(language) == "zh-CN"),
	}
	defer func() {
		for _, f := range fonts {
			if f != 0 {
				deleteObject.Call(uintptr(f))
			}
		}
	}()
	if fonts[0] == 0 || fonts[1] == 0 {
		return nil, fmt.Errorf("create notification fonts")
	}
	dc, _, err := createDC.Call(0)
	if dc == 0 {
		return nil, fmt.Errorf("create notification DC: %w", err)
	}
	title, label := textBounds(dc, fonts[0], symbol), textBounds(dc, fonts[1], text)
	// Fit real font metrics, including Windows accessibility text scaling.
	// Default footprint stays 140x88; larger text grows the card, never clips it.
	gap, pad := scale(4), scale(shadowInset)
	width := max(scale(cardWidth), max(title.Right, label.Right)+scale(28))
	contentHeight := title.Bottom + gap + label.Bottom
	height := max(scale(cardHeight), contentHeight+scale(22))
	s := &surface{dc: dc, size: point{width + 2*pad, height + 2*pad}, inset: pad}
	info := struct {
		Size                   uint32
		Width, Height          int32
		Planes, BitCount       uint16
		Compression, ImageSize uint32
		XPels, YPels           int32
		Used, Important        uint32
	}{Size: 40, Width: s.size.X, Height: -s.size.Y, Planes: 1, BitCount: 32}
	var bits uintptr
	s.bitmap, _, err = createDIB.Call(dc, uintptr(unsafe.Pointer(&info)), 0, uintptr(unsafe.Pointer(&bits)), 0, 0)
	if s.bitmap == 0 {
		deleteDC.Call(dc)
		return nil, fmt.Errorf("create notification bitmap: %w", err)
	}
	s.previous, _, _ = selectObject.Call(dc, s.bitmap)
	s.pixels = unsafe.Slice((*byte)(nativeform.MessagePointer(bits)), int(s.size.X)*int(s.size.Y)*4)
	palette := colors.ForTheme(dark)
	// Draw text onto an opaque, correctly colored surface first. GDI may write
	// zero alpha, so compose the final alpha explicitly after flushing GDI.
	face := palette.WindowBackground
	for p := 0; p < len(s.pixels); p += 4 {
		s.pixels[p] = byte(face >> 16)
		s.pixels[p+1] = byte(face >> 8)
		s.pixels[p+2] = byte(face)
		s.pixels[p+3] = 255
	}
	setBkMode.Call(dc, 1)
	ink := palette.SecondaryText
	if on {
		ink = palette.Accent
		if dark {
			ink = palette.Focus
		}
	}
	top := pad + (height-contentHeight)/2 - scale(1)
	for i, line := range [...]string{symbol, text} {
		box := rect{Left: pad + scale(12), Top: top, Right: pad + width - scale(12)}
		if i == 0 {
			box.Bottom = box.Top + title.Bottom
			setTextColor.Call(dc, uintptr(ink))
		} else {
			box.Bottom = box.Top + label.Bottom
			setTextColor.Call(dc, uintptr(palette.PrimaryText))
		}
		old, _, _ := selectObject.Call(dc, uintptr(fonts[i]))
		value := windows.StringToUTF16Ptr(line)
		drawText.Call(dc, uintptr(unsafe.Pointer(value)), ^uintptr(0), uintptr(unsafe.Pointer(&box)), 0x21) // CENTER | SINGLELINE
		selectObject.Call(dc, old)
		top = box.Bottom + gap
	}
	flushGDI.Call()
	composeSurface(s, float64(dpi)/96, palette.SubtleBorder, dark)
	return s, nil
}

// Signed distance to a rounded rectangle. Fractional edge coverage gives
// smooth corners at native pixel resolution; no binary HRGN clipping.
func roundDistance(x, y, halfWidth, halfHeight, radius float64) float64 {
	qx, qy := math.Abs(x)-halfWidth+radius, math.Abs(y)-halfHeight+radius
	return math.Hypot(math.Max(qx, 0), math.Max(qy, 0)) + math.Min(math.Max(qx, qy), 0) - radius
}

func composeSurface(s *surface, scale float64, border uint32, dark bool) {
	halfW, halfH := float64(s.size.X)/2, float64(s.size.Y)/2
	cardHalfW, cardHalfH := halfW-float64(s.inset), halfH-float64(s.inset)
	radius, sigma := 9*scale, 3.0*scale
	strength := 0.06
	if dark {
		strength = 0.12
	}
	for y := int32(0); y < s.size.Y; y++ {
		for x := int32(0); x < s.size.X; x++ {
			px, py := float64(x)+0.5-halfW, float64(y)+0.5-halfH
			d := roundDistance(px, py, cardHalfW, cardHalfH, radius)
			coverage := math.Max(0, math.Min(1, 0.5-d))
			inner := math.Max(0, math.Min(1, 0.5-d-0.75*scale))
			rim := coverage - inner
			shadowD := math.Max(0, d)
			shadow := strength * math.Exp(-shadowD*shadowD/(2*sigma*sigma))
			alpha := coverage + shadow*(1-coverage)
			p := int((y*s.size.X + x) * 4)
			for c := 0; c < 3; c++ {
				borderChannel := byte(border >> uint(8*(2-c)))
				s.pixels[p+c] = byte(math.Round(float64(s.pixels[p+c])*inner + float64(borderChannel)*rim))
			}
			s.pixels[p+3] = byte(math.Round(alpha * 255))
		}
	}
}

func (s *surface) present(hwnd uintptr, position point, alpha byte) bool {
	source := point{}
	blend := [4]byte{0, 0, alpha, 1} // AC_SRC_OVER, premultiplied per-pixel alpha.
	ok, _, _ := updateLayered.Call(hwnd, 0, uintptr(unsafe.Pointer(&position)), uintptr(unsafe.Pointer(&s.size)), s.dc, uintptr(unsafe.Pointer(&source)), 0, uintptr(unsafe.Pointer(&blend)), 2)
	return ok != 0
}
