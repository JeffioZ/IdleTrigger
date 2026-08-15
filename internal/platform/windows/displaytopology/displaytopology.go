// Package displaytopology exposes a small, stable fingerprint of the active
// Windows display paths. It intentionally excludes refresh-rate and timing
// details because VRR and dynamic refresh can change those values without a
// monitor being connected, disconnected, moved, or resized.
package displaytopology

import (
	"encoding/binary"
	"fmt"
	"sort"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	qdcOnlyActivePaths                  = 0x00000002
	displayConfigModeInfoSource         = 1
	displayConfigPathModeInvalid        = 0xffffffff
	errorInsufficientBuffer      uint32 = 122
	maxQueryAttempts                    = 5
)

var (
	displayUser32                = windows.NewLazySystemDLL("user32.dll")
	pGetDisplayConfigBufferSizes = displayUser32.NewProc("GetDisplayConfigBufferSizes")
	pQueryDisplayConfig          = displayUser32.NewProc("QueryDisplayConfig")
)

type adapterID struct {
	LowPart  uint32
	HighPart int32
}

type rational struct {
	Numerator   uint32
	Denominator uint32
}

type pathSourceInfo struct {
	AdapterID   adapterID
	ID          uint32
	ModeInfoIdx uint32
	StatusFlags uint32
}

type pathTargetInfo struct {
	AdapterID        adapterID
	ID               uint32
	ModeInfoIdx      uint32
	OutputTechnology uint32
	Rotation         uint32
	Scaling          uint32
	RefreshRate      rational
	ScanLineOrdering uint32
	TargetAvailable  int32
	StatusFlags      uint32
}

type pathInfo struct {
	SourceInfo pathSourceInfo
	TargetInfo pathTargetInfo
	Flags      uint32
}

// DISPLAYCONFIG_MODE_INFO contains a 48-byte union. Only the source-mode
// width, height, pixel format, and position are needed for the fingerprint.
type modeInfo struct {
	InfoType  uint32
	ID        uint32
	AdapterID adapterID
	Data      [48]byte
}

// Path is the subset of one active display path that should remain unchanged
// while Windows rebuilds its desktop topology.
type Path struct {
	SourceAdapter int64
	SourceID      uint32
	TargetAdapter int64
	TargetID      uint32
	X             int32
	Y             int32
	Width         uint32
	Height        uint32
	Rotation      uint32
}

// Snapshot is sorted deterministically so enumeration-order changes do not
// look like physical display changes.
type Snapshot struct {
	Paths []Path
}

// ActiveDisplayCount returns the number of active target paths.
func (s Snapshot) ActiveDisplayCount() int { return len(s.Paths) }

// Equal reports whether two snapshots describe the same desktop topology.
func (s Snapshot) Equal(other Snapshot) bool {
	if len(s.Paths) != len(other.Paths) {
		return false
	}
	for i := range s.Paths {
		if s.Paths[i] != other.Paths[i] {
			return false
		}
	}
	return true
}

// Query returns the active display topology. Windows can change the required
// buffer sizes between the sizing and query calls, so an insufficient-buffer
// result is retried with fresh counts.
func Query() (Snapshot, error) {
	for attempt := 0; attempt < maxQueryAttempts; attempt++ {
		var pathCount, modeCount uint32
		result, _, callErr := pGetDisplayConfigBufferSizes.Call(
			qdcOnlyActivePaths,
			uintptr(unsafe.Pointer(&pathCount)),
			uintptr(unsafe.Pointer(&modeCount)),
		)
		if uint32(result) != 0 {
			return Snapshot{}, displayConfigError("size active display topology", uint32(result), callErr)
		}
		if pathCount == 0 {
			return Snapshot{}, nil
		}

		paths := make([]pathInfo, pathCount)
		modes := make([]modeInfo, modeCount)
		var modePointer uintptr
		if len(modes) != 0 {
			modePointer = uintptr(unsafe.Pointer(&modes[0]))
		}
		result, _, callErr = pQueryDisplayConfig.Call(
			qdcOnlyActivePaths,
			uintptr(unsafe.Pointer(&pathCount)),
			uintptr(unsafe.Pointer(&paths[0])),
			uintptr(unsafe.Pointer(&modeCount)),
			modePointer,
			0,
		)
		code := uint32(result)
		if code == errorInsufficientBuffer {
			continue
		}
		if code != 0 {
			return Snapshot{}, displayConfigError("query active display topology", code, callErr)
		}
		paths = paths[:pathCount]
		modes = modes[:modeCount]
		return buildSnapshot(paths, modes), nil
	}
	return Snapshot{}, fmt.Errorf("query active display topology: display configuration kept changing")
}

func buildSnapshot(paths []pathInfo, modes []modeInfo) Snapshot {
	snapshot := Snapshot{Paths: make([]Path, 0, len(paths))}
	for _, path := range paths {
		fingerprint := Path{
			SourceAdapter: adapterValue(path.SourceInfo.AdapterID),
			SourceID:      path.SourceInfo.ID,
			TargetAdapter: adapterValue(path.TargetInfo.AdapterID),
			TargetID:      path.TargetInfo.ID,
			Rotation:      path.TargetInfo.Rotation,
		}
		if source, ok := sourceModeForPath(path.SourceInfo, modes); ok {
			fingerprint.Width = binary.LittleEndian.Uint32(source.Data[0:4])
			fingerprint.Height = binary.LittleEndian.Uint32(source.Data[4:8])
			fingerprint.X = int32(binary.LittleEndian.Uint32(source.Data[12:16]))
			fingerprint.Y = int32(binary.LittleEndian.Uint32(source.Data[16:20]))
		}
		snapshot.Paths = append(snapshot.Paths, fingerprint)
	}
	sort.Slice(snapshot.Paths, func(i, j int) bool {
		a, b := snapshot.Paths[i], snapshot.Paths[j]
		if a.TargetAdapter != b.TargetAdapter {
			return a.TargetAdapter < b.TargetAdapter
		}
		if a.TargetID != b.TargetID {
			return a.TargetID < b.TargetID
		}
		if a.SourceAdapter != b.SourceAdapter {
			return a.SourceAdapter < b.SourceAdapter
		}
		return a.SourceID < b.SourceID
	})
	return snapshot
}

func sourceModeForPath(source pathSourceInfo, modes []modeInfo) (modeInfo, bool) {
	if source.ModeInfoIdx != displayConfigPathModeInvalid && int(source.ModeInfoIdx) < len(modes) {
		mode := modes[source.ModeInfoIdx]
		if mode.InfoType == displayConfigModeInfoSource {
			return mode, true
		}
	}
	for _, mode := range modes {
		if mode.InfoType == displayConfigModeInfoSource && mode.ID == source.ID && mode.AdapterID == source.AdapterID {
			return mode, true
		}
	}
	return modeInfo{}, false
}

func adapterValue(id adapterID) int64 {
	return int64(uint64(uint32(id.HighPart))<<32 | uint64(id.LowPart))
}

func displayConfigError(operation string, code uint32, callErr error) error {
	if code != 0 {
		return fmt.Errorf("%s: Win32 error %d", operation, code)
	}
	return fmt.Errorf("%s: %v", operation, callErr)
}
