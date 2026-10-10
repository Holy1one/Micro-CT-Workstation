# PC-direct device modules (offline candidate)

Each device module owns one physical role and exposes message proxies from mod.rs. Hardware acceptance remains BLOCKED; tests use scripted transports, never real devices.

## xray/

- moxtek.rs owns the raw binary protocol, requested setpoints, measured readback and verified OFF. A complete monitoring pair (0x43 + 0x80) shares at most five seconds, bounded by the absolute output deadline. OFF keeps one second per transaction. No command is replayed; invalid packets and exhausted budgets fail closed.
- transport.rs accumulates fragmented replies across 50 ms cancellable read slices. Timeouts distinguish received bytes and elapsed/budget milliseconds; real driver errors fail immediately. A short OS read timeout alone does not discard a pending reply.
- worker.rs is the single serial owner for manual and scan actions, cached health, priority OFF, caller/worker cancellation, independent monitoring and hard output deadlines. Scan and background emission checks share only a fully verified pair less than 250 ms old in the current generation; mutations, OFF and faults invalidate it. Priority OFF cancels both queued operations and autonomous polling. A bounded diagnostic queue feeds domain logs and scan.log. Confirmed OFF preserves the two-second re-enable gate across all callers.
- Initial discovery failure before any source session or port evidence is a rejected connection attempt; it does not trigger Nano warning or a global Stop. Formerly owned sources and any transport/protocol/OFF failure retain fail-closed cleanup. The domain observer logs changed cached health failures once instead of on every engine tick.

## camera/

- actor.rs owns creation/use/release of COM objects on its dedicated thread and exposes a Send message proxy. wpd.rs independently implements Microsoft MTP extensions; ptp.rs validates bounded datasets.
- nikon.rs contains necessary MIT D7100 operations and property readback, exact native timed shutters, Live View/AF and supported focus steps. Other models receive only described standard capabilities; host capture stays disabled without proof.
- capture.rs binds exclusive session/operation events, separates ExposureEnded from object/transfer/file completion, verifies complete NEF/length/hash, and commits atomically under cancellation. Elapsed time or file appearance never proves exposure end.
- live_view.rs holds validated latest-only JPEG envelopes and a random user-ACL pipe. Images never enter JSONL; Stop/session changes/loss revoke live identity. See camera/README.md and THIRD-PARTY-NOTICES.md for event and hardware limits.
- storage.rs parses strict GetStorageIDs/GetStorageInfo/GetObjectHandles datasets and plans writable SD-card capacity offline; ptp.rs validates card-resident ObjectInfo NEF metadata with a separate strict parser that does not relax the RAM tolerance. Neither path contacts a device, and no card conclusion is drawn without hardware acceptance.
- card_capture.rs triggers one standard InitiateCapture (0x100E, never the private SDRAM code) and associates exactly one ObjectAdded object and one CaptureCompleteCard completion with it under a frozen card profile. The only object read is bounded GetObjectInfo metadata: no image, partial-object or thumbnail read, no local file and no hash. StoreFull overflow needs a planned second storage plus the advertised events; duplicates, unplanned storages, StoreRemoved, RAM evidence, changed sessions and trailing events refuse the receipt and leave the capture fail-closed.

## turntable/

- nano.rs owns identity, serial worker, heartbeat, HOME, advertised no-motion RESTORE_REFERENCE, warning, same-ID CAPTURE_DONE and priority STOP; cancelled epochs cannot admit late motion/rearm/reference restoration. Checkpoint and operator confirmation gates remain in device_controls.rs.
- planner.rs verifies fixed 8/96000 scaling and plans negative MOVE_REL from confirmed unwrapped pulses. Arrival and matching release must reach IDLE. Ordinary reverse/multiple turns are rejected; HOME retains the firmware's bounded retreat.
- firmware/rts9060_nano retains the recovered source provenance and pins the current 2.1.1 reference-recovery candidate for source rebuild and real-source host harness. VERSION/BUILD and offline tests do not certify installed flash or physical direction/accuracy.

Device modules do not call one another. scan.rs orders projection transactions; device_controls.rs owns independent HOME and all-device safety decisions. ct-engine remains the only domain authority.
