/// Loop type: fixed frequency or variable frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoopType {
    /// Loop with a constant time step between calls.
    FixedFreq = 0,
    /// Loop where the time step varies between calls.
    VariableFreq = 1,
}

/// Describes a time domain in which scrutiny-related probing can occur.
/// Registered with `Config` so the server can discover loop properties via `GetInfo`.
pub struct LoopHandler {
    name: &'static str,
    loop_type: LoopType,
    timestep_100ns: u32,
}

impl LoopHandler {
    /// Creates a fixed-frequency loop with the given name and timestep in 100 ns units.
    #[must_use]
    pub const fn fixed_freq(name: &'static str, timestep_100ns: u32) -> Self {
        Self { name, loop_type: LoopType::FixedFreq, timestep_100ns }
    }

    /// Creates a variable-frequency loop with the given name.
    #[must_use]
    pub const fn variable_freq(name: &'static str) -> Self {
        Self { name, loop_type: LoopType::VariableFreq, timestep_100ns: 0 }
    }

    /// Returns the loop name.
    #[must_use]
    pub const fn get_name(&self) -> &'static str {
        self.name
    }

    /// Returns the loop type.
    #[must_use]
    pub const fn loop_type(&self) -> LoopType {
        self.loop_type
    }

    /// Returns the fixed timestep in 100 ns units; 0 for variable-frequency loops.
    #[must_use]
    pub const fn get_timestep_100ns(&self) -> u32 {
        self.timestep_100ns
    }
}
