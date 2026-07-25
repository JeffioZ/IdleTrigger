package singleinstance

import (
	"fmt"
	"os"
	"testing"
	"time"
)

func TestAcquireNamedOwnsMutexUntilRelease(t *testing.T) {
	name := fmt.Sprintf("Local\\IdleTrigger-test-%d-%d", os.Getpid(), time.Now().UnixNano())

	first, primary, err := acquireNamed(name)
	if err != nil {
		t.Fatal(err)
	}
	if !primary || first == nil {
		t.Fatalf("first acquire = (%v, %v), want a primary guard", first, primary)
	}
	defer first.Release()

	second, primary, err := acquireNamed(name)
	if err != nil {
		t.Fatal(err)
	}
	if primary || second != nil {
		if second != nil {
			second.Release()
		}
		t.Fatalf("second acquire = (%v, %v), want an existing instance", second, primary)
	}

	first.Release()
	first.Release()

	reacquired, primary, err := acquireNamed(name)
	if err != nil {
		t.Fatal(err)
	}
	if !primary || reacquired == nil {
		t.Fatalf("acquire after release = (%v, %v), want a new primary guard", reacquired, primary)
	}
	reacquired.Release()
}

func TestAcquireNamedRejectsNUL(t *testing.T) {
	guard, primary, err := acquireNamed("Local\\IdleTrigger\x00invalid")
	if err == nil || guard != nil || primary {
		if guard != nil {
			guard.Release()
		}
		t.Fatalf("acquire invalid name = (%v, %v, %v), want an encoding error", guard, primary, err)
	}
}

func TestNilGuardReleaseIsSafe(t *testing.T) {
	var guard *Guard
	guard.Release()
}
