package theme

import (
	"errors"
	"fmt"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/registry"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
)

var (
	themeSwitchUser32              = windows.NewLazySystemDLL("user32.dll")
	pUpdatePerUserSystemParameters = themeSwitchUser32.NewProc("UpdatePerUserSystemParameters")
	pSendMessageTimeout            = themeSwitchUser32.NewProc("SendMessageTimeoutW")
	themeOperationMu               sync.Mutex
)

const regPath = `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`

var themeModeValueNames = [...]string{"AppsUseLightTheme", "SystemUsesLightTheme"}

type themePreferenceStore interface {
	GetIntegerValue(name string) (value uint64, valueType uint32, err error)
	SetDWordValue(name string, value uint32) error
	DeleteValue(name string) error
}

type themePreferenceValue struct {
	value  uint32
	exists bool
}

// DetectSupport verifies that the current Windows profile exposes writable
// light/dark Personalize settings. It deliberately does not change either
// value, so capability detection cannot cause a visible theme switch.
func DetectSupport() error {
	k, err := registry.OpenKey(registry.CURRENT_USER, regPath, registry.QUERY_VALUE|registry.SET_VALUE)
	if err != nil {
		return fmt.Errorf("open Windows Personalize settings: %w", err)
	}
	defer k.Close()
	for _, name := range themeModeValueNames {
		_, err := readThemePreferenceValue(k, name)
		if err != nil {
			return fmt.Errorf("read %s: %w", name, err)
		}
	}
	return nil
}

// Mode is the target theme mode.
type Mode int

const (
	ModeLight Mode = iota
	ModeDark
)

// Switch sets the Windows app and system theme.
func Switch(mode Mode) error {
	themeOperationMu.Lock()
	defer themeOperationMu.Unlock()

	var val uint32
	if mode == ModeLight {
		val = 1
	}
	k, err := registry.OpenKey(registry.CURRENT_USER, regPath, registry.QUERY_VALUE|registry.SET_VALUE)
	if err != nil {
		return err
	}
	defer k.Close()
	if err := applyThemeModeValues(k, val); err != nil {
		return err
	}

	notifyThemeChanged()
	return nil
}

// applyThemeModeValues treats the app and system preferences as one logical
// update. Registry writes are not transactional, so any write or verification
// failure restores the values observed before the operation.
func applyThemeModeValues(store themePreferenceStore, value uint32) error {
	desired := make(map[string]uint32, len(themeModeValueNames))
	for _, name := range themeModeValueNames {
		desired[name] = value
	}
	return applyThemePreferenceValues(store, desired)
}

func applyThemePreferenceValues(store themePreferenceStore, desired map[string]uint32) error {
	if err := validateThemePreferenceTargets(desired); err != nil {
		return err
	}

	previous := make(map[string]themePreferenceValue, len(themeModeValueNames))
	for _, name := range themeModeValueNames {
		current, err := readThemePreferenceValue(store, name)
		if err != nil {
			return fmt.Errorf("read %s before theme switch: %w", name, err)
		}
		previous[name] = current
	}

	attempted := make([]string, 0, len(themeModeValueNames))
	for _, name := range themeModeValueNames {
		attempted = append(attempted, name)
		if err := store.SetDWordValue(name, desired[name]); err != nil {
			return themeModeUpdateError(store, previous, attempted, fmt.Errorf("set %s: %w", name, err))
		}
	}

	for _, name := range themeModeValueNames {
		actual, valueType, err := store.GetIntegerValue(name)
		if err != nil {
			return themeModeUpdateError(store, previous, attempted, fmt.Errorf("verify %s: %w", name, err))
		}
		if valueType != registry.DWORD || uint32(actual) != desired[name] {
			return themeModeUpdateError(store, previous, attempted,
				fmt.Errorf("verify %s: Windows did not retain the requested value", name))
		}
	}
	return nil
}

func validateThemePreferenceTargets(desired map[string]uint32) error {
	for _, name := range themeModeValueNames {
		target, ok := desired[name]
		if !ok || target > 1 {
			return fmt.Errorf("target %s is not a valid Windows light/dark setting", name)
		}
	}
	return nil
}

func readThemePreferenceValue(store themePreferenceStore, name string) (themePreferenceValue, error) {
	current, valueType, err := store.GetIntegerValue(name)
	if errors.Is(err, registry.ErrNotExist) {
		return themePreferenceValue{}, nil
	}
	if err != nil {
		return themePreferenceValue{}, err
	}
	if valueType != registry.DWORD || current > 1 {
		return themePreferenceValue{}, errors.New("invalid Windows light/dark setting")
	}
	return themePreferenceValue{value: uint32(current), exists: true}, nil
}

func themeModeUpdateError(store themePreferenceStore, previous map[string]themePreferenceValue, attempted []string, updateErr error) error {
	var rollbackErrs []error
	for i := len(attempted) - 1; i >= 0; i-- {
		name := attempted[i]
		var err error
		if previous[name].exists {
			err = store.SetDWordValue(name, previous[name].value)
		} else {
			err = store.DeleteValue(name)
			if errors.Is(err, registry.ErrNotExist) {
				err = nil
			}
		}
		if err != nil {
			rollbackErrs = append(rollbackErrs, fmt.Errorf("restore %s: %w", name, err))
		}
	}
	if len(rollbackErrs) == 0 {
		return updateErr
	}
	return errors.Join(updateErr, fmt.Errorf("roll back partial theme switch: %w", errors.Join(rollbackErrs...)))
}

// Refresh repairs partially updated Windows 11 shell surfaces and then asks
// every top-level window to re-read the current theme settings.
func Refresh() error {
	themeOperationMu.Lock()
	defer themeOperationMu.Unlock()

	var repairErr error
	if windows.RtlGetVersion().BuildNumber >= windows11FullDWMRefreshBuild {
		repairErr = refreshDWMColorization()
	}
	notifyThemeChanged()
	return repairErr
}

type themeChangeNotification struct {
	message uint32
	token   string
}

func themeChangeNotifications() []themeChangeNotification {
	const (
		wmThemeChanged   = 0x031A
		wmSysColorChange = 0x0015
		wmSettingChange  = 0x001A
	)
	return []themeChangeNotification{
		{message: wmSettingChange, token: "ImmersiveColorSet"},
		{message: wmSettingChange, token: "WindowsThemeElement"},
		{message: wmSettingChange, token: "Policy"},
		{message: wmSysColorChange},
		{message: wmThemeChanged},
	}
}

func notifyThemeChanged() {
	const (
		hwndBroadcast   = 0xFFFF
		smtoAbortIfHung = 0x0002
	)

	pUpdatePerUserSystemParameters.Call(0, 1)

	// Notify every top-level window, including Explorer's taskbars on each
	// display. Windows components do not all listen for the same token, so keep
	// the complete set but deliver each event exactly once. The previous
	// asynchronous-plus-synchronous pair left duplicate messages queued after
	// the app had already committed its new frame, producing late hover redraws.
	for _, notification := range themeChangeNotifications() {
		var lParam uintptr
		if notification.token != "" {
			ptr, err := windows.UTF16PtrFromString(notification.token)
			if err != nil {
				continue
			}
			lParam = uintptr(unsafe.Pointer(ptr))
		}
		pSendMessageTimeout.Call(hwndBroadcast, uintptr(notification.message), 0, lParam, smtoAbortIfHung, 250, 0)
	}
}

// Current returns the current theme mode.
func Current() Mode {
	k, err := registry.OpenKey(registry.CURRENT_USER, regPath, registry.QUERY_VALUE)
	if err != nil {
		return currentModeFromImmersiveTheme()
	}
	defer k.Close()
	preference, err := readThemePreferenceValue(k, "AppsUseLightTheme")
	if err == nil && preference.exists {
		if preference.value == 0 {
			return ModeDark
		}
		return ModeLight
	}
	return currentModeFromImmersiveTheme()
}

func currentModeFromImmersiveTheme() Mode {
	if dark, supported := darkmode.AppsUseDark(); supported && dark {
		return ModeDark
	}
	return ModeLight
}
