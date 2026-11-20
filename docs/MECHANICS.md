# Mechanics reference

This sheet records mechanics recovered from the original Windows executable. The rebuilt app uses its own portable JSON recording format and does not claim compatibility with the original `.rec` format.

| Behavior | Recovered value or rule | Original source |
|---|---|---|
| Default playback speed | Setting value `0` is the normal `1x` menu choice; `2x` uses value `2`; `100x` uses value `100`. | `0x401000`, `0x401489` |
| Custom playback speed | Default multiplier `8`; input is clamped to `1..=100`. | `0x401000`, `0x401489` |
| Playback delay below `100x` | Let `d` be the next event timestamp minus the current event timestamp. Value `0` waits `d`; value `1` waits `2*d`; values above `1` wait integer `d/speed`. The first event is dispatched before a following-event delay is scheduled. | `0x4034c6` |
| `100x` playback path | Uses a `1 ms` timer, or `75 ms` after emitting mouse movement. When the left button is not held, a run of mouse moves is coalesced by emitting each sixth event and the last event; each emitted move is followed by a `1 ms` sleep. | `0x4034c6` |
| End of playback after a left click | Waits half the current Windows double-click interval (`GetDoubleClickTime()/2`) before completing the pass. | `0x4034c6` |
| Playback stop keys | Pause and Scroll Lock stop playback. | `0x4034c6`; string at `0x406341` |
| Recording timer interval | `10 ms`. The sampler scans virtual keys `1..=254`, records the first state transition it finds per tick, and ignores the left-shift virtual key (`0xA0`). | `0x4032fd` |
| Recorded actions | Key down/up, mouse move, and left/right mouse down/up. Unsupported keys and middle/side/wheel buttons are excluded. Mouse movement is ignored at coordinate zero and when the low-byte sum of x and y has not changed. Escape remains recordable; Pause and Scroll Lock stop recording or playback instead of being recorded. | `0x4032fd` |
| Event record size | `20 bytes` (`0x14`). | `0x4032fd`, `0x402f8e` |
| Default repeat count | `1`; the loop-count setting is stored at `0x40600c` and values below `1` are normalized to `1`. | `0x401489`; data at `0x40600c` |
| Continuous playback default | `Off`; the command handler toggles a zero-initialized state. | `0x401489`; zero-initialized data at `0x406d1a` |
| Original recording loader | Allocates file size plus `4096 bytes` of memory, reads from a caller-selected file offset, and validates at least two bytes, a zero first byte, and a length divisible by `20`. It derives event count from the allocated block minus the `4096-byte` padding; this does not prove a `4096-byte` on-disk header. The complete save layout remains unrecovered. | `0x402f8e`, `0x403d51`, `0x403ea0` |

## Rebuild behavior

The app starts at the original normal-speed setting (`0`), uses the original default custom multiplier (`8`), repeat count (`1`), continuous mode (`off`), speed formulas, fast-path cadence, and left-click completion pause. It collects global key edges through an event listener rather than reproducing the original one-key-transition-per-10-ms polling loop; pointer events are rate-limited to `10 ms` and use the recovered coordinate filter. The app records elapsed monotonic time and saves a separate JSON format, not the original binary `.rec` files.

The original data provides the values above, but the rebuild checker does not measure runtime behavior against this sheet; that comparison remains unverified.

## Changes in this version

Recording is now minimal: only key down/up, left/right mouse button down/up (stamped with the pointer position at that moment) and one final `move` event holding the last pointer position outside the app window. Continuous mouse movement is no longer stored, and clicks on the recorder's own buttons are ignored. During playback the pointer jumps to each click position before pressing/releasing, and to the final position at the end. The 100x mouse-move coalescing path was removed because recordings no longer contain move runs. Older macro files still play back.
