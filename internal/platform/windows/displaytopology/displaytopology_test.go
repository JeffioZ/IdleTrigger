package displaytopology

import (
	"encoding/binary"
	"testing"
	"unsafe"
)

func TestDisplayConfigLayoutsMatchWindowsABI(t *testing.T) {
	if got := unsafe.Sizeof(pathInfo{}); got != 72 {
		t.Fatalf("DISPLAYCONFIG_PATH_INFO size = %d, want 72", got)
	}
	if got := unsafe.Sizeof(modeInfo{}); got != 64 {
		t.Fatalf("DISPLAYCONFIG_MODE_INFO size = %d, want 64", got)
	}
}

func TestBuildSnapshotSortsPathsAndIgnoresRefreshRate(t *testing.T) {
	adapter := adapterID{LowPart: 9, HighPart: -2}
	modes := []modeInfo{
		sourceMode(adapter, 2, 1920, 1080, 1920, 0),
		sourceMode(adapter, 1, 1920, 1080, 0, 0),
	}
	paths := []pathInfo{
		{SourceInfo: pathSourceInfo{AdapterID: adapter, ID: 2, ModeInfoIdx: 0}, TargetInfo: pathTargetInfo{AdapterID: adapter, ID: 20, Rotation: 1, RefreshRate: rational{Numerator: 144000, Denominator: 1000}}},
		{SourceInfo: pathSourceInfo{AdapterID: adapter, ID: 1, ModeInfoIdx: 1}, TargetInfo: pathTargetInfo{AdapterID: adapter, ID: 10, Rotation: 1, RefreshRate: rational{Numerator: 60000, Denominator: 1000}}},
	}

	got := buildSnapshot(paths, modes)
	if len(got.Paths) != 2 || got.Paths[0].TargetID != 10 || got.Paths[1].TargetID != 20 {
		t.Fatalf("snapshot paths are not deterministic: %+v", got.Paths)
	}
	if got.Paths[1].X != 1920 || got.Paths[1].Width != 1920 || got.Paths[1].Height != 1080 {
		t.Fatalf("source geometry was not decoded: %+v", got.Paths[1])
	}

	paths[0].TargetInfo.RefreshRate = rational{Numerator: 48000, Denominator: 1000}
	if other := buildSnapshot(paths, modes); !got.Equal(other) {
		t.Fatalf("refresh-rate-only change altered topology: before=%+v after=%+v", got, other)
	}
}

func TestSourceModeFallsBackToIdentityLookup(t *testing.T) {
	adapter := adapterID{LowPart: 7}
	want := sourceMode(adapter, 3, 2560, 1440, -2560, 0)
	got, ok := sourceModeForPath(pathSourceInfo{AdapterID: adapter, ID: 3, ModeInfoIdx: displayConfigPathModeInvalid}, []modeInfo{want})
	if !ok || got != want {
		t.Fatalf("sourceModeForPath = (%+v, %v), want identity match", got, ok)
	}
}

func sourceMode(adapter adapterID, id, width, height uint32, x, y int32) modeInfo {
	mode := modeInfo{InfoType: displayConfigModeInfoSource, ID: id, AdapterID: adapter}
	binary.LittleEndian.PutUint32(mode.Data[0:4], width)
	binary.LittleEndian.PutUint32(mode.Data[4:8], height)
	binary.LittleEndian.PutUint32(mode.Data[12:16], uint32(x))
	binary.LittleEndian.PutUint32(mode.Data[16:20], uint32(y))
	return mode
}
