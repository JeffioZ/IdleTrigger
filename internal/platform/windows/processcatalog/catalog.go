// Package processcatalog reads the minimum metadata needed to present and
// match running applications. It never requests debug privileges, reads
// process memory, injects code, or scans executables that are not running.
package processcatalog

import (
	"fmt"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

const processQueryLimitedInformation = 0x1000

type Instance struct {
	PID         uint32
	Executable  string
	Path        string
	Description string
}

var (
	versionDLL        = windows.NewLazySystemDLL("version.dll")
	procVerQuery      = versionDLL.NewProc("VerQueryValueW")
	descriptionMu     sync.RWMutex
	descriptionByPath = make(map[string]string)
)

// SnapshotNames uses the Toolhelp process list. This operation does not open
// individual processes and is the only operation needed by name-only rules.
func SnapshotNames() ([]Instance, error) {
	snapshot, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return nil, err
	}
	defer windows.CloseHandle(snapshot)

	var pe windows.ProcessEntry32
	pe.Size = uint32(unsafe.Sizeof(pe))
	if err := windows.Process32First(snapshot, &pe); err != nil {
		return nil, err
	}
	instances := make([]Instance, 0, 128)
	for {
		name := strings.TrimSpace(windows.UTF16ToString(pe.ExeFile[:]))
		if includeSnapshotEntry(pe.ProcessID, name) {
			instances = append(instances, Instance{PID: pe.ProcessID, Executable: name})
		}
		err = windows.Process32Next(snapshot, &pe)
		if err == windows.ERROR_NO_MORE_FILES {
			break
		}
		if err != nil {
			return nil, err
		}
	}
	return instances, nil
}

// includeSnapshotEntry removes Toolhelp's PID-zero pseudo process. It is not a
// runnable executable and cannot be a useful automation target.
func includeSnapshotEntry(pid uint32, name string) bool {
	return pid != 0 && name != "" && !strings.EqualFold(name, "[System Process]")
}

// EnrichDescriptions keeps one representative result per executable name. It
// tries same-name instances only until one yields a FileDescription.
func EnrichDescriptions(instances []Instance) []Instance {
	return enrichDescriptions(instances, ImagePath, FileDescription)
}

func enrichDescriptions(
	instances []Instance,
	imagePath func(uint32) (string, error),
	fileDescription func(string) string,
) []Instance {
	out := append([]Instance(nil), instances...)
	candidatesByName := make(map[string][]int, len(out))
	described := make(map[string]struct{}, len(out))
	for index := range out {
		key := strings.ToLower(out[index].Executable)
		if out[index].Description != "" {
			described[key] = struct{}{}
		}
		candidatesByName[key] = append(candidatesByName[key], index)
	}
	for key := range described {
		delete(candidatesByName, key)
	}
	jobs := make(chan []int)
	var workers sync.WaitGroup
	workerCount := min(4, len(candidatesByName))
	for range workerCount {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for candidates := range jobs {
				firstAccessibleIndex := -1
				var firstAccessiblePath string
				for _, index := range candidates {
					path, err := imagePath(out[index].PID)
					if err != nil {
						continue
					}
					if firstAccessibleIndex < 0 {
						firstAccessibleIndex = index
						firstAccessiblePath = path
					}
					description := fileDescription(path)
					if description == "" {
						continue
					}
					out[index].Path = path
					out[index].Description = description
					firstAccessibleIndex = -1
					break
				}
				if firstAccessibleIndex >= 0 {
					out[firstAccessibleIndex].Path = firstAccessiblePath
				}
			}
		}()
	}
	for _, candidates := range candidatesByName {
		jobs <- candidates
	}
	close(jobs)
	workers.Wait()
	return out
}

// EnrichMatchingPaths limits handle opens to executable basenames used by an
// exact-path rule. Name-only rules should not call this function.
func EnrichMatchingPaths(instances []Instance, names map[string]struct{}) []Instance {
	out := append([]Instance(nil), instances...)
	for index := range out {
		if _, needed := names[strings.ToLower(out[index].Executable)]; !needed {
			continue
		}
		if path, err := ImagePath(out[index].PID); err == nil {
			out[index].Path = path
		}
	}
	return out
}

func ImagePath(pid uint32) (string, error) {
	handle, err := windows.OpenProcess(processQueryLimitedInformation, false, pid)
	if err != nil {
		return "", err
	}
	defer windows.CloseHandle(handle)
	buffer := make([]uint16, 32768)
	size := uint32(len(buffer))
	if err := windows.QueryFullProcessImageName(handle, 0, &buffer[0], &size); err != nil {
		return "", err
	}
	if size == 0 || size > uint32(len(buffer)) {
		return "", fmt.Errorf("invalid process image path length %d", size)
	}
	return filepath.Clean(windows.UTF16ToString(buffer[:size])), nil
}

func FileDescription(path string) string {
	key := strings.ToLower(filepath.Clean(path))
	descriptionMu.RLock()
	value, found := descriptionByPath[key]
	descriptionMu.RUnlock()
	if found {
		return value
	}
	value = readFileDescription(path)
	descriptionMu.Lock()
	descriptionByPath[key] = value
	descriptionMu.Unlock()
	return value
}

func readFileDescription(path string) string {
	var zero windows.Handle
	size, err := windows.GetFileVersionInfoSize(path, &zero)
	if err != nil || size == 0 {
		return ""
	}
	data := make([]byte, size)
	if err := windows.GetFileVersionInfo(path, 0, size, unsafe.Pointer(&data[0])); err != nil {
		return ""
	}
	languages := versionTranslations(data)
	// English Unicode is the conventional fallback when Translation is absent.
	languages = append(languages, [2]uint16{0x0409, 0x04b0})
	for _, lang := range languages {
		query := fmt.Sprintf("\\StringFileInfo\\%04x%04x\\FileDescription", lang[0], lang[1])
		if value := versionString(data, query); value != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func versionTranslations(data []byte) [][2]uint16 {
	query, _ := windows.UTF16PtrFromString("\\VarFileInfo\\Translation")
	var pointer unsafe.Pointer
	var length uint32
	result, _, _ := procVerQuery.Call(uintptr(unsafe.Pointer(&data[0])), uintptr(unsafe.Pointer(query)), uintptr(unsafe.Pointer(&pointer)), uintptr(unsafe.Pointer(&length)))
	if result == 0 || pointer == nil || length < 4 {
		return nil
	}
	words := unsafe.Slice((*uint16)(pointer), int(length/2))
	out := make([][2]uint16, 0, len(words)/2)
	for index := 0; index+1 < len(words); index += 2 {
		out = append(out, [2]uint16{words[index], words[index+1]})
	}
	return out
}

func versionString(data []byte, path string) string {
	query, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return ""
	}
	var pointer unsafe.Pointer
	var length uint32
	result, _, _ := procVerQuery.Call(uintptr(unsafe.Pointer(&data[0])), uintptr(unsafe.Pointer(query)), uintptr(unsafe.Pointer(&pointer)), uintptr(unsafe.Pointer(&length)))
	if result == 0 || pointer == nil || length == 0 {
		return ""
	}
	return windows.UTF16PtrToString((*uint16)(pointer))
}

type Group struct {
	Executable  string
	Description string
	Count       int
}

// GroupInstances creates the stable, name-deduplicated rows shown by the picker.
func GroupInstances(instances []Instance) []Group {
	groups := make(map[string]*Group)
	for _, instance := range instances {
		nameKey := strings.ToLower(instance.Executable)
		entry := groups[nameKey]
		if entry == nil {
			entry = &Group{Executable: instance.Executable}
			groups[nameKey] = entry
		}
		entry.Count++
		if entry.Description == "" && instance.Description != "" {
			entry.Description = instance.Description
		}
	}
	out := make([]Group, 0, len(groups))
	for _, entry := range groups {
		out = append(out, *entry)
	}
	sort.Slice(out, func(i, j int) bool {
		return strings.ToLower(out[i].Executable) < strings.ToLower(out[j].Executable)
	})
	return out
}
