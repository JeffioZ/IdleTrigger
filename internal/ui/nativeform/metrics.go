package nativeform

// Shared logical-pixel tokens keep the compact form windows aligned with the
// main control panel. They are deliberately small and fixed rather than a
// user-configurable skin.
const (
	FormPadding       = 18
	ControlGap        = 8
	SectionGap        = 14
	LabelGap          = 2
	ButtonHeight      = 36
	DialogButtonWidth = 104
	FieldHeight       = 34
	CheckboxSize      = 16
	CornerRadius      = 6
	MenuRowHeight     = 34
	MenuRowGap        = 1
	MenuSurfaceInset  = 4
	MenuAnchorGap     = 0
	MenuMarkerWidth   = 3
	ScrollbarWidth    = 16
	ScrollbarMinThumb = 32
)

func MenuHeight(rowCount, rowHeight, rowGap, surfaceInset int) int {
	if rowCount <= 0 {
		return 2 * surfaceInset
	}
	return 2*surfaceInset + rowCount*rowHeight + (rowCount-1)*rowGap
}
