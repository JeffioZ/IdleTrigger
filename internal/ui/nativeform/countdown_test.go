package nativeform

import "testing"

func TestCountdownCancellationStopsReplacedAndCurrentWorkers(t *testing.T) {
	var cancellation CountdownCancellation
	first := cancellation.Replace()
	second := cancellation.Replace()
	assertClosed(t, first, "replaced countdown")
	assertOpen(t, second, "current countdown")

	cancellation.Stop()
	assertClosed(t, second, "stopped countdown")
	cancellation.Stop()
}

func assertClosed(t *testing.T, signal <-chan struct{}, name string) {
	t.Helper()
	select {
	case <-signal:
	default:
		t.Fatalf("%s signal is still open", name)
	}
}

func assertOpen(t *testing.T, signal <-chan struct{}, name string) {
	t.Helper()
	select {
	case <-signal:
		t.Fatalf("%s signal is already closed", name)
	default:
	}
}
