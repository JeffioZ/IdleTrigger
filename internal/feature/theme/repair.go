package theme

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/registry"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
)

const (
	windows11FullDWMRefreshBuild = 22621

	coinitApartmentThreaded = 0x2
	clsctxAll               = 0x17

	themeApplyIgnoreBackground   = 1 << 0
	themeApplyIgnoreCursor       = 1 << 1
	themeApplyIgnoreDesktopIcons = 1 << 2
	themeApplyIgnoreSound        = 1 << 4
	themeApplyIgnoreScreensaver  = 1 << 5

	themeManagerInitMethod            = 3
	themeManagerGetCurrentThemeMethod = 11
	themeManagerSetCurrentThemeMethod = 12
	themeManagerGetCustomThemeMethod  = 13
	themeManagerUpdateCustomMethod    = 26
	legacyThemeManagerApplyMethod     = 4
)

const (
	themesRegistryPath    = `Software\Microsoft\Windows\CurrentVersion\Themes`
	themesPersonalizePath = `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
	explorerAccentPath    = `Software\Microsoft\Windows\CurrentVersion\Explorer\Accent`
	dwmRegistryPath       = `Software\Microsoft\Windows\DWM`
)

var (
	themeRepairOle32              = windows.NewLazySystemDLL("ole32.dll")
	themeRepairOleAut32           = windows.NewLazySystemDLL("oleaut32.dll")
	pCoCreateInstance             = themeRepairOle32.NewProc("CoCreateInstance")
	pSysAllocString               = themeRepairOleAut32.NewProc("SysAllocString")
	pSysFreeString                = themeRepairOleAut32.NewProc("SysFreeString")
	themeManagerClassID           = windows.GUID{Data1: 0x9324da94, Data2: 0x50ec, Data3: 0x4a14, Data4: [8]byte{0xa7, 0x70, 0xe9, 0x0c, 0xa0, 0x3e, 0x7c, 0x8f}}
	themeManagerInterfaceID       = windows.GUID{Data1: 0xc1e8c83e, Data2: 0x845d, Data3: 0x4d95, Data4: [8]byte{0x81, 0xdb, 0xe2, 0x83, 0xfd, 0xff, 0xc0, 0x00}}
	legacyThemeManagerClassID     = windows.GUID{Data1: 0xc04b329e, Data2: 0x5823, Data3: 0x4415, Data4: [8]byte{0x9c, 0x93, 0xba, 0x44, 0x68, 0x89, 0x47, 0xb0}}
	legacyThemeManagerInterfaceID = windows.GUID{Data1: 0x0646ebbe, Data2: 0xc1b7, Data3: 0x4045, Data4: [8]byte{0x8f, 0xd0, 0xff, 0xd6, 0x5d, 0x3f, 0xc7, 0x92}}
)

type themeManager2 struct {
	vtable *[themeManagerUpdateCustomMethod + 1]uintptr
}

type legacyThemeManager struct {
	vtable *[legacyThemeManagerApplyMethod + 1]uintptr
}

type themeFilePatch struct {
	displayName             string
	themeID                 string
	appMode                 string
	systemMode              string
	colorization            uint32
	setColorization         bool
	disableAutoColorization bool
}

// refreshDWMColorization follows Auto Dark Mode's full DWM refresh strategy:
// apply a copy of the current theme with a one-digit accent-color nudge, wait
// for DWM to commit it, and select the original theme again with color changes
// enabled. The original theme selection, accent color, and independent
// app/system modes are restored without applying a second disposable theme.
// Reference: AutoDarkModeSvc/Handlers/DwmRefreshHandler.cs in
// https://github.com/AutoDarkMode/Windows-Auto-Night-Mode.
func refreshDWMColorization() error {
	source, err := currentThemeSnapshot()
	if err != nil {
		return fmt.Errorf("prepare full DWM refresh: %w", err)
	}
	appsLight, systemLight, err := currentThemeModes(source)
	if err != nil {
		return fmt.Errorf("read current theme modes: %w", err)
	}
	accent, err := currentAccentColor()
	if err != nil {
		return fmt.Errorf("read current accent color: %w", err)
	}

	refreshID, err := windows.GenerateGUID()
	if err != nil {
		return fmt.Errorf("create DWM refresh theme id: %w", err)
	}
	appMode := modeName(appsLight)
	systemMode := modeName(systemLight)
	refreshTheme, err := patchThemeFile(source, themeFilePatch{
		displayName: "IdleTrigger DWM Refresh", themeID: themeIDString(refreshID),
		appMode: appMode, systemMode: systemMode,
		colorization: nudgedColorization(accent), setColorization: true, disableAutoColorization: true,
	})
	if err != nil {
		return fmt.Errorf("build DWM refresh theme: %w", err)
	}

	localAppData, err := os.UserCacheDir()
	if err != nil {
		return fmt.Errorf("locate Windows themes directory: %w", err)
	}
	userThemesDir := filepath.Join(localAppData, "Microsoft", "Windows", "Themes")
	if err := os.MkdirAll(userThemesDir, 0o700); err != nil {
		return fmt.Errorf("create Windows themes directory: %w", err)
	}
	// Keep one stable helper theme in the Windows user-theme directory, matching
	// Auto Dark Mode's DWM refresh strategy. Applying a disposable theme from
	// %TEMP% makes IThemeManager import a copy here and later recycle that copy.
	refreshPath := filepath.Join(userThemesDir, "IdleTriggerDwmRefresh.theme")
	if err := os.WriteFile(refreshPath, refreshTheme, 0o600); err != nil {
		return fmt.Errorf("write DWM refresh theme: %w", err)
	}

	if err := applyColorizationRefresh(refreshPath, appsLight, systemLight); err != nil {
		return err
	}
	mylog.Info("Theme repair: full DWM colorization refresh completed")
	return nil
}

func currentThemeSnapshot() ([]byte, error) {
	syncErr := syncCustomThemeMetadata()
	localAppData, err := os.UserCacheDir()
	if err != nil {
		return nil, err
	}
	customPath := filepath.Join(localAppData, "Microsoft", "Windows", "Themes", "Custom.theme")
	paths := make([]string, 0, 2)
	key, pathErr := registry.OpenKey(registry.CURRENT_USER, themesRegistryPath, registry.QUERY_VALUE)
	if pathErr == nil {
		if currentPath, _, currentErr := key.GetStringValue("CurrentTheme"); currentErr == nil && currentPath != "" {
			paths = append(paths, currentPath)
		} else if currentErr != nil && !errors.Is(currentErr, registry.ErrNotExist) {
			pathErr = currentErr
		}
		key.Close()
	}
	paths = append(paths, customPath)
	data, readErr := readThemeSnapshotFromPaths(paths)
	if readErr == nil {
		return data, nil
	}
	var snapshotErrs []error
	if syncErr != nil {
		snapshotErrs = append(snapshotErrs, fmt.Errorf("synchronize current Windows theme metadata: %w", syncErr))
	}
	if pathErr != nil {
		snapshotErrs = append(snapshotErrs, fmt.Errorf("read CurrentTheme registry value: %w", pathErr))
	}
	snapshotErrs = append(snapshotErrs, readErr)
	return nil, errors.Join(snapshotErrs...)
}

func syncCustomThemeMetadata() error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	initialized, err := initializeThemeCOM()
	if err != nil {
		return err
	}
	if initialized {
		defer windows.CoUninitialize()
	}
	manager, err := createThemeManager2()
	if err != nil {
		return err
	}
	defer manager.release()
	if err := manager.init(); err != nil {
		return err
	}
	currentTheme, err := manager.currentTheme()
	if err != nil {
		return err
	}
	customTheme, err := manager.customTheme()
	if err != nil {
		return err
	}
	if currentTheme == customTheme {
		return manager.updateCustomTheme()
	}
	return nil
}

func readThemeSnapshotFromPaths(paths []string) ([]byte, error) {
	seen := make(map[string]struct{}, len(paths))
	var readErrs []error
	for _, path := range paths {
		if path == "" {
			continue
		}
		cleanPath := filepath.Clean(path)
		key := strings.ToLower(cleanPath)
		if _, exists := seen[key]; exists {
			continue
		}
		seen[key] = struct{}{}
		data, err := os.ReadFile(cleanPath)
		if err != nil {
			readErrs = append(readErrs, fmt.Errorf("read %s: %w", cleanPath, err))
			continue
		}
		if !themeFileHasSection(data, "Theme") || !themeFileHasSection(data, "VisualStyles") {
			readErrs = append(readErrs, fmt.Errorf("read %s: incomplete theme file", cleanPath))
			continue
		}
		return data, nil
	}
	if len(readErrs) == 0 {
		return nil, errors.New("no current Windows theme file path is available")
	}
	return nil, errors.Join(readErrs...)
}

func themeFileHasSection(source []byte, section string) bool {
	want := "[" + section + "]"
	for _, line := range strings.Split(strings.ReplaceAll(string(source), "\r\n", "\n"), "\n") {
		if strings.EqualFold(strings.TrimSpace(line), want) {
			return true
		}
	}
	return false
}

func currentThemeModes(source []byte) (appsLight, systemLight bool, err error) {
	key, err := registry.OpenKey(registry.CURRENT_USER, themesPersonalizePath, registry.QUERY_VALUE)
	if err != nil {
		return false, false, err
	}
	defer key.Close()
	appsLight, err = currentThemeMode(key, "AppsUseLightTheme", "AppMode", source)
	if err != nil {
		return false, false, err
	}
	systemLight, err = currentThemeMode(key, "SystemUsesLightTheme", "SystemMode", source)
	if err != nil {
		return false, false, err
	}
	return appsLight, systemLight, nil
}

func currentThemeMode(store themePreferenceStore, registryName, themeName string, source []byte) (bool, error) {
	preference, err := readThemePreferenceValue(store, registryName)
	if err != nil {
		return false, err
	}
	if preference.exists {
		return preference.value != 0, nil
	}
	if light, ok := themeModeFromThemeFile(source, themeName); ok {
		return light, nil
	}
	// An absent Personalize value means the Windows default, which is light.
	return true, nil
}

func themeModeFromThemeFile(source []byte, name string) (bool, bool) {
	inVisualStyles := false
	for _, line := range strings.Split(strings.ReplaceAll(string(source), "\r\n", "\n"), "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "[") && strings.HasSuffix(trimmed, "]") {
			inVisualStyles = strings.EqualFold(trimmed, "[VisualStyles]")
			continue
		}
		if !inVisualStyles {
			continue
		}
		key, value, found := strings.Cut(trimmed, "=")
		if !found || !strings.EqualFold(strings.TrimSpace(key), name) {
			continue
		}
		switch strings.ToLower(strings.TrimSpace(value)) {
		case "light":
			return true, true
		case "dark":
			return false, true
		default:
			return false, false
		}
	}
	return false, false
}

func currentAccentColor() (uint32, error) {
	key, err := registry.OpenKey(registry.CURRENT_USER, explorerAccentPath, registry.QUERY_VALUE)
	if err == nil {
		palette, _, paletteErr := key.GetBinaryValue("AccentPalette")
		key.Close()
		if paletteErr == nil && len(palette) >= 16 {
			return 0xff000000 | uint32(palette[12])<<16 | uint32(palette[13])<<8 | uint32(palette[14]), nil
		}
	}

	key, err = registry.OpenKey(registry.CURRENT_USER, dwmRegistryPath, registry.QUERY_VALUE)
	if err != nil {
		return 0, err
	}
	defer key.Close()
	color, _, err := key.GetIntegerValue("ColorizationColor")
	if err != nil {
		return 0, err
	}
	return 0xff000000 | uint32(color)&0x00ffffff, nil
}

func modeName(light bool) string {
	if light {
		return "Light"
	}
	return "Dark"
}

func nudgedColorization(color uint32) uint32 {
	if color&0xf >= 9 {
		return color - 1
	}
	return color + 1
}

func themeIDString(id windows.GUID) string {
	value := strings.ToUpper(id.String())
	if strings.HasPrefix(value, "{") && strings.HasSuffix(value, "}") {
		return value
	}
	return "{" + value + "}"
}

func patchThemeFile(source []byte, patch themeFilePatch) ([]byte, error) {
	newline := "\n"
	if strings.Contains(string(source), "\r\n") {
		newline = "\r\n"
	}
	lines := strings.Split(string(source), newline)
	var err error
	lines, err = setThemeSectionValues(lines, "Theme", map[string]string{
		"DisplayName": patch.displayName,
		"ThemeId":     patch.themeID,
	})
	if err != nil {
		return nil, err
	}
	visualValues := map[string]string{
		"AppMode":    patch.appMode,
		"SystemMode": patch.systemMode,
	}
	if patch.setColorization {
		visualValues["ColorizationColor"] = fmt.Sprintf("0X%08X", patch.colorization)
	}
	if patch.disableAutoColorization {
		visualValues["AutoColorization"] = "0"
	}
	lines, err = setThemeSectionValues(lines, "VisualStyles", visualValues)
	if err != nil {
		return nil, err
	}
	return []byte(strings.Join(lines, newline)), nil
}

func setThemeSectionValues(lines []string, section string, values map[string]string) ([]string, error) {
	start := -1
	end := len(lines)
	wantSection := "[" + section + "]"
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if start < 0 {
			if strings.EqualFold(trimmed, wantSection) {
				start = i + 1
			}
			continue
		}
		if strings.HasPrefix(trimmed, "[") && strings.HasSuffix(trimmed, "]") {
			end = i
			break
		}
	}
	if start < 0 {
		return nil, fmt.Errorf("theme section %q is missing", section)
	}

	remaining := make(map[string]string, len(values))
	for key, value := range values {
		remaining[strings.ToLower(key)] = value
	}
	for i := start; i < end; i++ {
		key, _, found := strings.Cut(lines[i], "=")
		if !found {
			continue
		}
		normalized := strings.ToLower(strings.TrimSpace(key))
		if value, ok := remaining[normalized]; ok {
			lines[i] = strings.TrimSpace(key) + "=" + value
			delete(remaining, normalized)
		}
	}
	if len(remaining) == 0 {
		return lines, nil
	}
	insert := make([]string, 0, len(remaining))
	for _, key := range sortedThemeKeys(values) {
		if value, ok := remaining[strings.ToLower(key)]; ok {
			insert = append(insert, key+"="+value)
		}
	}
	lines = append(lines, make([]string, len(insert))...)
	copy(lines[end+len(insert):], lines[end:])
	copy(lines[end:], insert)
	return lines, nil
}

func sortedThemeKeys(values map[string]string) []string {
	order := []string{"DisplayName", "ThemeId", "AutoColorization", "ColorizationColor", "AppMode", "SystemMode"}
	keys := make([]string, 0, len(values))
	for _, candidate := range order {
		for key := range values {
			if strings.EqualFold(key, candidate) {
				keys = append(keys, key)
				break
			}
		}
	}
	return keys
}

func applyColorizationRefresh(refreshPath string, appsLight, systemLight bool) (returnErr error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	initialized, err := initializeThemeCOM()
	if err != nil {
		return err
	}
	if initialized {
		defer windows.CoUninitialize()
	}
	manager, err := createThemeManager2()
	if err != nil {
		return err
	}
	defer manager.release()
	if err := manager.init(); err != nil {
		return err
	}
	legacyManager, err := createLegacyThemeManager()
	if err != nil {
		return err
	}
	defer legacyManager.release()
	originalTheme, err := manager.currentTheme()
	if err != nil {
		return err
	}

	const colorOnlyFlags = themeApplyIgnoreBackground | themeApplyIgnoreCursor | themeApplyIgnoreDesktopIcons |
		themeApplyIgnoreSound | themeApplyIgnoreScreensaver
	restoreOriginal := true
	defer func() {
		if restoreOriginal {
			if err := restoreThemeAfterColorization(manager, originalTheme, colorOnlyFlags, appsLight, systemLight); err != nil {
				recoveryErr := fmt.Errorf("recover from interrupted DWM refresh: %w", err)
				if returnErr == nil {
					returnErr = recoveryErr
				} else {
					returnErr = errors.Join(returnErr, recoveryErr)
				}
			}
		}
	}()

	if err := legacyManager.applyTheme(refreshPath); err != nil {
		return fmt.Errorf("apply DWM refresh theme: %w", err)
	}
	time.Sleep(time.Second)
	if err := restoreThemeAfterColorization(manager, originalTheme, colorOnlyFlags, appsLight, systemLight); err != nil {
		return err
	}
	restoreOriginal = false
	return nil
}

func restoreThemeAfterColorization(manager *themeManager2, originalTheme int32, colorOnlyFlags uintptr,
	appsLight, systemLight bool) error {
	var restoreErrs []error
	if err := manager.setCurrentTheme(originalTheme, colorOnlyFlags); err != nil {
		restoreErrs = append(restoreErrs, fmt.Errorf("restore original Windows theme selection: %w", err))
	}
	if err := restoreThemePreferences(appsLight, systemLight); err != nil {
		restoreErrs = append(restoreErrs, fmt.Errorf("restore current app and system theme modes: %w", err))
	}
	return errors.Join(restoreErrs...)
}

func restoreThemePreferences(appsLight, systemLight bool) error {
	key, err := registry.OpenKey(registry.CURRENT_USER, themesPersonalizePath, registry.QUERY_VALUE|registry.SET_VALUE)
	if err != nil {
		return err
	}
	defer key.Close()
	desired := map[string]uint32{
		"AppsUseLightTheme":    boolDWORD(appsLight),
		"SystemUsesLightTheme": boolDWORD(systemLight),
	}
	return restoreThemePreferenceValues(key, desired)
}

// restoreThemePreferenceValues is intentionally best effort rather than
// transactional. During recovery, retaining each value that was restored
// successfully is safer than rolling it back to a possibly temporary theme.
func restoreThemePreferenceValues(store themePreferenceStore, desired map[string]uint32) error {
	if err := validateThemePreferenceTargets(desired); err != nil {
		return err
	}

	var restoreErrs []error
	for _, name := range themeModeValueNames {
		if err := store.SetDWordValue(name, desired[name]); err != nil {
			restoreErrs = append(restoreErrs, fmt.Errorf("set %s: %w", name, err))
		}
	}
	for _, name := range themeModeValueNames {
		actual, valueType, err := store.GetIntegerValue(name)
		if err != nil {
			restoreErrs = append(restoreErrs, fmt.Errorf("verify %s: %w", name, err))
			continue
		}
		if valueType != registry.DWORD || uint32(actual) != desired[name] {
			restoreErrs = append(restoreErrs,
				fmt.Errorf("verify %s: Windows did not retain the restored value", name))
		}
	}
	return errors.Join(restoreErrs...)
}

func boolDWORD(value bool) uint32 {
	if value {
		return 1
	}
	return 0
}

func initializeThemeCOM() (bool, error) {
	err := windows.CoInitializeEx(0, coinitApartmentThreaded)
	if err == nil {
		return true, nil
	}
	var errno syscall.Errno
	if errors.As(err, &errno) && errno == 1 { // S_FALSE: COM was already initialized on this thread.
		return true, nil
	}
	return false, fmt.Errorf("initialize theme COM apartment: %w", err)
}

func createThemeManager2() (*themeManager2, error) {
	var manager *themeManager2
	hr, _, _ := pCoCreateInstance.Call(
		uintptr(unsafe.Pointer(&themeManagerClassID)),
		0,
		clsctxAll,
		uintptr(unsafe.Pointer(&themeManagerInterfaceID)),
		uintptr(unsafe.Pointer(&manager)),
	)
	if hresultFailed(hr) || manager == nil {
		return nil, hresultError("create IThemeManager2", hr)
	}
	return manager, nil
}

func createLegacyThemeManager() (*legacyThemeManager, error) {
	var manager *legacyThemeManager
	hr, _, _ := pCoCreateInstance.Call(
		uintptr(unsafe.Pointer(&legacyThemeManagerClassID)),
		0,
		clsctxAll,
		uintptr(unsafe.Pointer(&legacyThemeManagerInterfaceID)),
		uintptr(unsafe.Pointer(&manager)),
	)
	if hresultFailed(hr) || manager == nil {
		return nil, hresultError("create IThemeManager", hr)
	}
	return manager, nil
}

func (m *themeManager2) init() error {
	hr, _, _ := syscall.SyscallN(m.vtable[themeManagerInitMethod], uintptr(unsafe.Pointer(m)), 0)
	if hresultFailed(hr) {
		return hresultError("initialize IThemeManager2", hr)
	}
	return nil
}

func (m *themeManager2) currentTheme() (int32, error) {
	var index int32
	hr, _, _ := syscall.SyscallN(
		m.vtable[themeManagerGetCurrentThemeMethod],
		uintptr(unsafe.Pointer(m)),
		uintptr(unsafe.Pointer(&index)),
	)
	if hresultFailed(hr) {
		return 0, hresultError("get current Windows theme", hr)
	}
	return index, nil
}

func (m *themeManager2) customTheme() (int32, error) {
	var index int32
	hr, _, _ := syscall.SyscallN(
		m.vtable[themeManagerGetCustomThemeMethod],
		uintptr(unsafe.Pointer(m)),
		uintptr(unsafe.Pointer(&index)),
	)
	if hresultFailed(hr) {
		return 0, hresultError("get custom Windows theme", hr)
	}
	return index, nil
}

func (m *themeManager2) updateCustomTheme() error {
	hr, _, _ := syscall.SyscallN(
		m.vtable[themeManagerUpdateCustomMethod],
		uintptr(unsafe.Pointer(m)),
	)
	if hresultFailed(hr) {
		return hresultError("update custom Windows theme", hr)
	}
	return nil
}

func (m *themeManager2) setCurrentTheme(index int32, applyFlags uintptr) error {
	hr, _, _ := syscall.SyscallN(
		m.vtable[themeManagerSetCurrentThemeMethod],
		uintptr(unsafe.Pointer(m)),
		0,
		uintptr(index),
		1,
		applyFlags,
		0,
	)
	if hr != 0 {
		return hresultError("set current Windows theme", hr)
	}
	return nil
}

func (m *legacyThemeManager) applyTheme(path string) error {
	bstr, err := allocBSTR(path)
	if err != nil {
		return err
	}
	defer pSysFreeString.Call(bstr)
	hr, _, _ := syscall.SyscallN(
		m.vtable[legacyThemeManagerApplyMethod],
		uintptr(unsafe.Pointer(m)),
		bstr,
	)
	if hresultFailed(hr) {
		return hresultError("apply Windows theme", hr)
	}
	return nil
}

func (m *themeManager2) release() {
	if m == nil {
		return
	}
	syscall.SyscallN(m.vtable[2], uintptr(unsafe.Pointer(m)))
}

func (m *legacyThemeManager) release() {
	if m == nil {
		return
	}
	syscall.SyscallN(m.vtable[2], uintptr(unsafe.Pointer(m)))
}

func allocBSTR(value string) (uintptr, error) {
	ptr, err := windows.UTF16PtrFromString(value)
	if err != nil {
		return 0, fmt.Errorf("encode theme path: %w", err)
	}
	bstr, _, _ := pSysAllocString.Call(uintptr(unsafe.Pointer(ptr)))
	if bstr == 0 {
		return 0, errors.New("allocate theme path BSTR")
	}
	return bstr, nil
}

func hresultFailed(hr uintptr) bool {
	return int32(uint32(hr)) < 0
}

func hresultError(operation string, hr uintptr) error {
	return fmt.Errorf("%s: HRESULT 0x%08X", operation, uint32(hr))
}
