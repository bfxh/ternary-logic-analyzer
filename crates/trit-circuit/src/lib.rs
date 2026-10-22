//! 三值组合网表的 X 传播与 stuck-at 可测性分析。

pub mod atpg;
pub mod circuit;
pub mod map;
pub mod netlist;
pub mod timing;
pub mod verilog;

pub use atpg::{AtpgReport, CoverageStat, TestPattern, generate_tests};
pub use circuit::{
    Circuit, CircuitError, CircuitReport, Fault, Gate, GateKind, Propagation, Signal,
    SimulationRow, UntestableReason, analyze, analyze_faults, propagate,
};
pub use map::{MapError, to_nand_only};
pub use netlist::{NetlistError, from_json, parse_netlist, to_json, to_netlist};
pub use timing::{
    OutputTiming, TimingModel, TimingPath, TimingReport, analyze_timing, critical_paths,
};
pub use verilog::{VerilogError, parse_verilog, to_verilog};
