package processcatalog

import (
	"errors"
	"reflect"
	"testing"
)

func TestIncludeSnapshotEntrySkipsSystemPseudoProcess(t *testing.T) {
	for _, test := range []struct {
		pid  uint32
		name string
		want bool
	}{
		{pid: 0, name: "[System Process]", want: false},
		{pid: 0, name: "System Idle Process", want: false},
		{pid: 4, name: "System", want: true},
		{pid: 42, name: "app.exe", want: true},
		{pid: 42, name: "", want: false},
	} {
		if got := includeSnapshotEntry(test.pid, test.name); got != test.want {
			t.Fatalf("includeSnapshotEntry(%d, %q) = %v, want %v", test.pid, test.name, got, test.want)
		}
	}
}

func TestGroupInstancesKeepsCountAndRepresentativeDescription(t *testing.T) {
	groups := GroupInstances([]Instance{
		{PID: 1, Executable: "app.exe", Path: `C:\A\app.exe`, Description: "App A"},
		{PID: 2, Executable: "APP.EXE", Path: `C:\A\app.exe`, Description: "App A"},
		{PID: 3, Executable: "app.exe", Path: `D:\B\app.exe`, Description: "App B"},
		{PID: 4, Executable: "app.exe"},
	})
	if len(groups) != 1 || groups[0].Count != 4 || groups[0].Description != "App A" {
		t.Fatalf("grouped instances = %+v", groups)
	}
}

func TestGroupInstancesDeduplicatesExecutableNames(t *testing.T) {
	groups := GroupInstances([]Instance{
		{PID: 1, Executable: "worker.exe"},
		{PID: 2, Executable: "WORKER.EXE"},
	})
	if len(groups) != 1 || groups[0].Count != 2 || groups[0].Executable != "worker.exe" {
		t.Fatalf("groups = %+v", groups)
	}
}

func TestEnrichDescriptionsTriesUntilDescriptionIsAvailable(t *testing.T) {
	var attempted []uint32
	var describedPaths []string
	instances := []Instance{
		{PID: 10, Executable: "app.exe"},
		{PID: 20, Executable: "APP.EXE"},
		{PID: 30, Executable: "app.exe"},
		{PID: 40, Executable: "other.exe", Description: "Already known"},
	}
	got := enrichDescriptions(
		instances,
		func(pid uint32) (string, error) {
			attempted = append(attempted, pid)
			if pid == 10 {
				return "", errors.New("access denied")
			}
			if pid == 20 {
				return `C:\Apps\undescribed.exe`, nil
			}
			return `C:\Apps\app.exe`, nil
		},
		func(path string) string {
			describedPaths = append(describedPaths, path)
			if path == `C:\Apps\undescribed.exe` {
				return ""
			}
			return "Accessible app"
		},
	)

	if !reflect.DeepEqual(attempted, []uint32{10, 20, 30}) {
		t.Fatalf("attempted PIDs = %v, want [10 20 30]", attempted)
	}
	if !reflect.DeepEqual(describedPaths, []string{`C:\Apps\undescribed.exe`, `C:\Apps\app.exe`}) {
		t.Fatalf("description paths = %v", describedPaths)
	}
	if got[0].Description != "" || got[1].Description != "" || got[2].Description != "Accessible app" {
		t.Fatalf("enriched instances = %+v", got)
	}
	if got[3] != instances[3] {
		t.Fatalf("pre-described instance changed: %+v", got[3])
	}
}
