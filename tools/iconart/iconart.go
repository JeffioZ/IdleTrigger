// Package iconart defines the Quiet Trigger Mark used by the application icon
// and the two purpose-built Windows notification-area variants.
package iconart

import (
	"image"
	"image/color"
	"math"
)

const supersample = 8

type point struct{ x, y float64 }

type iconSpec struct {
	margin float64
	radius float64
	border float64 // logical output pixels; zero means no visible rim.
	bolt   []point
}

// App draws the single Quiet Trigger application mark. It intentionally keeps
// one dark-blue tile in every context: the app icon is a brand asset, not a
// taskbar-adaptive glyph. The 16 px frame drops only the fragile thin rim.
func App(size int) image.Image {
	spec := markSpec(size)
	return render(size, func(x, y float64) color.NRGBA {
		if !roundedRect(x, y, spec.margin, spec.margin, 1-spec.margin, 1-spec.margin, spec.radius) {
			return color.NRGBA{}
		}
		if isBorder(x, y, spec, size) {
			return color.NRGBA{48, 83, 116, 255} // #305374: restrained blue rim.
		}
		if inPolygon(x, y, spec.bolt) {
			return color.NRGBA{53, 190, 240, 255} // #35BEF0: Quiet Trigger cyan.
		}
		return color.NRGBA{17, 34, 56, 255} // #112238: quiet midnight surface.
	})
}

// Tray draws the compact, high-contrast Windows notification-area mark. The
// geometry is identical for both themes; only the contrast palette changes.
// onDarkBackground selects the pale resource for dark taskbars/title bars.
func Tray(size int, onDarkBackground bool) image.Image {
	spec := markSpec(size)
	var rim, tile, bolt color.NRGBA
	if onDarkBackground {
		rim = color.NRGBA{151, 190, 214, 255}  // #97BED6
		tile = color.NRGBA{226, 238, 247, 255} // #E2EEF7
		bolt = color.NRGBA{4, 70, 108, 255}    // #04466C
	} else {
		rim = color.NRGBA{42, 92, 133, 255}   // #2A5C85
		tile = color.NRGBA{15, 38, 62, 255}   // #0F263E
		bolt = color.NRGBA{51, 187, 229, 255} // #33BBE5
	}
	return render(size, func(x, y float64) color.NRGBA {
		if !roundedRect(x, y, spec.margin, spec.margin, 1-spec.margin, 1-spec.margin, spec.radius) {
			return color.NRGBA{}
		}
		if isBorder(x, y, spec, size) {
			return rim
		}
		if inPolygon(x, y, spec.bolt) {
			return bolt
		}
		return tile
	})
}

// markSpec uses size-specific geometry rather than resizing a 256 px drawing.
// The 16 px frame prioritizes a stable lightning silhouette; 20 px and above
// recover a one-pixel-class rim, and high-DPI frames progressively restore the
// full Quiet Trigger proportions.
func markSpec(size int) iconSpec {
	spec := opticalSpec(size)
	// Keep one clear pixel around small frames and a restrained 1/32 margin
	// on larger frames. All resources use the same coverage, including the
	// theme-aware window icons, so changing surfaces does not shrink the mark.
	margin := math.Max(1/float64(size), 1.0/32)
	scale := (1 - 2*margin) / (1 - 2*spec.margin)
	spec.margin = margin
	spec.radius *= scale
	for i := range spec.bolt {
		spec.bolt[i].x = 0.5 + (spec.bolt[i].x-0.5)*scale
		spec.bolt[i].y = 0.5 + (spec.bolt[i].y-0.5)*scale
	}
	// Preserve the hand-tuned pixel stroke instead of thickening the rim.
	return spec
}

func opticalSpec(size int) iconSpec {
	switch size {
	case 16:
		return iconSpec{0.100, 0.190, 0, points(0.605, 0.175, 0.300, 0.545, 0.470, 0.545, 0.410, 0.815, 0.715, 0.420, 0.545, 0.420)}
	case 20:
		return iconSpec{0.085, 0.185, 0.85, points(0.605, 0.155, 0.285, 0.548, 0.465, 0.548, 0.400, 0.835, 0.728, 0.418, 0.545, 0.418)}
	case 24:
		return iconSpec{0.080, 0.180, 0.90, points(0.605, 0.145, 0.278, 0.550, 0.462, 0.550, 0.397, 0.842, 0.735, 0.415, 0.545, 0.415)}
	case 32:
		return iconSpec{0.070, 0.175, 1.00, points(0.603, 0.140, 0.270, 0.550, 0.460, 0.550, 0.392, 0.850, 0.742, 0.413, 0.542, 0.413)}
	case 40:
		return iconSpec{0.067, 0.172, 1.10, points(0.602, 0.137, 0.268, 0.550, 0.459, 0.550, 0.390, 0.853, 0.744, 0.412, 0.541, 0.412)}
	case 48:
		return iconSpec{0.065, 0.170, 1.15, points(0.600, 0.135, 0.265, 0.550, 0.458, 0.550, 0.388, 0.855, 0.746, 0.410, 0.540, 0.410)}
	case 64:
		return iconSpec{0.060, 0.180, 1.35, points(0.600, 0.135, 0.265, 0.550, 0.458, 0.550, 0.388, 0.855, 0.746, 0.410, 0.540, 0.410)}
	case 128:
		return iconSpec{0.057, 0.195, 1.85, points(0.600, 0.135, 0.265, 0.550, 0.458, 0.550, 0.388, 0.855, 0.746, 0.410, 0.540, 0.410)}
	default: // 256 px and any future high-resolution source frame.
		return iconSpec{0.055, 0.205, 2.50, points(0.600, 0.135, 0.265, 0.550, 0.458, 0.550, 0.388, 0.855, 0.746, 0.410, 0.540, 0.410)}
	}
}

func points(values ...float64) []point {
	result := make([]point, 0, len(values)/2)
	for i := 0; i < len(values); i += 2 {
		result = append(result, point{values[i], values[i+1]})
	}
	return result
}

func isBorder(x, y float64, spec iconSpec, size int) bool {
	if spec.border <= 0 {
		return false
	}
	inset := spec.border / float64(size)
	innerRadius := math.Max(0, spec.radius-inset)
	return !roundedRect(x, y, spec.margin+inset, spec.margin+inset, 1-spec.margin-inset, 1-spec.margin-inset, innerRadius)
}

func render(size int, sample func(x, y float64) color.NRGBA) image.Image {
	large := image.NewNRGBA(image.Rect(0, 0, size*supersample, size*supersample))
	for y := 0; y < large.Bounds().Dy(); y++ {
		for x := 0; x < large.Bounds().Dx(); x++ {
			large.SetNRGBA(x, y, sample((float64(x)+0.5)/float64(size*supersample), (float64(y)+0.5)/float64(size*supersample)))
		}
	}
	return downsample(large, size)
}

// downsample averages coverage in premultiplied-alpha space. This prevents
// transparent black from contaminating antialiased rounded corners when the
// Windows shell composites the PNG-backed ICO frame over a taskbar.
func downsample(src *image.NRGBA, size int) image.Image {
	dst := image.NewNRGBA(image.Rect(0, 0, size, size))
	for y := 0; y < size; y++ {
		for x := 0; x < size; x++ {
			var pr, pg, pb, a uint32
			for yy := 0; yy < supersample; yy++ {
				for xx := 0; xx < supersample; xx++ {
					c := src.NRGBAAt(x*supersample+xx, y*supersample+yy)
					pr += uint32(c.R) * uint32(c.A)
					pg += uint32(c.G) * uint32(c.A)
					pb += uint32(c.B) * uint32(c.A)
					a += uint32(c.A)
				}
			}
			if a == 0 {
				continue
			}
			dst.SetNRGBA(x, y, color.NRGBA{uint8(pr / a), uint8(pg / a), uint8(pb / a), uint8(a / uint32(supersample*supersample))})
		}
	}
	return dst
}

func roundedRect(x, y, left, top, right, bottom, radius float64) bool {
	if (x >= left+radius && x <= right-radius) || (y >= top+radius && y <= bottom-radius) {
		return x >= left && x <= right && y >= top && y <= bottom
	}
	cx := math.Max(left+radius, math.Min(x, right-radius))
	cy := math.Max(top+radius, math.Min(y, bottom-radius))
	dx, dy := x-cx, y-cy
	return dx*dx+dy*dy <= radius*radius
}

func inPolygon(x, y float64, polygon []point) bool {
	inside := false
	for i, j := 0, len(polygon)-1; i < len(polygon); j, i = i, i+1 {
		a, b := polygon[i], polygon[j]
		if (a.y > y) != (b.y > y) && x < (b.x-a.x)*(y-a.y)/(b.y-a.y)+a.x {
			inside = !inside
		}
	}
	return inside
}
