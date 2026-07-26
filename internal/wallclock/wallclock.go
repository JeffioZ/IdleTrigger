// Package wallclock resolves local scheduled times across time-zone changes.
package wallclock

import "time"

// At returns the requested local minute on day. During a daylight-saving
// overlap it selects the earliest occurrence; during a forward gap it selects
// the first valid local minute after the gap.
func At(day time.Time, minute int) time.Time {
	year, month, date := day.Date()
	hour, minuteOfHour := minute/60, minute%60
	location := day.Location()
	requested := time.Date(year, month, date, hour, minuteOfHour, 0, 0, time.UTC)
	normalized := time.Date(year, month, date, hour, minuteOfHour, 0, 0, location)

	offsets := make(map[int]struct{}, 3)
	boundaries := make([]time.Time, 0, 2)
	_, offset := normalized.Zone()
	offsets[offset] = struct{}{}
	start, end := normalized.ZoneBounds()
	if !start.IsZero() {
		boundaries = append(boundaries, start)
		_, previousOffset := start.Add(-time.Nanosecond).In(location).Zone()
		offsets[previousOffset] = struct{}{}
	}
	if !end.IsZero() {
		boundaries = append(boundaries, end)
		_, nextOffset := end.In(location).Zone()
		offsets[nextOffset] = struct{}{}
	}

	var earliest time.Time
	for candidateOffset := range offsets {
		candidate := requested.Add(-time.Duration(candidateOffset) * time.Second).In(location)
		if !sameLocalMinute(candidate, year, month, date, hour, minuteOfHour) {
			continue
		}
		if earliest.IsZero() || candidate.Before(earliest) {
			earliest = candidate
		}
	}
	if !earliest.IsZero() {
		return earliest
	}

	for _, boundary := range boundaries {
		candidate := boundary.In(location)
		candidateYear, candidateMonth, candidateDate := candidate.Date()
		candidateMinute := candidate.Hour()*60 + candidate.Minute()
		if candidateYear != year || candidateMonth != month || candidateDate != date || candidateMinute < minute {
			continue
		}
		if earliest.IsZero() || candidateMinute < earliest.Hour()*60+earliest.Minute() ||
			(candidateMinute == earliest.Hour()*60+earliest.Minute() && candidate.Before(earliest)) {
			earliest = candidate
		}
	}
	if !earliest.IsZero() {
		return earliest
	}

	// A location can exceptionally skip an entire civil day. With no valid
	// same-day minute, keep time.Date's normalized result as the fallback.
	return normalized
}

func sameLocalMinute(candidate time.Time, year int, month time.Month, date, hour, minute int) bool {
	candidateYear, candidateMonth, candidateDate := candidate.Date()
	return candidateYear == year &&
		candidateMonth == month &&
		candidateDate == date &&
		candidate.Hour() == hour &&
		candidate.Minute() == minute &&
		candidate.Second() == 0 &&
		candidate.Nanosecond() == 0
}
