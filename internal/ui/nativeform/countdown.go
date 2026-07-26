package nativeform

// CountdownCancellation owns the stop signal for one UI countdown. Replace
// and Stop must be called by the countdown's owning UI thread; workers only
// receive and wait on the returned channel.
type CountdownCancellation struct {
	stop chan struct{}
}

// Replace stops the previous countdown and returns a fresh stop signal.
func (c *CountdownCancellation) Replace() <-chan struct{} {
	c.Stop()
	c.stop = make(chan struct{})
	return c.stop
}

// Stop cancels the current countdown. It is safe to call repeatedly.
func (c *CountdownCancellation) Stop() {
	if c.stop == nil {
		return
	}
	close(c.stop)
	c.stop = nil
}
