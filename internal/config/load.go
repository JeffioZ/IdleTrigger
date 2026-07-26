package config

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"

	"github.com/BurntSushi/toml"
)

// Path returns the full path to the config file next to the executable.
func Path() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("cannot locate executable: %w", err)
	}
	return filepath.Join(filepath.Dir(exe), "IdleTrigger.toml"), nil
}

// Load reads and parses the config file.  If the file does not exist the
// factory defaults are returned and the file is created.
func Load() (Config, error) {
	p, err := Path()
	if err != nil {
		cfg := DefaultConfig()
		cfg.LoadError = err.Error()
		return cfg, err
	}
	return loadFrom(p)
}

func loadFrom(p string) (Config, error) {
	cfg := DefaultConfig()
	data, err := os.ReadFile(p)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			revision, saveErr := saveToAtRevision(p, cfg, "")
			if saveErr != nil {
				cfg.LoadError = saveErr.Error()
				return cfg, saveErr
			}
			cfg.SourceRevision = revision
			return cfg, nil
		}
		loadErr := fmt.Errorf("read config: %w", err)
		cfg.LoadError = loadErr.Error()
		return cfg, loadErr
	}
	if err := toml.Unmarshal(data, &cfg); err != nil {
		loadErr := fmt.Errorf("parse config: %w", err)
		cfg = DefaultConfig()
		cfg.SourceRevision = configRevision(data)
		cfg.LoadError = loadErr.Error()
		return cfg, loadErr
	}
	normalized := NormalizeConfig(cfg)
	normalized.SourceRevision = configRevision(data)
	normalizedDisk, originalDisk := normalized, cfg
	normalizedDisk.AutomationIssues = nil
	normalizedDisk.SourceRevision = ""
	normalizedDisk.LoadError = ""
	originalDisk.AutomationIssues = nil
	originalDisk.SourceRevision = ""
	originalDisk.LoadError = ""
	corrected := !reflect.DeepEqual(normalizedDisk, originalDisk)
	// Invalid automatic-task rules are kept on disk byte-for-byte. The runtime
	// disables only those rules and the manager exposes their diagnostics; a
	// corrected representation is written only after an explicit user save.
	if len(normalized.AutomationIssues) > 0 {
		return normalized, nil
	}
	refreshAnnotations := needsAnnotatedTOMLRefresh(data)
	rewrite := refreshAnnotations || corrected
	if rewrite {
		if err := os.WriteFile(p+".bak", data, 0600); err != nil {
			loadErr := fmt.Errorf("back up config before refresh: %w", err)
			normalized.LoadError = loadErr.Error()
			return normalized, loadErr
		}
	}
	if rewrite {
		revision, err := saveToAtRevision(p, normalized, normalized.SourceRevision)
		if err != nil {
			loadErr := fmt.Errorf("refresh config annotations: %w", err)
			normalized.LoadError = loadErr.Error()
			return normalized, loadErr
		}
		normalized.SourceRevision = revision
	}
	return normalized, nil
}

func needsAnnotatedTOMLRefresh(data []byte) bool {
	text := string(data)
	if !strings.Contains(text, configTemplateVersionMarker()) {
		return true
	}
	for _, marker := range []string{
		"# -- 保持唤醒 / Stay Awake --",
		"# -- 自动任务 / Automatic Tasks --",
		"# -- 空闲监测 / Idle Monitoring --",
		"# -- 昼夜主题 / Day/Night Theme --",
		"# -- 设置 / Settings --",
	} {
		if !strings.Contains(text, marker) {
			return true
		}
	}
	return false
}

func configTemplateVersionMarker() string {
	return fmt.Sprintf("# config_template_version = %d", configTemplateVersion)
}
