package font

import (
	"golang.org/x/sys/windows"
	"testing"
	"unsafe"
)

func TestChineseUIFontContainsApplicationGlyphs(t *testing.T) {
	for _, weight := range []int32{400, 700} {
		h, choice := NewForLayout(21, weight, true)
		if h == 0 {
			t.Fatal("font allocation failed")
		}
		defer pDeleteObject.Call(uintptr(h))
		dc, _, _ := pCreateCompatibleDC.Call(0)
		if dc == 0 {
			t.Fatal("create font measurement DC")
		}
		defer pDeleteDC.Call(dc)
		old, _, _ := pSelectObject.Call(dc, uintptr(h))
		text, _ := windows.UTF16FromString("状态提示设置Aa09")
		glyphs := make([]uint16, len(text)-1)
		n, _, _ := gdi32.NewProc("GetGlyphIndicesW").Call(dc, uintptr(unsafe.Pointer(&text[0])), uintptr(len(glyphs)), uintptr(unsafe.Pointer(&glyphs[0])), 1)
		pSelectObject.Call(dc, old)
		t.Logf("face=%s weight=%d glyphs=%v", choice.Face, weight, glyphs)
		if uint32(n) == 0xffffffff {
			t.Fatal("read glyph coverage")
		}
		for _, glyph := range glyphs {
			if glyph == 0xffff {
				t.Fatalf("%s is missing application glyphs", choice.Face)
			}
		}
	}
}

func TestLayoutFontAppliesTextScaleExactlyOnce(t *testing.T) {
	restore := OverrideTextScaleFactor(2)
	defer restore()
	for _, chinese := range []bool{false, true} {
		for _, makeFont := range []func() uintptr{
			func() uintptr { h, _ := New(14, 600, chinese); return uintptr(h) },
			func() uintptr { h, _ := NewForLayout(28, 600, chinese); return uintptr(h) },
		} {
			h := makeFont()
			if h == 0 {
				t.Fatal("font allocation failed")
			}
			var lf logFont
			n, _, _ := gdi32.NewProc("GetObjectW").Call(h, unsafe.Sizeof(lf), uintptr(unsafe.Pointer(&lf)))
			pDeleteObject.Call(h)
			if n == 0 || lf.Height != -28 {
				t.Fatalf("font height = %d, want -28", lf.Height)
			}
			if chinese && lf.Weight != 700 {
				t.Fatalf("Chinese heading weight = %d", lf.Weight)
			}
		}
	}
}

func TestCandidateOrderFavorsCurrentUILanguage(t *testing.T) {
	zh := candidates(true)
	if zh[0] != "Microsoft YaHei UI" {
		t.Fatalf("Chinese first choice = %q", zh[0])
	}
	en := candidates(false)
	if en[0] != "Segoe UI Variable Text" || en[1] != "Segoe UI" {
		t.Fatalf("Latin candidate order = %#v", en)
	}
}

func TestSameFaceIsCaseInsensitive(t *testing.T) {
	if !sameFace("Segoe UI", "segoe ui") || sameFace("Segoe UI", "Microsoft YaHei UI") {
		t.Fatal("unexpected font face comparison")
	}
}
