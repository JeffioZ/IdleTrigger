package theme

import (
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"golang.org/x/sys/windows"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
)

func TestSunriseSunsetRejectsNonFiniteCoordinates(t *testing.T) {
	date := time.Date(2026, 7, 14, 12, 0, 0, 0, time.UTC)
	for _, coordinates := range [][2]float64{{math.NaN(), 0}, {0, math.Inf(1)}} {
		if light, dark := CalcSunriseSunset(date, coordinates[0], coordinates[1]); light != -1 || dark != -1 {
			t.Fatalf("non-finite coordinates returned %d/%d", light, dark)
		}
	}
}

func TestParseTime(t *testing.T) {
	for input, want := range map[string]int{
		"00:00": 0,
		"07:30": 450,
		"23:59": 1439,
		"bad":   -1,
		"aa:10": -1,
		"24:00": -1,
	} {
		if got := parseTime(input); got != want {
			t.Errorf("parseTime(%q) = %d, want %d", input, got, want)
		}
	}
}

func TestSunriseSunsetBeijingSummer(t *testing.T) {
	loc := time.FixedZone("CST", 8*60*60)
	date := time.Date(2026, 6, 21, 12, 0, 0, 0, loc)
	sunrise, sunset := CalcSunriseSunset(date, 39.9, 116.4)
	if sunrise < 240 || sunrise > 360 {
		t.Fatalf("unexpected sunrise: %d", sunrise)
	}
	if sunset < 1140 || sunset > 1260 {
		t.Fatalf("unexpected sunset: %d", sunset)
	}
}

func TestSunriseSunsetPolarCondition(t *testing.T) {
	date := time.Date(2026, 12, 21, 12, 0, 0, 0, time.UTC)
	sunrise, sunset := CalcSunriseSunset(date, 89, 0)
	if sunrise != -1 || sunset != -1 {
		t.Fatalf("polar condition should be unavailable, got %d/%d", sunrise, sunset)
	}
}

func TestScheduleInfoFallsBackToFixedTimeWhenSunriseUnavailable(t *testing.T) {
	date := time.Date(2026, 12, 21, 12, 0, 0, 0, time.UTC)
	info := ScheduleInfoFor("sunrise", "08:30", "20:45", 89, 0, date)
	if !info.OK {
		t.Fatal("schedule should fall back to fixed time")
	}
	if !info.FixedFallback {
		t.Fatal("schedule should report fixed fallback")
	}
	if info.LightTime != "08:30" || info.DarkTime != "20:45" {
		t.Fatalf("schedule = %s/%s, want 08:30/20:45", info.LightTime, info.DarkTime)
	}
}

func TestSchedulerFallsBackToFixedTimeWhenSunriseUnavailable(t *testing.T) {
	date := time.Date(2026, 12, 21, 12, 0, 0, 0, time.UTC)
	s := NewScheduler("sunrise", "08:30", "20:45", 89, 0, false, false)
	light, dark := s.scheduleMinutes(date)
	if light != 510 || dark != 1245 {
		t.Fatalf("schedule minutes = %d/%d, want 510/1245", light, dark)
	}
}

func TestLocationFromTimezoneUsesEffectiveWindowsBias(t *testing.T) {
	tests := []struct {
		name                             string
		bias, standardBias, daylightBias int32
		state                            uint32
		wantLongitude                    float64
	}{
		{"standard time", 300, 15, -60, windows.TIME_ZONE_ID_STANDARD, -78.75},
		{"daylight time", 300, 0, -60, windows.TIME_ZONE_ID_DAYLIGHT, -60},
		{"no daylight saving", -480, 15, -45, windows.TIME_ZONE_ID_UNKNOWN, 120},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			info := locationFromTimezone("Unknown Localized Time", tt.bias, tt.standardBias, tt.daylightBias, tt.state)
			if info.Source != LocationSourceUTCOffset {
				t.Fatalf("source = %s, want %s", info.Source, LocationSourceUTCOffset)
			}
			if info.Longitude != tt.wantLongitude {
				t.Fatalf("longitude = %v, want %v", info.Longitude, tt.wantLongitude)
			}
			if info.Latitude != 35 {
				t.Fatalf("latitude = %v, want 35", info.Latitude)
			}
		})
	}
}

func TestLocationFromTimezoneUsesDefaultForInvalidOrUnknownState(t *testing.T) {
	for _, state := range []uint32{timeZoneIDInvalid, 99} {
		info := locationFromTimezone("Unknown Localized Time", -480, 0, -60, state)
		if info.Source != LocationSourceDefault {
			t.Fatalf("state %d: source = %s, want %s", state, info.Source, LocationSourceDefault)
		}
	}
}

func TestLocationFromTimezonePrefersKnownTimezoneRegardlessOfState(t *testing.T) {
	info := locationFromTimezone("China Standard Time", 300, 0, -60, timeZoneIDInvalid)
	if info.Source != LocationSourceTimezone {
		t.Fatalf("source = %s, want %s", info.Source, LocationSourceTimezone)
	}
	if info.Latitude != 39.9 || info.Longitude != 116.4 {
		t.Fatalf("coordinates = %.1f/%.1f, want 39.9/116.4", info.Latitude, info.Longitude)
	}
}

func TestParseIPLocation(t *testing.T) {
	info, err := parseIPLocation(strings.NewReader(`{"success":true,"latitude":31.2304,"longitude":121.4737,"city":"Shanghai","region":"Shanghai","country":"China"}`))
	if err != nil {
		t.Fatalf("parseIPLocation: %v", err)
	}
	if info.Source != LocationSourceIP {
		t.Fatalf("source = %s, want %s", info.Source, LocationSourceIP)
	}
	if info.Latitude != 31.2304 || info.Longitude != 121.4737 {
		t.Fatalf("coordinates = %.4f/%.4f", info.Latitude, info.Longitude)
	}
	if info.LocationLabel != "Shanghai, Shanghai, China" {
		t.Fatalf("label = %q", info.LocationLabel)
	}
}

func TestParseIPLocationRejectsFailedResponse(t *testing.T) {
	if _, err := parseIPLocation(strings.NewReader(`{"success":false,"message":"rate limited"}`)); err == nil {
		t.Fatal("expected failed service response to return an error")
	}
}

func TestWinHTTPGetRejectsEmbeddedNULBeforeOpeningRequest(t *testing.T) {
	for _, tc := range []struct {
		name string
		host string
		path string
	}{
		{name: "host", host: "ipwho.is\x00invalid", path: "/"},
		{name: "path", host: "ipwho.is", path: "/\x00invalid"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := winHTTPGet(tc.host, tc.path, time.Second); err == nil {
				t.Fatal("expected an embedded NUL to return an encoding error")
			}
		})
	}
}

func TestResetIPLocationFailureCooldownAllowsImmediateRetry(t *testing.T) {
	originalInfo, originalOK, originalQuerying, originalLastAttempt := snapshotIPLocationCache()
	defer restoreIPLocationCache(originalInfo, originalOK, originalQuerying, originalLastAttempt)

	recentFailure := time.Now().Add(-time.Minute)
	restoreIPLocationCache(LocationInfo{}, false, false, recentFailure)
	ResetIPLocationFailureCooldown()

	_, ok, querying, lastAttempt := snapshotIPLocationCache()
	if ok || querying {
		t.Fatalf("failed cache state changed unexpectedly: ok=%v querying=%v", ok, querying)
	}
	if !lastAttempt.IsZero() {
		t.Fatalf("failure cooldown was not reset: lastAttempt=%s", lastAttempt)
	}
}

func TestResetIPLocationFailureCooldownPreservesSuccess(t *testing.T) {
	originalInfo, originalOK, originalQuerying, originalLastAttempt := snapshotIPLocationCache()
	defer restoreIPLocationCache(originalInfo, originalOK, originalQuerying, originalLastAttempt)

	wantInfo := LocationInfo{Latitude: 31.2304, Longitude: 121.4737, Source: LocationSourceIP}
	wantAttempt := time.Now().Add(-time.Hour)
	restoreIPLocationCache(wantInfo, true, false, wantAttempt)
	ResetIPLocationFailureCooldown()

	info, ok, querying, lastAttempt := snapshotIPLocationCache()
	if !ok || querying || info != wantInfo || !lastAttempt.Equal(wantAttempt) {
		t.Fatalf("successful cache changed: info=%+v ok=%v querying=%v lastAttempt=%s", info, ok, querying, lastAttempt)
	}
}

func snapshotIPLocationCache() (LocationInfo, bool, bool, time.Time) {
	ipCache.Lock()
	defer ipCache.Unlock()
	return ipCache.info, ipCache.ok, ipCache.querying, ipCache.lastAttempt
}

func restoreIPLocationCache(info LocationInfo, ok, querying bool, lastAttempt time.Time) {
	ipCache.Lock()
	defer ipCache.Unlock()
	ipCache.info = info
	ipCache.ok = ok
	ipCache.querying = querying
	ipCache.lastAttempt = lastAttempt
}

func TestValidateHTTPStatus(t *testing.T) {
	for _, status := range []uint32{200, 204, 299} {
		if err := validateHTTPStatus(status); err != nil {
			t.Errorf("status %d rejected: %v", status, err)
		}
	}
	for _, status := range []uint32{0, 199, 300, 429, 500} {
		if err := validateHTTPStatus(status); err == nil {
			t.Errorf("status %d accepted", status)
		}
	}
}

func TestManualOverrideEndsAtNextScheduledTransition(t *testing.T) {
	loc := time.FixedZone("CST", 8*60*60)
	s := NewScheduler("fixed", "07:00", "19:00", 0, 0, false, false)
	now := time.Date(2026, 7, 11, 10, 0, 0, 0, loc)
	s.HoldManualOverride(now)
	want := time.Date(2026, 7, 11, 19, 0, 0, 0, loc)
	if got := time.Unix(0, s.manualUntil.Load()); !got.Equal(want) {
		t.Fatalf("manual override ends at %s, want %s", got, want)
	}
}

func TestManualOverrideCrossesMidnightToNextTransition(t *testing.T) {
	loc := time.FixedZone("CST", 8*60*60)
	s := NewScheduler("fixed", "19:00", "07:00", 0, 0, false, false)
	now := time.Date(2026, 7, 11, 22, 0, 0, 0, loc)
	s.HoldManualOverride(now)
	want := time.Date(2026, 7, 12, 7, 0, 0, 0, loc)
	if got := time.Unix(0, s.manualUntil.Load()); !got.Equal(want) {
		t.Fatalf("manual override ends at %s, want %s", got, want)
	}
}

func TestManualOverrideUsesDSTTransitionDayWallClock(t *testing.T) {
	loc, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Skipf("load DST location: %v", err)
	}
	s := NewScheduler("fixed", "07:00", "19:00", 0, 0, false, false)
	now := time.Date(2026, 3, 8, 0, 30, 0, 0, loc)
	s.HoldManualOverride(now)
	want := time.Date(2026, 3, 8, 7, 0, 0, 0, loc)
	if got := time.Unix(0, s.manualUntil.Load()); !got.Equal(want) {
		t.Fatalf("manual override ends at %s, want %s", got, want)
	}
}

func TestSwitchFailureLogDedupesUntilSuccess(t *testing.T) {
	dir := t.TempDir()
	mylog.Init(true, dir)
	defer mylog.Close()

	s := NewScheduler("fixed", "07:00", "19:00", 0, 0, false, false)
	err := errTestSwitchFailure{}
	s.logSwitchFailure("schedule", ModeDark, err)
	s.logSwitchFailure("schedule", ModeDark, err)
	s.clearSwitchFailure()
	s.logSwitchFailure("schedule", ModeDark, err)

	text, readErr := os.ReadFile(filepath.Join(dir, "IdleTrigger.log"))
	if readErr != nil {
		t.Fatalf("read log: %v", readErr)
	}
	const want = "Theme switch failed: reason=schedule target=dark error=test switch failure"
	if got := strings.Count(string(text), want); got != 2 {
		t.Fatalf("failure log count = %d, want 2:\n%s", got, text)
	}
}

func TestSchedulerRetriesAfterThemeSwitchFailure(t *testing.T) {
	s := NewScheduler("fixed", "07:00", "19:00", 0, 0, false, false)
	attempts := 0
	s.switchTheme = func(Mode) error {
		attempts++
		if attempts == 1 {
			return errTestSwitchFailure{}
		}
		return nil
	}

	s.switchIfAllowed("schedule", ModeDark, nil)
	s.switchIfAllowed("schedule", ModeDark, nil)
	if attempts != 2 {
		t.Fatalf("theme switch attempts = %d, want retry after the first failure", attempts)
	}
}

type errTestSwitchFailure struct{}

func (errTestSwitchFailure) Error() string { return "test switch failure" }
