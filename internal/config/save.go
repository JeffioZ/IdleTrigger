package config

import (
	"errors"
	"fmt"
	"hash/fnv"
	"os"
	"path/filepath"
	"strconv"
)

var (
	ErrConfigChanged          = errors.New("configuration changed on disk")
	ErrConfigRecoveryRequired = errors.New("configuration must be repaired and reloaded before it can be saved")
)

// Save atomically writes the configuration to disk.
func Save(cfg Config) error {
	p, err := Path()
	if err != nil {
		return err
	}
	return saveTo(p, cfg)
}

// SaveAtRevision atomically writes cfg only when the current file still
// matches the revision that was loaded by the caller. It closes the stale UI
// snapshot window without relying on the polling config watcher.
func SaveAtRevision(cfg Config, expectedRevision string) (string, error) {
	p, err := Path()
	if err != nil {
		return "", err
	}
	return saveToAtRevision(p, cfg, expectedRevision)
}

func saveTo(p string, cfg Config) error {
	_, err := saveToAtRevision(p, cfg, "")
	return err
}

func saveToAtRevision(p string, cfg Config, expectedRevision string) (string, error) {
	if err := cfg.Validate(); err != nil {
		return "", err
	}
	f, err := os.CreateTemp(filepath.Dir(p), ".IdleTrigger-*.toml.tmp")
	if err != nil {
		return "", fmt.Errorf("create temporary config file: %w", err)
	}
	tmpPath := f.Name()
	ok := false
	defer func() {
		_ = f.Close()
		if !ok {
			_ = os.Remove(tmpPath)
		}
	}()

	contents := renderAnnotatedTOML(cfg)
	if _, err := f.WriteString(contents); err != nil {
		return "", fmt.Errorf("encode config: %w", err)
	}
	if err := f.Sync(); err != nil {
		return "", fmt.Errorf("sync config: %w", err)
	}
	if err := f.Close(); err != nil {
		return "", fmt.Errorf("close config: %w", err)
	}
	if expectedRevision != "" {
		current, err := os.ReadFile(p)
		if errors.Is(err, os.ErrNotExist) {
			return "", ErrConfigChanged
		}
		if err != nil {
			return "", fmt.Errorf("verify current config: %w", err)
		}
		if configRevision(current) != expectedRevision {
			return "", ErrConfigChanged
		}
	}
	if err := os.Rename(tmpPath, p); err != nil {
		return "", fmt.Errorf("replace config: %w", err)
	}
	ok = true
	return configRevision([]byte(contents)), nil
}

func configRevision(data []byte) string {
	hash := fnv.New64a()
	_, _ = hash.Write(data)
	return strconv.FormatUint(hash.Sum64(), 16)
}
