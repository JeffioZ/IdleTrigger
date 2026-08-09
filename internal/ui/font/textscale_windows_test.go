package font

import "testing"

func TestTextScaleFactorValidationAndSize(t *testing.T) {
	original := queryTextScaleFactor
	t.Cleanup(func() { queryTextScaleFactor = original })
	for _, test := range []struct {
		name   string
		factor float64
		want   int32
	}{
		{"default", 1, 14},
		{"large", 1.5, 21},
		{"maximum", 2.25, 32},
		{"invalid low", 0.5, 14},
		{"invalid high", 3, 14},
	} {
		t.Run(test.name, func(t *testing.T) {
			queryTextScaleFactor = func() float64 { return test.factor }
			if got := scaleRequestedSize(14); got != test.want {
				t.Fatalf("scaled size = %d, want %d", got, test.want)
			}
		})
	}
}

func TestTextScaleOverrideRestoresPreviousValue(t *testing.T) {
	restore := OverrideTextScaleFactor(1.75)
	if got := TextScaleFactor(); got != 1.75 {
		t.Fatalf("override = %v, want 1.75", got)
	}
	restore()
}
