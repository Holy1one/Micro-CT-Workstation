# PC-direct device modules (offline candidate)

Each device module owns one physical role and exposes message proxies from mod.rs. Hardware acceptance remains BLOCKED; tests use scripted transports, never real devices.

## xray/

- moxtek.rs owns the raw binary protocol, requested setpoints, measured readback and verified OFF. A new emission's status read allows five seconds, bounded by its continuous-output deadline; monitoring/OFF keep short fail-closed timeouts.
- worker.rs is the single serial owner for manual and scan actions, cached health, priority OFF, caller/worker cancellation, independent monitoring and hard output deadlines. Confirmed OFF preserves the two-second re-enable gate across all callers.
- Initial discovery failure before any source session or port evidence is a rejected connection attempt; it does not trigger Nano warning or a global Stop. Formerly owned sources and any transport/protocol/OFF failure retain fail-closed cleanup. The domain observer logs changed cached health failures once instead of on every engine tick.

## camera/

- actor.rs owns creation/use/release of COM objects on its dedicated thread and exposes a Send message proxy. wpd.rs independently implements Microsoft MTP extensions; ptp.rs validates bounded datasets.
- nikon.rs contains necessary MIT D7100 operations and property readback, exact native timed shutters, Live View/AF and supported focus steps. Other models receive only described standard capabilities; host capture stays disabled without proof.
- capture.rs binds exclusive session/operation events, separates ExposureEnded from object/transfer/file completion, verifies complete NEF/length/hash, and commits atomically under cancellation. Elapsed time or file appearance never proves exposure end.
- live_view.rs holds validated latest-only JPEG envelopes and a random user-ACL pipe. Images never enter JSONL; Stop/session changes/loss revoke live identity. See camera/README.md and THIRD-PARTY-NOTICES.md for event and hardware limits.

## turntable/

- nano.rs owns identity, serial worker, heartbeat, HOME, advertised no-motion RESTORE_REFERENCE, warning, same-ID CAPTURE_DONE and priority STOP; cancelled epochs cannot admit late motion/rearm/reference restoration. Checkpoint and operator confirmation gates remain in device_controls.rs.
- planner.rs verifies fixed 8/96000 scaling and plans negative MOVE_REL from confirmed unwrapped pulses. Arrival and matching release must reach IDLE. Ordinary reverse/multiple turns are rejected; HOME retains the firmware's bounded retreat.
- firmware/rts9060_nano retains the recovered source provenance and pins the current 2.1.1 reference-recovery candidate for source rebuild and real-source host harness. VERSION/BUILD and offline tests do not certify installed flash or physical direction/accuracy.

Device modules do not call one another. scan.rs orders projection transactions; device_controls.rs owns independent HOME and all-device safety decisions. ct-engine remains the only domain authority.
