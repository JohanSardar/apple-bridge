# E2E first with an independent Truth oracle

Features start as failing end-to-end scenarios run by `bridge-lab` at sim, real, replica and agent levels, and are judged by Truth: SimTruth in sim, and AppleTruth, a separate Swift package sharing no code with the Helper, on the Air. Code-level tests exist only for decoders, time math and unreachable crash timing. A shared-code oracle was rejected because a Helper bug could hide itself.
