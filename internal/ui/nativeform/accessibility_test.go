package nativeform

import "testing"

func TestAccessibleCheckStateIncludesSemanticState(t *testing.T) {
	if got := accessibleCheckState(0, false); got != accStateFocusable {
		t.Fatalf("unchecked state = %#x, want focusable", got)
	}
	if got := accessibleCheckState(0, true); got != accStateFocusable|accStateChecked {
		t.Fatalf("checked state = %#x, want focusable+checked", got)
	}
}
