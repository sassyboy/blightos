//
// BlightOS Kernel
//
// Memory Management Module
// 

#[path = "phys.rs"]
pub mod phys;

#[path = "virt.rs"]
pub mod virt;

#[path = "heap.rs"]
pub mod heap;

///
/// Architecuture-agnostic memory types for proviing hints to the MMU driver
/// regarding the caching policy to use when mapping memory.
pub enum MemoryType {
    Normal,     // Cached for data/code (e.g., WriteBack)
    Device,     // Uncached for MMIO regions, slow but consistent
    OutputDMA,  // DMA buffers that are only written by the software and
                // read by the hardware, e.g., framebuffer and audio output.
                // Writes may be combined for better performance
}
