//
// BlightOS Kernel
//
// Physical Memory Manager
//   Marks physical memory frames as free or allocated
//
// 

use crate::util::*;

#[derive(Copy, Clone)]
pub struct PMMapElement {
    pub base: usize,
    pub len:  usize,
    pub avail:bool,
}

pub fn pmm_init() {
    klog!("Physical Memory Manager Initialized\n");
}