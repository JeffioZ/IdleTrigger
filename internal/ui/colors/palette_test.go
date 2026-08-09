package colors

import (
	"math"
	"testing"
)

func TestAccentTextContrast(t *testing.T) {
	for _, dark := range []bool{false, true} {
		palette := ForTheme(dark)
		for _, text := range []struct {
			name       string
			foreground uint32
			background uint32
			minimum    float64
		}{
			{"accent", palette.AccentText, palette.Accent, 4.5},
			{"primary", palette.PrimaryText, palette.WindowBackground, 7},
			{"secondary", palette.SecondaryText, palette.WindowBackground, 4.5},
			{"muted", palette.MutedText, palette.WindowBackground, 3},
			{"disabled", palette.DisabledText, palette.DisabledSurface, 3},
			{"tooltip", palette.TooltipText, palette.TooltipBackground, 4.5},
			{"danger default", palette.DangerText, palette.DangerBackground, 4.5},
			{"danger hover", palette.DangerText, palette.DangerHover, 4.5},
			{"danger pressed", palette.DangerText, palette.DangerPressed, 4.5},
			{"close default", palette.CloseText, palette.WindowBackground, 4.5},
			{"close hover", palette.CloseActiveText, palette.CloseHover, 4.5},
			{"close pressed", palette.CloseActiveText, palette.ClosePressed, 4.5},
		} {
			if got := contrast(text.foreground, text.background); got < text.minimum {
				t.Fatalf("dark=%v %s contrast = %.2f, want >= %.1f", dark, text.name, got, text.minimum)
			}
		}
		if palette.Focus == palette.Accent || palette.FocusOnAccent == palette.AccentText {
			t.Fatalf("dark=%v focus colors must remain distinct from selected-control colors", dark)
		}
		if got := contrast(palette.Focus, palette.Surface); got < 3 {
			t.Fatalf("dark=%v standard focus contrast = %.2f, want >= 3", dark, got)
		}
		if got := contrast(palette.FocusOnAccent, palette.Accent); got < 3 {
			t.Fatalf("dark=%v selected focus contrast = %.2f, want >= 3", dark, got)
		}
		for _, surface := range []struct {
			name  string
			color uint32
		}{{"window", palette.WindowBackground}, {"surface", palette.Surface}} {
			if got := contrast(palette.Border, surface.color); got < 3 {
				t.Fatalf("dark=%v default border/%s contrast = %.2f, want >= 3", dark, surface.name, got)
			}
		}
		if got := contrast(palette.DangerSurfaceText, palette.Surface); got < 4.5 {
			t.Fatalf("dark=%v danger surface text contrast = %.2f, want >= 4.5", dark, got)
		}
		if got := contrast(palette.DangerFocus, palette.DangerBackground); got < 3 {
			t.Fatalf("dark=%v danger focus contrast = %.2f, want >= 3", dark, got)
		}
		for _, state := range []struct {
			name       string
			border     uint32
			background uint32
		}{
			{"default", palette.DangerBorder, palette.DangerBackground},
			{"hover", palette.DangerHoverBorder, palette.DangerHover},
			{"pressed", palette.DangerPressedBorder, palette.DangerPressed},
		} {
			if got := contrast(state.border, state.background); got < 1.5 {
				t.Fatalf("dark=%v danger %s border contrast = %.2f, want >= 1.5", dark, state.name, got)
			}
		}
	}
}

func TestExitAndCloseStatesRemainSemanticallyDistinct(t *testing.T) {
	for _, dark := range []bool{false, true} {
		palette := ForTheme(dark)
		if palette.DangerBackground == palette.WindowBackground || palette.DangerText == palette.PrimaryText {
			t.Fatalf("dark=%v exit state is not visually distinct", dark)
		}
		if palette.CloseHover == palette.DangerHover || palette.CloseActiveText == palette.DangerText {
			t.Fatalf("dark=%v warning close state must stay neutral rather than inherit exit colors", dark)
		}
		if palette.DangerBorder == palette.DangerHoverBorder || palette.DangerHoverBorder == palette.DangerPressedBorder {
			t.Fatalf("dark=%v exit borders must differentiate default, hover, and pressed states", dark)
		}
	}
}

func contrast(a, b uint32) float64 {
	la, lb := luminance(a), luminance(b)
	if la < lb {
		la, lb = lb, la
	}
	return (la + 0.05) / (lb + 0.05)
}

func luminance(c uint32) float64 {
	r := channel(c, 0)
	g := channel(c, 8)
	b := channel(c, 16)
	return 0.2126*linear(r) + 0.7152*linear(g) + 0.0722*linear(b)
}

func channel(c uint32, shift uint) float64 { return float64((c>>shift)&0xff) / 255 }
func linear(v float64) float64 {
	if v <= 0.04045 {
		return v / 12.92
	}
	return math.Pow((v+0.055)/1.055, 2.4)
}
