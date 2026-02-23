//
// BlightOS Kernel
//
// Flattened Device Tree
// 
// Reference:
// https://www.kernel.org/doc/Documentation/devicetree/booting-without-of.txt
//
// NB: Values are stored in big endian
//

use crate::mem::phys::PMMapElement;
use crate::util::*;

#[repr(C, packed)]
struct FdtHeader {
    magic:              u32,    /* magic word OF_DT_HEADER (0xd00dfeed) */
    total_size:         u32,    /* total size of DT block */
    struct_off:         u32,    /* offset to structure */
    strings_off:        u32,    /* offset to strings */
    mem_rsvmap_off:     u32,    /* offset to memory reserve map */
    version:            u32,    /* format version */
    last_comp_version:  u32,    /* last compatible version */
    /* version 2 fields below */
    boot_cpuid_phys:    u32,    /* Which physical CPU id we're booting on */
}

#[repr(C, packed)]
struct FdtRsvMapItem {
    base_addr:          u64,
    length:             u64,
}

// MMIO devices on SoCs present bus addresses, which need to be translated
// into physical memory addresses. The soc object in the device tree presents
// the address-translation rules. Every device under the SoC presents Bus
// addresses
#[derive(Copy, Clone, Debug)]
pub struct BusToPhysMemMapping {
    bus_addr:           usize,
    mem_addr:           usize,
    length:             usize
}
impl BusToPhysMemMapping {
    pub const fn new() -> Self {
        Self {
            bus_addr:   0,
            mem_addr:   0,
            length:     0
        }
    }
}
#[derive(Copy, Clone, Debug)]
pub struct FdtCpuInfo {
    // Properties
    // Name,               Value
    // device_type         "cpu"
    // clock-frequency     a u32 or u64: clock speed of the CPU in HZ
    // timebase-frequency  a u32 or u64: the freq. (in Hz) at which timebase and
    //                                    decrementer registers are updated.
    // status              "okay" or "disabled" or         "failed"
    //                     ^ running  ^ see enable-method  ^ ignore the CPU
    // enable-method       "spin-table", "[vendor],[method]"
    // cpu-release-addr    u64: Address of the CPU's entry in the spin-table
    pub release_addr:       u64,
    pub compatible:         &'static str, //e.g., arm,cortex-a53
}
impl FdtCpuInfo {
    pub const fn new() -> Self {
        Self {
            release_addr:   0,
            compatible:     "",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FdtDeviceType {
    Serial,
    I2C,
    SPI,
    GPIO,
    Audio,
    Sound,
    DMA,
    IntCtrl,
    WatchDog,
    MailBox,
    SdHost,
    MMC,
    VideoCore,
    VCHIQ, // messaging interface between the kernel and the firmware running on VideoCore
    FrameBuffer,
    Ethernet,
    Unknown,
}

#[derive(Copy, Clone, Debug)]
pub struct FdtPeripheral {
    pub dev_type:           FdtDeviceType,
    pub compat:             &'static str,
    pub mmio_base:          usize,
    pub mmio_len:           usize
}
impl FdtPeripheral {
    pub const fn new() -> Self {
        Self {
            dev_type:       FdtDeviceType::Unknown,
            compat:         "",
            mmio_base:      0,
            mmio_len:       0
        }
    }
}

#[derive(Debug)]
pub struct FdtMachineResources {
    pub machine_name:       &'static str,
    pub machine_model:      &'static str,
    pub machine_compat:     &'static str,
    pub root_addr_cells:    u32,
    pub root_size_cells:    u32,
    pub cpu_count:          u8,
    pub cpus:               [FdtCpuInfo;  Self::MAX_CPU_COUNT],
    pub mmap_count:         u8,
    pub mmap:               [PMMapElement; Self::MAX_MMAP_COUNT],
    pub bus_to_pmem_map:    [BusToPhysMemMapping; Self::MAX_MMAP_COUNT],
    pub bus_to_pmem_count:  u8,
    pub devices:            [FdtPeripheral; Self::MAX_DEVICE_COUNT],
    pub device_count:       u8
}
impl FdtMachineResources {
    const MAX_CPU_COUNT:    usize = 8;
    const MAX_MMAP_COUNT:   usize = 32;
    const MAX_DEVICE_COUNT: usize = 64;
    pub const fn new() -> Self {
        Self {
            machine_name:       "",
            machine_model:      "",
            machine_compat:     "",
            root_addr_cells:    0,
            root_size_cells:    0,
            cpu_count:          0,
            cpus:               [FdtCpuInfo::new(); Self::MAX_CPU_COUNT],
            mmap_count:         0,
            mmap:               [PMMapElement::new(); Self::MAX_MMAP_COUNT],
            bus_to_pmem_map:    [BusToPhysMemMapping::new(); Self::MAX_MMAP_COUNT],
            bus_to_pmem_count:  0,
            devices:            [FdtPeripheral::new(); Self::MAX_DEVICE_COUNT],
            device_count:       0
        }
    }
}
#[derive(Default, Debug)]
struct Fdt{
    base:               usize,
    // Taken from the header (converted to LE)
    total_size:         u32,    // total size of DT block
    struct_off:         u32,    // offset to structure
    strings_off:        u32,    // offset to strings
    mem_rsvmap_off:     u32,    // offset to memory reserve map
    version:            u32,    // format version
    // Cached String offsets (as it appears in the tree) for easy comparison
    so_name:            u32,    // "name"
    so_compatible:      u32,    // "compatible"
    so_model:           u32,    // "model"
    so_addr_cells:      u32,    // "#address-cells"
    so_size_cells:      u32,    // "#size-cells"
    so_dev_type:        u32,    // "device_type"
    so_clock_freq:      u32,    // "clock-frequency"
    so_timer_freq:      u32,    // "timebase-frequency"
    so_status:          u32,    // "status"
    so_spin_tbl_off:    u32,    // "cpu-release-addr",
    so_reg:             u32,    // "reg"
    so_ranges:          u32,
}
impl Fdt {
    const TOKEN_NODE_BEGIN:     u32 = 1;
    const TOKEN_NODE_END:       u32 = 2;
    const TOKEN_PROPERTY:       u32 = 3;
    const TOKEN_TREE_END:       u32 = 9;
}

#[derive(Default, Debug)]
struct FdtNode {
    offset:             u32,
    path:               &'static str,
}
impl FdtNode {
    fn from_offset(fdt: &Fdt, off: u32) -> FdtNode {
        let mut path_len = 0;
        let mut u8p = (fdt.base + off as usize + 4) as *const u8;
        unsafe {
            for i in 0..256 { // Limit the path string to 256 characters (arbitrary)
                if u8p.read() == 0 {
                    path_len = i;
                    break;
                }
                u8p = u8p.add(1);
            }
            Self {
                offset: off,
                path:   str::from_utf8_unchecked(core::slice::from_raw_parts(
                (fdt.base + off as usize + 4) as *const u8, path_len as usize))
            }
        }
    }
}

struct FdtProperty {
    name_so:            u32,    // Offset of the property name in the str table
    value_len:          u32,
    value_addr:         usize,  // Memory address (NOT offset) of property value
}
impl FdtProperty {
    fn from_addr(addr: usize) -> Self {
        let u32p = addr as *const u32;
        unsafe {
            Self {
                name_so:   u32p.add(2).read(),
                value_len: u32::from_le_bytes(u32p.add(1).read().to_be_bytes()),
                value_addr:u32p.add(3) as usize
            }
        }
        
    }
    fn value_as_str(&self) -> &'static str {
        unsafe {
            str::from_utf8_unchecked(core::slice::from_raw_parts(
                    self.value_addr as *const u8, self.value_len as usize - 1))
        }
    }

    fn value_as_u32(&self) -> u32 {
        unsafe {
            u32::from_le_bytes((self.value_addr as *const u32)
                                                .read().to_be_bytes())
        }
    }

    fn value_as_u64(&self) -> u64 {
        unsafe {
            u64::from_le_bytes((self.value_addr as *const u64)
                                                .read().to_be_bytes())
        }
    }

    fn cell_value(&self, vaddr: usize, num_cells: u32) -> usize {
        if num_cells == 1 { // 32-bit address/size field
            unsafe {
                return u32::from_le_bytes( (vaddr as *const u32).read()
                                                .to_be_bytes() ) as usize;
            }
        } else if num_cells == 2 {
            unsafe {
                return u64::from_le_bytes( (vaddr as *const u64).read()
                                                .to_be_bytes() ) as usize;
                }
        } else {
            panic!("Unsupported Address/SizeCell in the Device Tree");
        }
    }

    fn property_name_str(&self, fdt: &Fdt) -> &'static str {
        let straddr = fdt.base + fdt.strings_off as usize +
                        u32::from_le_bytes(self.name_so.to_be_bytes()) as usize;
        let mut u8p = straddr as *const u8;
        let mut strl = 0;
        unsafe {
            for i in 0..256 { // Limit the path string to 256 characters (arbitrary)
                if u8p.read() == 0 {
                    strl = i;
                    break;
                }
                u8p = u8p.add(1);
            }
            str::from_utf8_unchecked(
                    core::slice::from_raw_parts(straddr as *const u8, strl))
        }
    }
}

pub fn fdt_parse_tree(fdt_base: usize, sys_res: &mut FdtMachineResources) -> bool {
    let mut dth;
    unsafe {
        dth = (fdt_base as *const FdtHeader).read_volatile();
        dth.magic       = u32::from_le_bytes(dth.magic.to_be_bytes());
        dth.total_size  = u32::from_le_bytes(dth.total_size.to_be_bytes());
        dth.version     = u32::from_le_bytes(dth.version.to_be_bytes());
        dth.struct_off  = u32::from_le_bytes(dth.struct_off.to_be_bytes());
        dth.strings_off = u32::from_le_bytes(dth.strings_off.to_be_bytes());
        dth.mem_rsvmap_off = u32::from_le_bytes(dth.mem_rsvmap_off.to_be_bytes());
    }
    if dth.magic != 0xD00DFEED {
        return false;
    }
    
    let fdt = Fdt {
        base:           fdt_base,
        total_size:     dth.total_size,
        struct_off:     dth.struct_off,
        strings_off:    dth.strings_off,
        mem_rsvmap_off: dth.mem_rsvmap_off,
        version:        dth.version,
        so_name:        find_str_off(fdt_base, &dth, "name"),
        so_compatible:  find_str_off(fdt_base, &dth, "compatible"),
        so_model:       find_str_off(fdt_base, &dth, "model"),
        so_addr_cells:  find_str_off(fdt_base, &dth, "#address-cells"),
        so_size_cells:  find_str_off(fdt_base, &dth, "#size-cells"),
        so_dev_type:    find_str_off(fdt_base, &dth, "device_type"),
        so_clock_freq:  find_str_off(fdt_base, &dth, "clock-frequency"),
        so_timer_freq:  find_str_off(fdt_base, &dth, "timebase-frequency"),
        so_status:      find_str_off(fdt_base, &dth, "status"),
        so_spin_tbl_off:find_str_off(fdt_base, &dth, "cpu-release-addr"),
        so_reg:         find_str_off(fdt_base, &dth, "reg"),
        so_ranges:      find_str_off(fdt_base, &dth, "ranges")
    };
    // klog!("FDT: {:?}\n", fdt);

    // Walk the DT Struct block and look for /memory    
    let mut cur_node = FdtNode::default();
    let mut root = true;
    while let Some(next_node) = find_next_node(&fdt, &cur_node) {
        cur_node = next_node;
        // klog!("Investigating node {:?}\n", cur_node);
        if root {
            enum_root_node(&fdt, &cur_node, sys_res);
            root = false;
        } else if cur_node.path.starts_with("memory@") {
            enum_mem_device(&fdt, &cur_node, sys_res);
        } else if cur_node.path.starts_with("soc") {
            // Extract the "ranges" property for
            // bus-to-physical address translation
            enum_soc_node(&fdt, &cur_node, sys_res);
        } else if cur_node.path.starts_with("interrupt-controller@") {
            enum_peripheral(&fdt, &cur_node, sys_res,
                                                    FdtDeviceType::IntCtrl);
        } else if cur_node.path.starts_with("serial@") {
            enum_peripheral(&fdt, &cur_node, sys_res,
                                                    FdtDeviceType::Serial);
        } else if cur_node.path.starts_with("i2c@") {
            enum_peripheral(&fdt, &cur_node, sys_res, FdtDeviceType::I2C);
        } else if cur_node.path.starts_with("spi@") {
            enum_peripheral(&fdt, &cur_node, sys_res, FdtDeviceType::SPI);
        } else if cur_node.path.starts_with("mailbox@") {
            enum_peripheral(&fdt, &cur_node, sys_res,
                                                    FdtDeviceType::MailBox);
        } else if cur_node.path.starts_with("mmc@") {
            enum_peripheral(&fdt, &cur_node, sys_res, FdtDeviceType::MMC);
        } else if cur_node.path.starts_with("vec@") { // VideoCore
            enum_peripheral(&fdt, &cur_node, sys_res,
                                                    FdtDeviceType::VideoCore);
        } else if cur_node.path.starts_with("vchiq@") { // VideoCore FW IF
            enum_peripheral(&fdt, &cur_node, sys_res,
                                                    FdtDeviceType::VCHIQ);
        } else {
            if let Some(prop) = find_property(&fdt, &cur_node, fdt.so_dev_type) {
                // klog!("Found a node with device_type - val: \"{}\" ({})\n", 
                //     prop.value_as_str(), prop.value_as_str().len());
                if prop.value_as_str().eq("cpu") {
                    enum_cpu(&fdt, &cur_node, sys_res);
                }
                // klog!("Found a node with device_type: {}\n", prop.value_as_str());
            }
        }
    }
    // Add the reserved memory range to the mmap
    enum_rsvd_mem(&fdt, sys_res);

    // Go over all enumerated devices with mmio/dma base addresses that are
    // presented as addresses and convert them to physical memory address
    // The translation table must be provided by the soc object!
    for i in 0..sys_res.device_count as usize {
        match bus_to_mem_addr(&sys_res, sys_res.devices[i].mmio_base) {
            Some(addr)  => {
                sys_res.devices[i].mmio_base = addr;
            },
            _ => {}
        }
    }
    true
}

fn bus_to_mem_addr(mres: &FdtMachineResources, bus_addr: usize) -> Option<usize>
{
    for i in 0..mres.bus_to_pmem_count as usize {
        if bus_addr >= mres.bus_to_pmem_map[i].bus_addr &&
            bus_addr <= mres.bus_to_pmem_map[i].bus_addr + 
                mres.bus_to_pmem_map[i].length {
            return Some(mres.bus_to_pmem_map[i].mem_addr + bus_addr -
                                            mres.bus_to_pmem_map[i].bus_addr);
        }
    }
    None
}

fn enum_root_node(fdt: &Fdt, node: &FdtNode, result: &mut FdtMachineResources) {
    // Root node: extract name, model, #address-cells and #size-cells
    // klog!("enum_root_node - offset: {}\n", node.offset);
    if let Some(prop) = find_property(fdt, node, fdt.so_name) {
        result.machine_name = prop.value_as_str();
    }
    if let Some(prop) = find_property(fdt, node, fdt.so_model) {
        result.machine_model = prop.value_as_str();
    }
    if let Some(prop) = find_property(fdt, node, fdt.so_compatible) {
        result.machine_compat = prop.value_as_str();
    }
    if let Some(prop) = find_property(fdt, node, fdt.so_addr_cells) {
        result.root_addr_cells = prop.value_as_u32();
    }
    if let Some(prop) = find_property(fdt, node, fdt.so_size_cells) {
        if prop.value_len > 0 {
            result.root_size_cells = prop.value_as_u32();
        }
    }
}

fn enum_soc_node(fdt: &Fdt, node: &FdtNode, res: &mut FdtMachineResources) {
    // Find the ranges address for device bus-address to physical-memory-addr
    // translation.
    // Only supporting Buses with compatible="simple-bus"
    // Might have to look into #address-cells, #size-cells and dma-ranges too!
    if let Some(comp) = find_property(fdt, node, fdt.so_compatible) {
        if !comp.value_as_str().eq("simple-bus") {
            klog!("enum_soc_node compatible=\"{}\" not supported\n",
                                                        comp.value_as_str());
        }
    }
    let address_cells: u32;
    let size_cells: u32;
    if let Some(adrc) = find_property(fdt, node, fdt.so_addr_cells) {
        address_cells = adrc.value_as_u32();
    } else {
        address_cells = res.root_addr_cells;
    }
    if let Some(szc) = find_property(fdt, node, fdt.so_size_cells) {
        size_cells = szc.value_as_u32();
    } else {
        size_cells = res.root_size_cells;
    }
    // klog!("SOC addr/size cells: {}/{}\n", address_cells, size_cells);
    if let Some(ranges) = find_property(fdt, node, fdt.so_ranges) {
        // Each element is formatted as:
        // bus address, parent bus address, size
        let cnt = ranges.value_len / (address_cells * 4 * 2 + size_cells * 4);
        let mut vaddr = ranges.value_addr;
        for _i in 0..cnt {
            // Bus Address (Listed for the device)
            res.bus_to_pmem_map[res.bus_to_pmem_count as usize].bus_addr = 
                                        ranges.cell_value(vaddr, address_cells);
            vaddr += (address_cells * 4) as usize;
            // Parent Bus Address (Physical Memory Address)
            res.bus_to_pmem_map[res.bus_to_pmem_count as usize].mem_addr = 
                                        ranges.cell_value(vaddr, address_cells);
            vaddr += (address_cells * 4) as usize;
            // Range Size
            res.bus_to_pmem_map[res.bus_to_pmem_count as usize].length =
                                        ranges.cell_value(vaddr, size_cells);
            vaddr += (size_cells * 4) as usize;
            // klog!("  BusAddr: {:X} -> MemAddr: {:X}, Len: {:X}\n",
            //     res.bus_to_pmem_map[res.bus_to_pmem_count as usize].bus_addr,
            //     res.bus_to_pmem_map[res.bus_to_pmem_count as usize].mem_addr,
            //     res.bus_to_pmem_map[res.bus_to_pmem_count as usize].length
            // );
            res.bus_to_pmem_count += 1;
        }
    }
}

fn enum_cpu(fdt: &Fdt, node: &FdtNode, res: &mut FdtMachineResources) {
    if res.cpu_count as usize >= FdtMachineResources::MAX_CPU_COUNT {
        return;
    }
    if let Some(prop) = find_property(fdt, node, fdt.so_spin_tbl_off) {
        res.cpus[res.cpu_count as usize].release_addr = prop.value_as_u64();
    }
    res.cpu_count += 1;
}

fn enum_mem_device(fdt: &Fdt, node: &FdtNode, res: &mut FdtMachineResources) {
    // Properties:
    // - device_type : has to be "memory"
    // - reg : This property contains all the physical memory ranges of
    //   your board. It's a list of addresses/sizes concatenated
    //   together, with the number of cells of each defined by the
    //   #address-cells and #size-cells of the root node. For example,
    //   with both of these properties being 2 like in the example given
    //   earlier, a 970 based machine with 6Gb of RAM could typically
    //   have a "reg" property here that looks like:
    //   00000000 00000000 00000000 80000000
    //   00000001 00000000 00000001 00000000
    if let Some(p) = find_property(fdt, node, fdt.so_reg) {
        let cnt = p.value_len / 
                        (res.root_addr_cells * 4 + res.root_size_cells * 4);
        let mut vaddr = p.value_addr;
        for _i in 0..cnt {
            // Decode the base address
            if res.root_addr_cells == 1 { // 32-bit addresses
                unsafe {
                    res.mmap[res.mmap_count as usize].base = 
                            u32::from_le_bytes( (vaddr as *const u32).read()
                                                .to_be_bytes() ) as usize;
                    vaddr += 4;
                }
            } else if res.root_addr_cells == 2 {
                unsafe {
                    res.mmap[res.mmap_count as usize].base = 
                            u64::from_le_bytes( (vaddr as *const u64).read()
                                                .to_be_bytes() ) as usize;
                    vaddr += 8;
                }
            } else {
                panic!("Unsupported AddressCell in the Device Tree");
            }
            // Decode the size
            if res.root_size_cells == 1 { // 32-bit size
                unsafe {
                    res.mmap[res.mmap_count as usize].len = 
                            u32::from_le_bytes( (vaddr as *const u32).read()
                                                .to_be_bytes() ) as usize;
                    vaddr += 4;
                }
            } else if res.root_size_cells == 2 {
                unsafe {
                    res.mmap[res.mmap_count as usize].len = 
                            u64::from_le_bytes( (vaddr as *const u64).read()
                                                .to_be_bytes() ) as usize;
                    vaddr += 8;
                }
            } else {
                panic!("Unsupported SizeCell in the Device Tree");
            }
            res.mmap[res.mmap_count as usize].avail = true;
            res.mmap_count += 1;
        }
        
    }
}

fn enum_rsvd_mem(fdt: &Fdt, res: &mut FdtMachineResources) {
    // Extract the reserved memory region list
    unsafe {
        let rmep = (fdt.mem_rsvmap_off as usize + fdt.base) as *const FdtRsvMapItem;
        let mut rme;
        for i in 0..32 {
            rme = rmep.add(i).read_volatile();
            rme.base_addr   = u64::from_le_bytes(rme.base_addr.to_be_bytes());
            rme.length      = u64::from_le_bytes(rme.length.to_be_bytes());
            if rme.base_addr == 0 && rme.length == 0 {
                break;
            }
            res.mmap[res.mmap_count as usize].base   = rme.base_addr as usize;
            res.mmap[res.mmap_count as usize].len    = rme.length as usize;
            res.mmap_count += 1;
        }
    }
}

fn enum_peripheral(fdt: &Fdt, node: &FdtNode, res: &mut FdtMachineResources,
                    dev_type: FdtDeviceType)
{
    if res.device_count >= 32 {
        klog!("Device registry memory full!\n");
        return;
    }
    let dev = res.device_count as usize;
    // Decode the compatible property so that drives can match
    if let Some(comp) = find_property(fdt, node, fdt.so_compatible) {
        res.devices[dev].compat = comp.value_as_str();
    }
    // Decode the reg property (for MMIO Mapping)
    if let Some(p) = find_property(fdt, node, fdt.so_reg) {
        let cnt = p.value_len / 
                        (res.root_addr_cells * 4 + res.root_size_cells * 4);
        let mut vaddr = p.value_addr;
        for _i in 0..cnt {
            // Decode the base address
            let base = p.cell_value(vaddr, res.root_addr_cells);
            vaddr += (res.root_addr_cells * 4) as usize;
            // Decode the size field
            let len = p.cell_value(vaddr, res.root_size_cells);
            res.devices[dev].mmio_base = base;
            res.devices[dev].mmio_len = len;
            // vaddr += (res.root_size_cells * 4) as usize;
            break; // Don't support more than 1 MMIO register file
        }        
    }
    // TODO: Interrupt Information?
    res.devices[dev].dev_type = dev_type;
    res.device_count += 1;
}



// Returns a byte offset from FDT's base address
// offset: The offset of the current node in the FDT area
//         0 to find the first node
fn find_next_node(fdt: &Fdt, node: &FdtNode) -> Option<FdtNode> {
    if node.offset != 0 && (node.offset < fdt.struct_off || 
                            node.offset >= fdt.total_size) {
        return None;
    }
    let addr;
    if node.offset == 0 {
        // First call -> Find the first/root node
        addr = fdt.base + fdt.struct_off as usize;
    } else {
        addr = fdt.base + node.offset as usize + 4; // Skip the current Node tag
    }
    
    unsafe {
        let mut u32p = addr as *const u32;
        loop {
            let token = u32::from_le_bytes(u32p.read().to_be_bytes());
            if token == Fdt::TOKEN_NODE_BEGIN {
                return Some(FdtNode::from_offset(fdt,
                                            (u32p as usize - fdt.base) as u32));
            } else if token == Fdt::TOKEN_TREE_END {
                break;
            } else if token == Fdt::TOKEN_PROPERTY {
                // Skip the property (it may have a value that matches a token)
                let plen = u32::from_le_bytes(u32p.add(1).read().to_be_bytes());
                // Debug------
                // let prop = FdtProperty::from_addr(u32p as usize);
                // klog!("   SKIP P(offset:{},data_len:{}) pname:{} val:{} \n",
                //     u32p as usize - fdt.base, plen,
                //     prop.property_name_str(fdt), prop.value_as_str());
                // ------------
                if plen == 0 {
                    // no value -> skip token, val_size, prop_name
                    u32p = u32p.add(3);
                } else {
                    u32p = u32p.add(3 + plen as usize / 4);
                }
            } else {
                u32p = u32p.add(1);
            }
            
        }
    }
    None
}

// Returns a byte-offset from FDT's base address
// node : offset of the node in question
// pname: pick from one of the so_* fields in fdt
/*
     * token OF_DT_BEGIN_NODE (that is 0x00000001)
     * for version 1 to 3, this is the node full path as a zero
       terminated string, starting with "/". For version 16 and later,
       this is the node unit name only (or an empty string for the
       root node)
     * [align gap to next 4 bytes boundary]
     * for each property:
        * token OF_DT_PROP (that is 0x00000003)
        * 32-bit value of property value size in bytes (or 0 if no
          value)
        * 32-bit value of offset in string block of property name
        * property value data if any
        * [align gap to next 4 bytes boundary]
     * [child nodes if any]
     * token OF_DT_END_NODE (that is 0x00000002)
*/
fn find_property(fdt: &Fdt, node: &FdtNode, pname: u32) -> Option<FdtProperty> {
    if node.offset < fdt.struct_off || node.offset >= fdt.total_size {
        return None;
    }
    unsafe {
        let mut u32p = (fdt.base + node.offset as usize) as *const u32;
        let mut token = u32::from_le_bytes(u32p.read_volatile().to_be_bytes());

        if token != Fdt::TOKEN_NODE_BEGIN {
            return None;
        }
        u32p = u32p.add(1); // Skip the TOKEN_NODE_BEGIN
        // Iterate over the properties
        while (u32p as usize) < (fdt.base + fdt.total_size as usize) {
            token = u32::from_le_bytes(u32p.read_volatile().to_be_bytes());
            if token == Fdt::TOKEN_PROPERTY {
                let prop_val_len = u32::from_le_bytes(
                                            u32p.add(1).read().to_be_bytes());
                if u32p.add(2).read() == pname {
                    // Found it
                    return Some(FdtProperty {
                        name_so:    pname,
                        value_len:  prop_val_len,
                        value_addr: u32p.add(3) as usize
                    });
                } else {
                    // Skip to the next property
                    u32p = u32p.add(2 + (prop_val_len/4) as usize);
                }
            } else if token == Fdt::TOKEN_NODE_BEGIN ||
                        token == Fdt::TOKEN_NODE_END ||
                        token == Fdt::TOKEN_TREE_END {
                return None;
            }
            u32p = u32p.add(1);
        }
    }
    None
}

// Finds the offset (u32 BE) of the given property name in the FDT's strings
// block and returns it. Used when iterating over the properties of a
// node in the tree
fn find_str_off(fdt_base: usize, fdt_header: &FdtHeader, find: &str) -> u32 {
    let last_addr = fdt_base + fdt_header.total_size as usize;
    let strs_base = fdt_base + fdt_header.strings_off as usize;
    let mut str_addr = strs_base;
    let prop_bytes = find.as_bytes();
    while str_addr < last_addr {
        for i in 0..find.len() {
            let c;
            unsafe { c = ((str_addr + i) as *const u8).read(); }
            if c != prop_bytes[i] {
                break;
            } else if i == find.len() - 1 {
                // Found
                return u32::from_be_bytes(
                    ((str_addr - strs_base) as u32).to_le_bytes());
            }
        }
        
        str_addr += 1;
    }
    u32::MAX // Makes it out of range so it wouldn't match anything
}
