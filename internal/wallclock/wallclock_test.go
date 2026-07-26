package wallclock

import (
	"testing"
	"time"
)

func TestAtKeepsOrdinaryWallClockAcrossDSTDays(t *testing.T) {
	location := newYork(t)
	for _, day := range []time.Time{
		time.Date(2026, 3, 8, 12, 0, 0, 0, location),
		time.Date(2026, 11, 1, 12, 0, 0, 0, location),
	} {
		got := At(day, 7*60)
		if got.Hour() != 7 || got.Minute() != 0 {
			t.Fatalf("schedule on %s = %s, want 07:00 wall time", day.Format("2006-01-02"), got)
		}
	}
}

func TestAtUsesFirstValidMinuteAfterDSTGap(t *testing.T) {
	location := newYork(t)
	day := time.Date(2024, 3, 10, 12, 0, 0, 0, location)
	got := At(day, 2*60+30)
	want := time.Date(2024, 3, 10, 7, 0, 0, 0, time.UTC).In(location)
	if !got.Equal(want) || got.Hour() != 3 || got.Minute() != 0 {
		t.Fatalf("spring-forward schedule = %s, want first valid local minute %s", got, want)
	}
}

func TestAtUsesEarliestDSTOverlapOccurrence(t *testing.T) {
	location := newYork(t)
	day := time.Date(2024, 11, 3, 12, 0, 0, 0, location)
	got := At(day, 1*60+30)
	want := time.Date(2024, 11, 3, 5, 30, 0, 0, time.UTC).In(location)
	if !got.Equal(want) {
		t.Fatalf("fall-back schedule = %s, want earliest occurrence %s", got, want)
	}
	if _, offset := got.Zone(); offset != -4*60*60 {
		t.Fatalf("fall-back offset = %d, want EDT offset %d", offset, -4*60*60)
	}
}

func newYork(t *testing.T) *time.Location {
	t.Helper()
	location, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Skipf("load DST location: %v", err)
	}
	return location
}
