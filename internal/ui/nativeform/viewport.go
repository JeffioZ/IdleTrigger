package nativeform

import (
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"golang.org/x/sys/windows"
)

// Viewport keeps fixed form layouts reachable when enlarged text exceeds the
// monitor work area. Coordinates and scroll positions are in layout units.
type Viewport struct {
	X, Y, Width, Height     int
	totalWidth, totalHeight int
	horizontal, vertical    *Scrollbar
	onChange                func()
}

func NewViewport(parent windows.Handle, onChange func()) (*Viewport, error) {
	v := &Viewport{onChange: onChange}
	var err error
	v.horizontal, err = NewScrollbar(ScrollbarOptions{Parent: parent, Horizontal: true, OnChange: func(x int) { v.SetPosition(x, v.Y) }})
	if err != nil {
		return nil, err
	}
	v.vertical, err = NewScrollbar(ScrollbarOptions{Parent: parent, OnChange: func(y int) { v.SetPosition(v.X, y) }})
	if err != nil {
		v.horizontal.Close()
		return nil, err
	}
	return v, nil
}

func (v *Viewport) Sync(parent windows.Handle, scale float64, width, height int, palette colors.Palette) {
	pw, ph, err := ClientSize(parent)
	if err != nil {
		return
	}
	v.totalWidth, v.totalHeight = width, height
	w, h := int(float64(pw)/scale), int(float64(ph)/scale)
	// Either scrollbar may cause the other axis to overflow.
	horizontal, vertical := width > w, height > h
	for range 2 {
		if vertical {
			horizontal = width > w-ScrollbarWidth-2
		}
		if horizontal {
			vertical = height > h-ScrollbarWidth-2
		}
	}
	v.Width, v.Height = w, h
	if vertical {
		v.Width -= ScrollbarWidth + 2
	}
	if horizontal {
		v.Height -= ScrollbarWidth + 2
	}
	v.Width, v.Height = max(1, v.Width), max(1, v.Height)
	v.X, v.Y = min(v.X, max(0, width-v.Width)), min(v.Y, max(0, height-v.Height))
	b := max(1, int(float64(ScrollbarWidth)*scale+0.5))
	for _, bar := range []*Scrollbar{v.horizontal, v.vertical} {
		bar.SetScale(scale)
		bar.SetTheme(palette, palette.WindowBackground)
	}
	v.horizontal.SetBounds(0, ph-b, pw-b, b)
	v.vertical.SetBounds(pw-b, 0, b, ph-b)
	v.horizontal.SetMetrics(width, v.Width, v.X)
	v.vertical.SetMetrics(height, v.Height, v.Y)
}

func (v *Viewport) SetPosition(x, y int) {
	x, y = max(0, min(x, v.totalWidth-v.Width)), max(0, min(y, v.totalHeight-v.Height))
	if x == v.X && y == v.Y {
		return
	}
	v.X, v.Y = x, y
	if v.onChange != nil {
		v.onChange()
	}
}

func (v *Viewport) EnsureVisible(x, y, width, height int) {
	nx, ny := v.X, v.Y
	if x+width > nx+v.Width {
		nx = x + width - v.Width
	}
	if y+height > ny+v.Height {
		ny = y + height - v.Height
	}
	if x < nx {
		nx = x
	}
	if y < ny {
		ny = y
	}
	v.SetPosition(nx, ny)
}

func (v *Viewport) Wheel(wParam uintptr) bool {
	delta := int16(wParam >> 16)
	step := 36
	if delta > 0 {
		step = -step
	}
	if delta == 0 {
		return false
	}
	if wParam&4 != 0 || v.totalHeight <= v.Height {
		if v.totalWidth <= v.Width {
			return false
		}
		v.SetPosition(v.X+step, v.Y)
	} else {
		v.SetPosition(v.X, v.Y+step)
	}
	return true
}

func (v *Viewport) Windows() []windows.Handle {
	return []windows.Handle{v.horizontal.Window(), v.vertical.Window()}
}
func (v *Viewport) Close() { v.horizontal.Close(); v.vertical.Close() }
