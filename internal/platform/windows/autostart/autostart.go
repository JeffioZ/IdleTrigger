// Package autostart manages the Windows registry Run key so the user
// can enable or disable IdleTrigger auto-start on login.
package autostart

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/windows/registry"
)

const (
	runKey  = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
	valName = "IdleTrigger"
)

// IsEnabled returns true when the auto-start registry entry is present.
func IsEnabled() (bool, error) {
	enabled, _, err := read()
	return enabled, err
}

// EnsureCurrent keeps an existing auto-start entry pointed at this executable.
// It does not enable auto-start when the entry is absent.
func EnsureCurrent() (enabled bool, updated bool, err error) {
	enabled, current, err := read()
	if err != nil || !enabled {
		return enabled, false, err
	}
	want, err := currentCommandLine()
	if err != nil {
		return enabled, false, err
	}
	if current == want {
		return enabled, false, nil
	}
	if err := write(want); err != nil {
		return enabled, false, err
	}
	return enabled, true, nil
}

// Enable writes the auto-start registry entry pointing to the current
// executable with a "--minimized" argument so it starts directly to the tray.
func Enable() error {
	cmd, err := currentCommandLine()
	if err != nil {
		return err
	}
	return write(cmd)
}

// Disable removes the auto-start registry entry.
func Disable() error {
	k, err := registry.OpenKey(registry.CURRENT_USER, runKey, registry.SET_VALUE)
	if err != nil {
		if err == registry.ErrNotExist {
			return nil
		}
		return fmt.Errorf("open Run key: %w", err)
	}
	defer func() { _ = k.Close() }()

	err = k.DeleteValue(valName)
	if err == registry.ErrNotExist {
		return nil
	}
	return err
}

func read() (bool, string, error) {
	k, err := registry.OpenKey(registry.CURRENT_USER, runKey, registry.QUERY_VALUE)
	if err != nil {
		if err == registry.ErrNotExist {
			return false, "", nil
		}
		return false, "", err
	}
	defer func() { _ = k.Close() }()

	value, _, err := k.GetStringValue(valName)
	if err == registry.ErrNotExist {
		return false, "", nil
	}
	return err == nil, value, err
}

func write(cmd string) error {
	k, _, err := registry.CreateKey(registry.CURRENT_USER, runKey, registry.SET_VALUE)
	if err != nil {
		return fmt.Errorf("create or open Run key: %w", err)
	}
	defer func() { _ = k.Close() }()

	return k.SetStringValue(valName, cmd)
}

func currentCommandLine() (string, error) {
	exe, err := exePath()
	if err != nil {
		return "", err
	}
	return commandLine(exe), nil
}

func exePath() (string, error) {
	p, err := os.Executable()
	if err != nil {
		return "", err
	}
	return filepath.Clean(p), nil
}

func commandLine(exe string) string {
	return fmt.Sprintf("\"%s\" --minimized", exe)
}
