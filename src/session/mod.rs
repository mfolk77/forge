#[allow(dead_code)]
pub mod budget;
#[allow(dead_code)]
pub mod manager;
#[allow(dead_code)]
pub mod memory;

#[allow(unused_imports)]
pub use memory::{MemoryEntry, MemoryManager, MemorySource};

#[cfg(test)]
mod security_tests;
