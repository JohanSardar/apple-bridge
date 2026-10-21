package world

type CaptureProbe struct {
	AtomicOffers            int
	LegacyOffers            int
	AtomicWelcomes          int
	PartialFields           int
	PartialAckWithoutCursor int
	PartialAckWithCursor    int
	PartialFrames           int
	PartialBytes            int
	ResetFrames             int
	LegacyWelcomes          int
	HelloSequences          []uint64
}

func (w *World) ProbeCaptures(reset bool) error {
	if err := w.configureFrameFaults(); err != nil {
		return err
	}
	w.proxy.mu.Lock()
	w.proxy.frames.captureReset = reset
	w.proxy.mu.Unlock()
	w.proxy.Partition()
	w.proxy.Heal()
	return nil
}
func (w *World) CaptureProbe() CaptureProbe {
	w.proxy.mu.Lock()
	defer w.proxy.mu.Unlock()
	f := w.proxy.frames
	return CaptureProbe{AtomicOffers: f.atomicOffers, LegacyOffers: f.legacyOffers, AtomicWelcomes: f.atomicWelcomes, PartialFields: f.partialFields, PartialAckWithoutCursor: f.partialAckWithoutCursor, PartialAckWithCursor: f.partialAckWithCursor, PartialFrames: f.partialFrames, PartialBytes: f.partialBytes, ResetFrames: f.resetFrames, LegacyWelcomes: f.legacyWelcomes, HelloSequences: append([]uint64(nil), f.helloSequences...)}
}
