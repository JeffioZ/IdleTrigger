//go:build windows && devtools

package locknotify

import (
	"fmt"
	"image"
	"image/color"
	"image/color/palette"
	"image/draw"
	"image/gif"
	"image/png"
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/ui/font"
)

// Opt-in captures use the production renderer, never change Windows settings
// or synthesize keyboard input, and stay out of release binaries.
func TestCaptureLockNotification(t *testing.T) {
	dir := os.Getenv("IDLETRIGGER_LOCK_CAPTURE_OUT")
	if dir == "" {
		t.Skip("set IDLETRIGGER_LOCK_CAPTURE_OUT for design review")
	}
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restore := font.OverrideTextScaleFactor(1)
	defer restore()
	for _, dpi := range []uint32{96, 120, 144, 192} {
		scale := float64(dpi) / 96
		cellW, cellH := int(180*scale), int(128*scale)
		sheet := image.NewRGBA(image.Rect(0, 0, cellW*4, cellH*2))
		for row, dark := range []bool{false, true} {
			backdrop := color.RGBA{232, 237, 242, 255}
			if dark {
				backdrop = color.RGBA{23, 26, 31, 255}
			}
			draw.Draw(sheet, image.Rect(0, row*cellH, cellW*4, (row+1)*cellH), image.NewUniform(backdrop), image.Point{}, draw.Src)
			for col := 0; col < 4; col++ {
				on := col%2 == 0
				language, symbol, text := "zh-CN", "AA", "Caps Lock 开"
				if !on {
					symbol = "aa"
					text = "Caps Lock 关"
				}
				if col >= 2 {
					language = "en"
					symbol = "↕"
					text = "Scroll Lock On"
					if !on {
						text = "Scroll Lock Off"
					}
				}
				s, err := renderSurface(dpi, dark, on, language, symbol, text)
				if err != nil {
					t.Fatal(err)
				}
				rgba := surfaceImage(s)
				at := image.Pt(col*cellW+(cellW-rgba.Bounds().Dx())/2, row*cellH+(cellH-rgba.Bounds().Dy())/2)
				draw.Draw(sheet, rgba.Bounds().Add(at), rgba, image.Point{}, draw.Over)
				if dpi == 144 && row == 0 && col == 0 {
					captureMotion(t, dir, rgba)
				}
				s.close()
			}
		}
		file, err := os.Create(filepath.Join(dir, fmt.Sprintf("locknotify-%d.png", dpi)))
		if err != nil {
			t.Fatal(err)
		}
		err = png.Encode(file, sheet)
		file.Close()
		if err != nil {
			t.Fatal(err)
		}
	}
}

func surfaceImage(s *surface) *image.RGBA {
	img := image.NewRGBA(image.Rect(0, 0, int(s.size.X), int(s.size.Y)))
	for p := 0; p < len(s.pixels); p += 4 {
		img.Pix[p] = s.pixels[p+2]
		img.Pix[p+1] = s.pixels[p+1]
		img.Pix[p+2] = s.pixels[p]
		img.Pix[p+3] = s.pixels[p+3]
	}
	return img
}

func captureMotion(t *testing.T, dir string, card *image.RGBA) {
	animation := gif.GIF{LoopCount: 0}
	bounds := card.Bounds().Add(image.Pt(24, 24))
	canvas := image.Rect(0, 0, bounds.Max.X+24, bounds.Max.Y+24)
	for ms := 0; ms <= 1900; ms += 20 {
		frame := image.NewRGBA(canvas)
		draw.Draw(frame, canvas, image.NewUniform(color.RGBA{232, 237, 242, 255}), image.Point{}, draw.Src)
		elapsed := time.Duration(ms) * time.Millisecond
		at := image.Pt(24, 24+int(riseOffset(elapsed, 6, true)))
		draw.DrawMask(frame, card.Bounds().Add(at), card, image.Point{}, image.NewUniform(color.Alpha{opacity(elapsed, 0, true)}), image.Point{}, draw.Over)
		indexed := image.NewPaletted(canvas, palette.Plan9)
		draw.FloydSteinberg.Draw(indexed, canvas, frame, image.Point{})
		animation.Image = append(animation.Image, indexed)
		animation.Delay = append(animation.Delay, 2)
	}
	file, err := os.Create(filepath.Join(dir, "locknotify-motion.gif"))
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := gif.EncodeAll(file, &animation); err != nil {
		t.Fatal(err)
	}
}
