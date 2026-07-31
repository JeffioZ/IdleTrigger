package theme

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"golang.org/x/sys/windows"
)

func TestPatchThemeFileForcesColorizationAndCurrentModes(t *testing.T) {
	source := []byte("[Theme]\r\nDisplayName=Original\r\nThemeId={OLD}\r\n\r\n[VisualStyles]\r\nAutoColorization=1\r\nColorizationColor=0X112233\r\nAppMode=Light\r\nSystemMode=Light\r\n\r\n[Sounds]\r\nSchemeName=Default\r\n")
	got, err := patchThemeFile(source, themeFilePatch{
		displayName: "IdleTrigger DWM Refresh", themeID: "{NEW}",
		appMode: "Dark", systemMode: "Dark", colorization: 0xff123456,
		setColorization: true, disableAutoColorization: true,
	})
	if err != nil {
		t.Fatalf("patchThemeFile: %v", err)
	}
	text := string(got)
	for _, want := range []string{
		"DisplayName=IdleTrigger DWM Refresh", "ThemeId={NEW}",
		"AutoColorization=0", "ColorizationColor=0XFF123456",
		"AppMode=Dark", "SystemMode=Dark",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("patched theme missing %q:\n%s", want, text)
		}
	}
	if !strings.Contains(text, "[Sounds]\r\nSchemeName=Default") {
		t.Fatalf("unrelated theme section changed:\n%s", text)
	}
}

func TestPatchThemeFilePreservesAutoColorizationWhenNotDisabled(t *testing.T) {
	source := []byte("[Theme]\nDisplayName=Original\nThemeId={OLD}\n[VisualStyles]\nAutoColorization=1\nColorizationColor=0XABCDEF\n")
	got, err := patchThemeFile(source, themeFilePatch{
		displayName: "Patched", themeID: "{NEW}",
		appMode: "Dark", systemMode: "Light", colorization: 0xff123456, setColorization: true,
	})
	if err != nil {
		t.Fatalf("patchThemeFile: %v", err)
	}
	text := string(got)
	for _, want := range []string{"AutoColorization=1", "ColorizationColor=0XFF123456", "AppMode=Dark", "SystemMode=Light"} {
		if !strings.Contains(text, want) {
			t.Errorf("patched theme missing %q:\n%s", want, text)
		}
	}
}

func TestPatchThemeFileRejectsMissingRequiredSection(t *testing.T) {
	_, err := patchThemeFile([]byte("[Theme]\nDisplayName=Original\n"), themeFilePatch{
		displayName: "Refresh", themeID: "{NEW}", appMode: "Dark", systemMode: "Dark",
	})
	if err == nil || !strings.Contains(err.Error(), "VisualStyles") {
		t.Fatalf("patchThemeFile error = %v, want missing VisualStyles", err)
	}
}

func TestReadThemeSnapshotSkipsIncompleteCurrentTheme(t *testing.T) {
	dir := t.TempDir()
	incomplete := filepath.Join(dir, "Custom.theme")
	complete := filepath.Join(dir, "Current.theme")
	if err := os.WriteFile(incomplete, []byte("[Theme]\nDisplayName=Incomplete\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	want := []byte("[Theme]\nDisplayName=Current\n[VisualStyles]\nAppMode=Dark\nSystemMode=Light\n")
	if err := os.WriteFile(complete, want, 0o600); err != nil {
		t.Fatal(err)
	}

	got, err := readThemeSnapshotFromPaths([]string{incomplete, complete, complete})
	if err != nil {
		t.Fatalf("readThemeSnapshotFromPaths: %v", err)
	}
	if string(got) != string(want) {
		t.Fatalf("theme snapshot = %q, want complete current theme", got)
	}
}

func TestReadThemeSnapshotSkipsOversizedTheme(t *testing.T) {
	dir := t.TempDir()
	oversized := filepath.Join(dir, "Oversized.theme")
	complete := filepath.Join(dir, "Current.theme")
	if err := os.WriteFile(oversized, bytes.Repeat([]byte("x"), maxThemeSnapshotSize+1), 0o600); err != nil {
		t.Fatal(err)
	}
	want := []byte("[Theme]\nDisplayName=Current\n[VisualStyles]\nAppMode=Dark\nSystemMode=Light\n")
	if err := os.WriteFile(complete, want, 0o600); err != nil {
		t.Fatal(err)
	}

	got, err := readThemeSnapshotFromPaths([]string{oversized, complete})
	if err != nil {
		t.Fatalf("readThemeSnapshotFromPaths: %v", err)
	}
	if string(got) != string(want) {
		t.Fatalf("theme snapshot = %q, want complete fallback theme", got)
	}
}

func TestReadThemeSnapshotRejectsOversizedTheme(t *testing.T) {
	path := filepath.Join(t.TempDir(), "Oversized.theme")
	if err := os.WriteFile(path, bytes.Repeat([]byte("x"), maxThemeSnapshotSize+1), 0o600); err != nil {
		t.Fatal(err)
	}

	_, err := readThemeSnapshotFromPaths([]string{path})
	if err == nil || !strings.Contains(err.Error(), "1048576-byte limit") {
		t.Fatalf("readThemeSnapshotFromPaths error = %v, want size limit", err)
	}
}

func TestThemeModeFromThemeFile(t *testing.T) {
	source := []byte("[Theme]\nDisplayName=Current\n[VisualStyles]\nAppMode=Dark\nSystemMode=Light\n")
	if light, ok := themeModeFromThemeFile(source, "AppMode"); !ok || light {
		t.Fatalf("AppMode = light:%v found:%v, want dark", light, ok)
	}
	if light, ok := themeModeFromThemeFile(source, "SystemMode"); !ok || !light {
		t.Fatalf("SystemMode = light:%v found:%v, want light", light, ok)
	}
	if _, ok := themeModeFromThemeFile(source, "MissingMode"); ok {
		t.Fatal("missing theme mode was reported as present")
	}
}

func TestNudgedColorizationChangesOnlyLowNibble(t *testing.T) {
	for _, test := range []struct {
		input uint32
		want  uint32
	}{
		{0xff123456, 0xff123457},
		{0xff123459, 0xff123458},
		{0xff12345f, 0xff12345e},
	} {
		if got := nudgedColorization(test.input); got != test.want {
			t.Errorf("nudgedColorization(%08X) = %08X, want %08X", test.input, got, test.want)
		}
	}
}

func TestThemeIDStringHasSingleBracePair(t *testing.T) {
	id := windows.GUID{Data1: 0x12345678, Data2: 0x1234, Data3: 0xabcd, Data4: [8]byte{0x80, 0, 1, 2, 3, 4, 5, 6}}
	got := themeIDString(id)
	if strings.Count(got, "{") != 1 || strings.Count(got, "}") != 1 || got != strings.ToUpper(got) {
		t.Fatalf("themeIDString = %q, want one uppercase brace pair", got)
	}
}
