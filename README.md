# scrutiny-embedded-rust

Port of scrutiny-embedded to Rust.

Work in progress

## Loops

Declare each loop with its own type, then register the shared core of every loop with the config.
Variable-frequency loops have no `process()`: the timestep must be given on every call.

```rust
static FAST: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Fast", 1000);
static SLOW: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Slow");
static LOOPS: [&LoopHandler; 2] = [FAST.handler(), SLOW.handler()];
// config.set_loops(&LOOPS);
// In the fast task:  FAST.process();
// In the slow task:  SLOW.process_with_time(dt);
```
