package config

import (
	"bytes"
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
	contents := renderAnnotatedTOML(cfg)
	if expectedRevision != "" {
		// Idempotent saves should not create short-lived files. Validate the
		// caller's disk snapshot first so stale writers are still rejected.
		current, revision, err := readAtRevision(p, expectedRevision)
		if err != nil {
			return "", err
		}
		if bytes.Equal(current, []byte(contents)) {
			return revision, nil
		}
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
		if _, _, err := readAtRevision(p, expectedRevision); err != nil {
			return "", err
		}
	}
	if err := os.Rename(tmpPath, p); err != nil {
		return "", fmt.Errorf("replace config: %w", err)
	}
	ok = true
	return configRevision([]byte(contents)), nil
}

func readAtRevision(p, expectedRevision string) ([]byte, string, error) {
	current, err := os.ReadFile(p)
	if errors.Is(err, os.ErrNotExist) {
		return nil, "", ErrConfigChanged
	}
	if err != nil {
		return nil, "", fmt.Errorf("verify current config: %w", err)
	}
	revision := configRevision(current)
	if revision != expectedRevision {
		return nil, "", ErrConfigChanged
	}
	return current, revision, nil
}

func configRevision(data []byte) string {
	hash := fnv.New64a()
	_, _ = hash.Write(data)
	return strconv.FormatUint(hash.Sum64(), 16)
}
