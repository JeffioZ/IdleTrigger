package theme

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"sync"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
)

// LocationSource describes how automatic sunrise/sunset coordinates were
// resolved when the config does not provide explicit coordinates.
type LocationSource string

const (
	LocationSourceConfigured LocationSource = "configured"
	LocationSourceIP         LocationSource = "ip"
	LocationSourceTimezone   LocationSource = "timezone"
	LocationSourceUTCOffset  LocationSource = "utc_offset"
	LocationSourceDefault    LocationSource = "default"
)

// LocationInfo is the resolved coordinate pair used for sunrise/sunset mode.
type LocationInfo struct {
	Latitude, Longitude float64
	Source              LocationSource
	TimezoneName        string
	LocationLabel       string
}

const (
	ipLocationHost            = "ipwho.is"
	ipLocationPath            = "/?fields=success,message,latitude,longitude,city,region,country"
	ipLocationRequestTimeout  = 4 * time.Second
	maxIPLocationResponseSize = 64 * 1024
	// IPLocationRetryInterval is shared by the lookup cache and the tray's
	// single delayed retry so both enforce the same cooldown.
	IPLocationRetryInterval   = 30 * time.Minute
	ipLocationSuccessCacheTTL = 24 * time.Hour
)

var (
	winhttp                    = windows.NewLazySystemDLL("winhttp.dll")
	procWinHttpOpen            = winhttp.NewProc("WinHttpOpen")
	procWinHttpConnect         = winhttp.NewProc("WinHttpConnect")
	procWinHttpOpenRequest     = winhttp.NewProc("WinHttpOpenRequest")
	procWinHttpSendRequest     = winhttp.NewProc("WinHttpSendRequest")
	procWinHttpReceiveResponse = winhttp.NewProc("WinHttpReceiveResponse")
	procWinHttpQueryHeaders    = winhttp.NewProc("WinHttpQueryHeaders")
	procWinHttpQueryData       = winhttp.NewProc("WinHttpQueryDataAvailable")
	procWinHttpReadData        = winhttp.NewProc("WinHttpReadData")
	procWinHttpSetTimeouts     = winhttp.NewProc("WinHttpSetTimeouts")
	procWinHttpCloseHandle     = winhttp.NewProc("WinHttpCloseHandle")
)

var ipCache struct {
	sync.Mutex
	info        LocationInfo
	ok          bool
	querying    bool
	lastAttempt time.Time
}

// ResetIPLocationFailureCooldown lets an explicit new lookup cycle retry
// immediately after an earlier failure. A successful location remains cached,
// and an in-flight lookup is still shared instead of duplicated.
func ResetIPLocationFailureCooldown() {
	ipCache.Lock()
	defer ipCache.Unlock()
	if !ipCache.ok {
		ipCache.lastAttempt = time.Time{}
	}
}

func cachedIPLocation(now time.Time) (LocationInfo, bool) {
	ipCache.Lock()
	if ipCache.ok && now.Sub(ipCache.lastAttempt) < ipLocationSuccessCacheTTL {
		info := ipCache.info
		ipCache.Unlock()
		return info, true
	}
	if !ipCache.ok && !ipCache.lastAttempt.IsZero() && now.Sub(ipCache.lastAttempt) < IPLocationRetryInterval {
		ipCache.Unlock()
		return LocationInfo{}, false
	}
	if ipCache.querying {
		ipCache.Unlock()
		return LocationInfo{}, false
	}
	ipCache.lastAttempt = now
	ipCache.querying = true
	ipCache.Unlock()

	info, reason, ok := queryIPLocation()

	ipCache.Lock()
	defer ipCache.Unlock()
	ipCache.lastAttempt = time.Now()
	ipCache.querying = false
	if ok {
		ipCache.info = info
		ipCache.ok = true
		mylog.Info("Theme location: IP location resolved lat=%.4f lon=%.4f label=%q", info.Latitude, info.Longitude, info.LocationLabel)
		return info, true
	}
	ipCache.ok = false
	mylog.Info("Theme location: IP location unavailable: %s", reason)
	return LocationInfo{}, false
}

func cachedIPLocationFast(now time.Time) (LocationInfo, bool) {
	ipCache.Lock()
	defer ipCache.Unlock()
	if ipCache.ok && now.Sub(ipCache.lastAttempt) < ipLocationSuccessCacheTTL {
		return ipCache.info, true
	}
	return LocationInfo{}, false
}

type ipWhoIsResponse struct {
	Success   bool    `json:"success"`
	Message   string  `json:"message"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
	City      string  `json:"city"`
	Region    string  `json:"region"`
	Country   string  `json:"country"`
}

func queryIPLocation() (LocationInfo, string, bool) {
	body, err := winHTTPGet(ipLocationHost, ipLocationPath, ipLocationRequestTimeout)
	if err != nil {
		return LocationInfo{}, err.Error(), false
	}
	info, err := parseIPLocation(strings.NewReader(body))
	if err != nil {
		return LocationInfo{}, err.Error(), false
	}
	return info, "", true
}

func winHTTPGet(host, path string, timeout time.Duration) (string, error) {
	const (
		accessTypeDefaultProxy = 0
		defaultHTTPSPort       = 443
		flagSecure             = 0x00800000
	)
	userAgent, err := windows.UTF16PtrFromString("IdleTrigger")
	if err != nil {
		return "", fmt.Errorf("encode WinHTTP user agent: %w", err)
	}
	hostPtr, err := windows.UTF16PtrFromString(host)
	if err != nil {
		return "", fmt.Errorf("encode WinHTTP host: %w", err)
	}
	method, err := windows.UTF16PtrFromString("GET")
	if err != nil {
		return "", fmt.Errorf("encode WinHTTP method: %w", err)
	}
	pathPtr, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return "", fmt.Errorf("encode WinHTTP path: %w", err)
	}
	session, _, err := procWinHttpOpen.Call(uintptr(unsafe.Pointer(userAgent)), accessTypeDefaultProxy, 0, 0, 0)
	if session == 0 {
		return "", fmt.Errorf("WinHttpOpen: %w", err)
	}
	defer procWinHttpCloseHandle.Call(session)

	timeoutMS := int(timeout / time.Millisecond)
	procWinHttpSetTimeouts.Call(session, uintptr(timeoutMS), uintptr(timeoutMS), uintptr(timeoutMS), uintptr(timeoutMS))

	connect, _, err := procWinHttpConnect.Call(session, uintptr(unsafe.Pointer(hostPtr)), defaultHTTPSPort, 0)
	if connect == 0 {
		return "", fmt.Errorf("WinHttpConnect: %w", err)
	}
	defer procWinHttpCloseHandle.Call(connect)

	request, _, err := procWinHttpOpenRequest.Call(connect, uintptr(unsafe.Pointer(method)), uintptr(unsafe.Pointer(pathPtr)), 0, 0, 0, flagSecure)
	if request == 0 {
		return "", fmt.Errorf("WinHttpOpenRequest: %w", err)
	}
	defer procWinHttpCloseHandle.Call(request)

	if ok, _, err := procWinHttpSendRequest.Call(request, 0, 0, 0, 0, 0, 0); ok == 0 {
		return "", fmt.Errorf("WinHttpSendRequest: %w", err)
	}
	if ok, _, err := procWinHttpReceiveResponse.Call(request, 0); ok == 0 {
		return "", fmt.Errorf("WinHttpReceiveResponse: %w", err)
	}
	const (
		queryStatusCode = 19
		queryFlagNumber = 0x20000000
	)
	var statusCode uint32
	statusCodeSize := uint32(unsafe.Sizeof(statusCode))
	if ok, _, err := procWinHttpQueryHeaders.Call(
		request,
		queryStatusCode|queryFlagNumber,
		0,
		uintptr(unsafe.Pointer(&statusCode)),
		uintptr(unsafe.Pointer(&statusCodeSize)),
		0,
	); ok == 0 {
		return "", fmt.Errorf("WinHttpQueryHeaders(status): %w", err)
	}
	if err := validateHTTPStatus(statusCode); err != nil {
		return "", err
	}

	var out strings.Builder
	for {
		var available uint32
		if ok, _, err := procWinHttpQueryData.Call(request, uintptr(unsafe.Pointer(&available))); ok == 0 {
			return "", fmt.Errorf("WinHttpQueryDataAvailable: %w", err)
		}
		if available == 0 {
			break
		}
		if !responseChunkFits(out.Len(), available) {
			return "", errors.New("response too large")
		}
		buf := make([]byte, available)
		var read uint32
		if ok, _, err := procWinHttpReadData.Call(request, uintptr(unsafe.Pointer(&buf[0])), uintptr(available), uintptr(unsafe.Pointer(&read))); ok == 0 {
			return "", fmt.Errorf("WinHttpReadData: %w", err)
		}
		out.WriteString(string(buf[:read]))
	}
	return out.String(), nil
}

func responseChunkFits(currentSize int, available uint32) bool {
	if currentSize < 0 || currentSize > maxIPLocationResponseSize {
		return false
	}
	return uint64(available) <= uint64(maxIPLocationResponseSize-currentSize)
}

func validateHTTPStatus(statusCode uint32) error {
	if statusCode < 200 || statusCode >= 300 {
		return fmt.Errorf("HTTP status %d", statusCode)
	}
	return nil
}

func parseIPLocation(r io.Reader) (LocationInfo, error) {
	var data ipWhoIsResponse
	if err := json.NewDecoder(r).Decode(&data); err != nil {
		return LocationInfo{}, fmt.Errorf("decode response: %w", err)
	}
	if !data.Success {
		if data.Message == "" {
			data.Message = "service returned success=false"
		}
		return LocationInfo{}, errors.New(data.Message)
	}
	if !validCoordinates(data.Latitude, data.Longitude) {
		return LocationInfo{}, fmt.Errorf("invalid coordinates lat=%.4f lon=%.4f", data.Latitude, data.Longitude)
	}
	return LocationInfo{
		Latitude:      data.Latitude,
		Longitude:     data.Longitude,
		Source:        LocationSourceIP,
		LocationLabel: ipLocationLabel(data),
	}, nil
}

func ipLocationLabel(data ipWhoIsResponse) string {
	parts := make([]string, 0, 3)
	for _, value := range []string{data.City, data.Region, data.Country} {
		value = strings.TrimSpace(value)
		if value != "" {
			parts = append(parts, value)
		}
	}
	return strings.Join(parts, ", ")
}

func validCoordinates(lat, lon float64) bool {
	return lat >= -90 && lat <= 90 && lon >= -180 && lon <= 180 && (lat != 0 || lon != 0)
}
