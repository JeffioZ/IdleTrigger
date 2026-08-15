package nativeform

import (
	"testing"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

func TestCheckboxHitWidthForTextMatchesDrawnContent(t *testing.T) {
	if got, want := checkboxHitWidthForText(80), 108; got != want {
		t.Fatalf("checkbox hit width = %d, want %d", got, want)
	}
	if got, want := checkboxHitWidthForText(-1), 28; got != want {
		t.Fatalf("empty checkbox hit width = %d, want %d", got, want)
	}
}

func TestScaledPixelsPreservesFractionalDPI(t *testing.T) {
	tests := []struct {
		scale float64
		want  int32
	}{
		{scale: 1, want: 18},
		{scale: 1.25, want: 23},
		{scale: 1.5, want: 27},
		{scale: 2, want: 36},
	}
	for _, test := range tests {
		if got := scaledPixels(18, test.scale); got != test.want {
			t.Errorf("scaledPixels(18, %.2f) = %d, want %d", test.scale, got, test.want)
		}
	}
}

func TestCenteredTextTopKeepsOddRemainderBelowLabel(t *testing.T) {
	// At 150% DPI a 36 px logical button is 54 physical pixels while the
	// Chinese UI font commonly measures 25 pixels high. The spare 29th pixel
	// belongs below the label so its optical center does not drift downward.
	if got, want := centeredTextTop(100, 154, 25), int32(114); got != want {
		t.Fatalf("centered text top = %d, want %d", got, want)
	}
	if got, want := centeredTextTop(100, 136, 17), int32(109); got != want {
		t.Fatalf("96-DPI centered text top = %d, want %d", got, want)
	}
	if got := centeredTextTop(100, 120, 20); got != 100 {
		t.Fatalf("oversized text top = %d, want unchanged bounds", got)
	}
}

func TestHanOpticalLiftDoesNotChangeLatinButtons(t *testing.T) {
	if !needsHanOpticalLift("切换主题") {
		t.Fatal("Han button label did not request its optical lift")
	}
	for _, label := range []string{"Switch Theme", "Repairing…", "123"} {
		if needsHanOpticalLift(label) {
			t.Fatalf("Latin label %q requested a Han optical lift", label)
		}
	}
}

func TestButtonVisualStatePriority(t *testing.T) {
	palette := colors.ForTheme(false)
	tests := []struct {
		name       string
		state      ControlState
		wantFill   uint32
		wantBorder uint32
	}{
		{name: "hover", state: ControlState{Hovered: true}, wantFill: palette.HoverSurface, wantBorder: palette.Accent},
		{name: "active hover", state: ControlState{Active: true, Hovered: true}, wantFill: palette.SelectedHover, wantBorder: palette.SelectedHover},
		{name: "pressed beats hover", state: ControlState{Hovered: true, Pressed: true}, wantFill: palette.AccentPressed, wantBorder: palette.AccentPressed},
		{name: "pressed beats active hover", state: ControlState{Active: true, Hovered: true, Pressed: true}, wantFill: palette.AccentPressed, wantBorder: palette.AccentPressed},
		{name: "disabled beats pressed", state: ControlState{Pressed: true, Disabled: true}, wantFill: palette.DisabledSurface, wantBorder: palette.SubtleBorder},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			fill, border, _ := buttonVisual(palette, test.state)
			if fill != test.wantFill || border != test.wantBorder {
				t.Fatalf("buttonVisual(%+v) = fill %#x border %#x, want fill %#x border %#x", test.state, fill, border, test.wantFill, test.wantBorder)
			}
		})
	}
}

func TestTableHeaderVisualUsesSemanticThemeStates(t *testing.T) {
	palette := colors.ForTheme(true)
	tests := []struct {
		name     string
		state    ControlState
		wantFill uint32
		wantText uint32
		wantLine uint32
	}{
		{"rest", ControlState{}, palette.ElevatedSurface, palette.PrimaryText, palette.SubtleBorder},
		{"hover", ControlState{Hovered: true}, palette.HoverSurface, palette.PrimaryText, palette.Accent},
		{"pressed", ControlState{Hovered: true, Pressed: true}, palette.AccentPressed, palette.AccentText, palette.AccentPressed},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			fill, text, line := tableHeaderVisual(palette, test.state)
			if fill != test.wantFill || text != test.wantText || line != test.wantLine {
				t.Fatalf("tableHeaderVisual(%+v) = (%06x, %06x, %06x), want (%06x, %06x, %06x)", test.state, fill, text, line, test.wantFill, test.wantText, test.wantLine)
			}
		})
	}
}
