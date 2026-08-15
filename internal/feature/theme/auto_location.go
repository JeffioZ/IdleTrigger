package theme

import (
	"fmt"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
)

var (
	themeKernel32           = windows.NewLazySystemDLL("kernel32.dll")
	pGetTimeZoneInformation = themeKernel32.NewProc("GetTimeZoneInformation")
)

// timezoneLookup maps Windows timezone names to approximate lat/lon.
var timezoneLookup = map[string][2]float64{
	"China Standard Time":            {39.9, 116.4},
	"Taipei Standard Time":           {25.0, 121.5},
	"Tokyo Standard Time":            {35.7, 139.7},
	"Korea Standard Time":            {37.6, 127.0},
	"Singapore Standard Time":        {1.35, 103.8},
	"India Standard Time":            {28.6, 77.2},
	"W. Europe Standard Time":        {52.5, 13.4},
	"GMT Standard Time":              {51.5, -0.1},
	"Central Europe Standard Time":   {48.2, 16.4},
	"E. Europe Standard Time":        {50.4, 30.5},
	"Russian Standard Time":          {55.8, 37.6},
	"Eastern Standard Time":          {40.7, -74.0},
	"Central Standard Time":          {41.9, -87.6},
	"Mountain Standard Time":         {33.4, -112.0},
	"Pacific Standard Time":          {34.0, -118.2},
	"Alaskan Standard Time":          {61.2, -149.9},
	"Hawaiian Standard Time":         {21.3, -157.8},
	"E. South America Standard Time": {-23.5, -46.6},
	"Atlantic Standard Time":         {-34.6, -58.4},
	"AUS Eastern Standard Time":      {-33.9, 151.2},
	"AUS Central Standard Time":      {-34.9, 138.6},
	"New Zealand Standard Time":      {-36.8, 174.8},
}

// AutoLocationInfo resolves coordinates for sunrise/sunset mode. When IP
// location is enabled, it is tried before falling back to the current Windows
// timezone, UTC offset, and finally Beijing.
func AutoLocationInfo(useIPLocation bool, blockIPLookup bool) LocationInfo {
	if useIPLocation {
		if blockIPLookup {
			if info, ok := cachedIPLocation(time.Now()); ok {
				return logAutoLocationInfo(info)
			}
		} else if info, ok := cachedIPLocationFast(time.Now()); ok {
			return logAutoLocationInfo(info)
		}
	}

	// Get the Windows timezone name via GetTimeZoneInformation.
	type tzInfo struct {
		Bias         int32
		StandardName [32]uint16
		StandardDate windows.Systemtime
		StandardBias int32
		DaylightName [32]uint16
		DaylightDate windows.Systemtime
		DaylightBias int32
	}
	var tz tzInfo
	result, _, _ := pGetTimeZoneInformation.Call(uintptr(unsafe.Pointer(&tz)))
	name := windows.UTF16ToString(tz.StandardName[:])
	return logAutoLocationInfo(locationFromTimezone(name, tz.Bias, tz.StandardBias, tz.DaylightBias, uint32(result)))
}

const timeZoneIDInvalid = ^uint32(0)

func locationFromTimezone(name string, bias, standardBias, daylightBias int32, state uint32) LocationInfo {
	if coords, ok := timezoneLookup[name]; ok {
		return LocationInfo{Latitude: coords[0], Longitude: coords[1], Source: LocationSourceTimezone, TimezoneName: name}
	}
	if effectiveBias, ok := effectiveTimezoneBias(bias, standardBias, daylightBias, state); ok {
		// Windows Bias is minutes west of UTC. Convert the effective UTC offset
		// to a rough longitude so sunrise/sunset is still locally plausible when
		// the timezone name is not in the lookup table. GetTimeZoneInformation
		// reports whether the current bias is standard, daylight, or a zone with
		// no daylight-saving transition; unrecognized and invalid states do not guess.
		offsetMinutes := -effectiveBias
		lon := float64(offsetMinutes) / 4
		if lon < -180 {
			lon = -180
		} else if lon > 180 {
			lon = 180
		}
		return LocationInfo{Latitude: 35, Longitude: lon, Source: LocationSourceUTCOffset, TimezoneName: name}
	}
	return LocationInfo{Latitude: 39.9, Longitude: 116.4, Source: LocationSourceDefault}
}

func effectiveTimezoneBias(bias, standardBias, daylightBias int32, state uint32) (int32, bool) {
	switch state {
	case windows.TIME_ZONE_ID_STANDARD:
		return bias + standardBias, true
	case windows.TIME_ZONE_ID_DAYLIGHT:
		return bias + daylightBias, true
	case windows.TIME_ZONE_ID_UNKNOWN:
		// UNKNOWN explicitly means this timezone has no daylight-saving period.
		return bias, true
	case timeZoneIDInvalid:
		return 0, false
	default:
		return 0, false
	}
}

var lastAutoLocationLog atomic.Value

func logAutoLocationInfo(info LocationInfo) LocationInfo {
	key := fmt.Sprintf("%s|%s|%.4f|%.4f", info.Source, info.TimezoneName, info.Latitude, info.Longitude)
	if last, _ := lastAutoLocationLog.Load().(string); last == key {
		return info
	}
	lastAutoLocationLog.Store(key)

	switch info.Source {
	case LocationSourceIP:
		if info.LocationLabel != "" {
			mylog.Info("Theme location: using IP location %q lat=%.4f lon=%.4f", info.LocationLabel, info.Latitude, info.Longitude)
		} else {
			mylog.Info("Theme location: using IP location lat=%.4f lon=%.4f", info.Latitude, info.Longitude)
		}
	case LocationSourceTimezone:
		mylog.Info("Theme location: using timezone %q lat=%.4f lon=%.4f", info.TimezoneName, info.Latitude, info.Longitude)
	case LocationSourceUTCOffset:
		if info.TimezoneName != "" {
			mylog.Info("Theme location: estimating from timezone %q lat=%.4f lon=%.4f", info.TimezoneName, info.Latitude, info.Longitude)
		} else {
			mylog.Info("Theme location: estimating from UTC offset lat=%.4f lon=%.4f", info.Latitude, info.Longitude)
		}
	case LocationSourceDefault:
		mylog.Info("Theme location: using fallback location lat=%.4f lon=%.4f", info.Latitude, info.Longitude)
	}
	return info
}
