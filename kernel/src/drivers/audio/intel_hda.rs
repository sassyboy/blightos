#![allow(dead_code)]
///
/// BlightOS kernel
///
/// Intel High Definition Audio (HDA) PCI Driver
///
/// Provides:
/// - audio_playback(buffer: &[u8], sample_rate, channels)
/// - audio_stop()
///
/// Only supports Audio Output (DAC) and Audio Input (ADC) widgets.
/// The following widgets are not supported yet:
/// Pin Complex,Mixer, Selector, Power Widget, volume knob
/// 
/// AudioOutput path:
///   <StreamNumber, ChannelId> -> [Optional Processing Node] -> DAC ->
///   [Optional Amp w/ Mute] -> Output
///   Output can be Speakers, Headphones, HDMI, or other internal components
///   such as mixers, selectors, pin complexes, etc.
/// 
/// TODO: Add support for mic using an AudioInput widget.
///       Add support pin complex widgets (See section 7.2.3.3)
///       Add support for headset plug detection
///       Add VFS interfaces for user-space audio playback and recording
///       Test with a large wav file frome the user-space
///       Add multi-stream support for simultaneous playback and recording, etc.
///           via software-based mixing (challenging but fun)
/// OPTIONAL: Use CORB/RIRB for sending commands to codecs and receiving
///           responsesinstead of immediate command interface.
/// 

use core::{hint, ptr, fmt::Debug};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use crate::arch::{MMUMapping};
use crate::drivers::{MMIORegisterFile, pci::*};
use crate::fs::{DirectoryEntry, FileOperation, MountPoint};
use crate::drivers::storage::IOCompletion;
use crate::sched::Task;
use crate::util::*;
use crate::mem::phys::*;

#[cfg(feature="debug_ihda")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[IHDA] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_ihda"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

static HDA_DEVICE: Spinlock<IntelHDA> = Spinlock::new(IntelHDA::new());
static PCM_FILES:  Spinlock<BTreeMap<usize, Arc<PcmFileObject>>> =
                                            Spinlock::new(BTreeMap::new());

pub struct IntelHDA {
    // PCI device information
    regs:           MMIORegisterFile,
    irq:            u8,
    // Codec information
    widgets:        Vec<AudioWidget>, // Discovered audio widgets
    speakers_widx:  Option<usize>,  // Index of AudioOuput widget connected to
                                    // speakers in the widgets list
    // Stream information
    ostream:        [HDAStream; 2], // Output streams x 2 for double buffering
    istream:        [HDAStream; 2], // Input streams
    ostream_cnt:    u8,             // Number of output streams supported
    istream_cnt:    u8,             // Number of input streams supported
    next_os:        u8,             // Index of the next ostream for playback
    // HDA state information
    enabled:        bool,
    playing:        bool,
    recording:      bool,
    output_mute:    bool,
    output_gain_l:  u8, // 0 to MAX_GAIN
    output_gain_r:  u8, // 0 to MAX_GAIN
    // User-space interface
    next_pcm_hnd:   usize, // Handle for the next PCM file to be opened
}

impl IntelHDA {
    const fn new() -> Self {
        Self {
            regs:           MMIORegisterFile::new(0, 0),
            irq:            0,
            widgets:        Vec::new(),
            speakers_widx:  None,
            ostream:        [HDAStream::new(); 2],
            istream:        [HDAStream::new(); 2],
            ostream_cnt:    0,
            istream_cnt:    0,
            next_os:        0,
            enabled:        false,
            playing:        false,
            recording:      false,
            output_mute:    false,
            output_gain_l:  Self::MAX_GAIN,
            output_gain_r:  Self::MAX_GAIN,
            next_pcm_hnd:   Self::DEV_HND_PCM_FILE_BASE,
        }
    }
    
    ///
    /// Module Interface
    ///
    const INTEL_VENDOR_ID: u16 = 0x8086;
    const CLASS_MULTIMEDIA: u8 = 0x04;
    const SUBCLASS_AUDIO: u8 = 0x03;
    
    /// Enumerate PCI devices and store the first compatible Intel HDA device,
    /// returning the number of compatible devices found (0 or 1).
    pub fn enumerate() -> usize {
        // PCI devices are pre-enumerated and available via PCI_DEVICES.lock()
        let pci_devs = PCI_DEVICES.lock();
        for dev in &pci_devs.dev_lst {
            if dev.vendor_id != Self::INTEL_VENDOR_ID || 
                dev.class != Self::CLASS_MULTIMEDIA ||
                dev.sub_class != Self::SUBCLASS_AUDIO {
                continue; // Not an Intel HDA device
            }
            // Get MMIO BAR (usually BAR0)
            if let Some((mmio_base, mmio_len)) = dev.get_bar_address(0) {
                // Enable device memory and bus mastering
                dev.enable_memspace();
                dev.enable_bus_master();
                let irq = dev.irq_line;

                dbg!("Intel HDA device found at PCI address {}:{}:{} \
                    VendorId: {:X}, DeviceId: {:X}\n    \
                    MMIO: ({:#X}, {:X}), IRQ: {}, CMD: {:X?}, STS: {:X?}, \
                    HDA.GCAP: {:X}\n",
                    dev.bus_id, dev.slot_id, dev.func_id,
                    dev.vendor_id, dev.device_id,
                    mmio_base, mmio_len, irq,
                    dev.get_command(), dev.get_status(),
                    mmio_read16(mmio_base, Self::REG_GCAP));

                // Register the IRQ handler
                let gsi = dev.irq_line as u32;
                let vec = dev.irq_line;
                crate::arch::irq_reroute(gsi, vec, true);
                crate::arch::isr_register(vec as u16, Self::irq_handler);
                // Set up the HDA instance
                let mut hda = HDA_DEVICE.lock();
                hda.regs.base_virt = MMUMapping::dma_from_kernel_phys(mmio_base);
                hda.regs.length    = mmio_len;
                hda.irq         = irq;
                // Initialize the controller
                hda.init();
                // Register a mont-point for user-space access
                let mnt_obj = MountPoint {
                    name:       String::from("audio"),
                    fops:       Self::fops_handler
                };
                MountPoint::mount(mnt_obj);
                return 1;
            } else {
                continue;
            }
        }
        0
    }

    pub fn post_enum() {
        // TODO - Spawn the mixer thread
    }

    pub fn release( _device: usize) {
    }

    ///
    /// Intel HDA Controller
    /// 
    const MAX_STREAMS: usize = 15; // Max streams supported by the controller
    // HDA Register offsets
    const REG_GCAP:         usize = 0x00; // Global Capabilities (2 bytes)
    const REG_GCTL:         usize = 0x08; // Global control (4 bytes)
    const REG_STATESTS:     usize = 0x0E; // State change status (2 bytes)
    const REG_GSTS:         usize = 0x10; // Global status (2 bytes)
    
    const REG_INTCTL:       usize = 0x20; // Interrupt Control      
    const REG_INTSTS:       usize = 0x24; // Interrupt Status
    const REG_SSYNC:        usize = 0x38; // Stream Synchronization (4 bytes)
    // CORB: Command Output Ring Buffer
    const REG_CORB_LBASE:   usize = 0x40; // CORB Lower Base Address (4 bytes)
    const REG_CORB_UBASE:   usize = 0x44; // CORB Upper Base Address (4 bytes)
    const REG_CORB_WPTR:    usize = 0x48; // CORB Write Pointer      (2 bytes)
    const REG_CORB_RPTR:    usize = 0x4A; // CORB Read Pointer       (2 bytes)
    const REG_CORB_CTL:     usize = 0x4C; // CORB Control            (1 byte)
    const REG_CORB_STS:     usize = 0x4D; // CORB Status             (1 byte)
    const REG_CORB_SIZE:    usize = 0x4E; // CORB Size               (1 byte)
    // // RIRB: Response Input Ring Buffer
    const REG_RIRB_LBASE:   usize = 0x50; // RIRB Lower Base Address (4 bytes)
    const REG_RIRB_UBASE:   usize = 0x54; // RIRB Upper Base Address (4 bytes)
    const REG_RIRB_WPTR:    usize = 0x58; // RIRB Write Pointer      (2 bytes)
    const REG_RINT_CNT:     usize = 0x5A; // Response Interrupt Count(2 bytes)
    const REG_RIRB_CTL:     usize = 0x5C; // RIRB Control            (1 byte)
    const REG_RIRB_STS:     usize = 0x5D; // RIRB Status             (1 byte)
    const REG_RIRB_SIZE:    usize = 0x5E; // RIRB Size               (1 byte)
    // Immediate Command Output/Input Registers (for simple verbs without using CORB/RIRB)
    const REG_ICOI:         usize = 0x60; // Immediate Command Output Interface
    const REG_IRII:         usize = 0x64; // Immediate Response Input Interface
    const REG_ICS:          usize = 0x68; // Immediate Command Status
    const REG_STREAMS_BASE: usize = 0x80; // Base offset for stream descriptors
    // Control bits and flags
    const GCTL_CRST: u32 = 1 << 0;

    const MAX_GAIN: u8 = 0x7F; // Max gain value for the codecs' amplifiers

    /// Basic controller reset
    fn reset_controller(&mut self) {
        // Toggle CRST in global control to reset controller.
        // Writing a 0 to CRST causes the HDA to transition to the Reset state.
        // After the hardware has completed sequencing into the reset state, it
        // will report a 0 in this bit.
        // When a 1 is written to the CRST bit, the controller will go through
        // the sequence of steps necessary to take itself out of reset. The link
        // will be started, and state machines will initialize themselves.
        // While the hardware is taking these steps, the CRST bit, if read, will
        //still appear to be 0. When the initialization has been completed, a
        // read of the CRST bit will return a 1 indicating that the controller
        // is now ready to function. Therefore, after taking the controller out
        // of reset, the software should wait until CRST is read as 1 before
        // continuing.

        let mut gctl : u32 = self.regs.read(Self::REG_GCTL);
        // Put the controller in reset and wait until it transitions to reset
        gctl &= !Self::GCTL_CRST;
        self.regs.write(Self::REG_GCTL, gctl);
        let mut tries = 10000;
        while self.regs.read::<u32>(Self::REG_GCTL) & Self::GCTL_CRST != 0
                                                                && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        // Set CRST to 1 to bring controller out of reset and wait for it to
        // complete initialization
        gctl |= Self::GCTL_CRST;
        self.regs.write(Self::REG_GCTL, gctl);
        // wait for GCTL_CRST to set
        tries = 10000;
        while self.regs.read::<u32>(Self::REG_GCTL) & Self::GCTL_CRST != 1
                                                                && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        // Wait for codecs to report state
        // Available codecs are reported by the hardware by setting bits in the
        // REG_STATESTS
        tries = 10000;
        while self.regs.read::<u16>(Self::REG_STATESTS) == 0 && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        if self.regs.read::<u32>(Self::REG_GCTL) & Self::GCTL_CRST == 0 {
            klog!("Error: Controller failed to come out of reset\n");
        } else if self.regs.read::<u16>(Self::REG_STATESTS) == 0 {
            klog!("Warning: Controller reset complete but no codec state reported\n");
        } else {
            // Enable global interrupts
            // Individual stream interrupts will be enabled later.
            self.regs.write::<u32>(Self::REG_INTCTL, 0xC0000000);

            dbg!("Controller reset complete, GCTL:{:X}, state status: {:X}, \
                  SSYNC: {:X}\n",
                    self.regs.read::<u32>(Self::REG_GCTL),
                    self.regs.read::<u16>(Self::REG_STATESTS),
                    self.regs.read::<u32>(Self::REG_SSYNC));
        }
    }

    fn init_streams(&mut self) {
        // Enumerate and initialize the audio streams
        // Stream IDs [0           to istream_cnt) are used for input streams
        // Stream IDs [istream_cnt to istream_cnt + ostream_cnt): output streams
        let gcap : u16 = self.regs.read(Self::REG_GCAP);
        self.ostream_cnt = ((gcap >> 8) & 0xF) as u8;
        self.istream_cnt = ((gcap >> 12) & 0xF) as u8;
        if self.ostream_cnt < 2 || self.istream_cnt < 2 {
            klog!("Error: Controller does not support enough streams 
                    ({} output, {} input)\n",
                self.ostream_cnt, self.istream_cnt);
            return;
        }
        dbg!("Controller supports {} output streams, {} input streams\n",
            self.ostream_cnt, self.istream_cnt);

        let mut int_clt : u32 = self.regs.read(Self::REG_INTCTL);
        let mut stream_num  = 1;
        let mut stream_mmio = self.regs.base_virt + Self::REG_STREAMS_BASE;
        // Initialize the first 2 input streams
        for i in 0..self.istream_cnt as usize {
            // Only using one input stream for now
            if i < 2 {
                self.istream[i].init(stream_num, stream_mmio);
                // Set the INTCTL.SIE bit for this stream
                int_clt |= 1 << (stream_num - 1);
            }
            stream_num  += 1;
            stream_mmio += 0x20; // Each stream's registers are 0x20 bytes apart
        }
        // Initialize the first 2 output streams
        for i in 0..self.ostream_cnt as usize {
            if i < 2 {
                self.ostream[i].init(stream_num, stream_mmio);
                // Set the INTCTL.SIE bit for this stream
                int_clt |= 1 << (stream_num - 1);
            }
            stream_num  += 1;
            stream_mmio += 0x20; // Each stream's registers are 0x20 bytes apart
        }
        
        // Enable stream interrupts
        self.regs.write::<u32>(Self::REG_INTCTL, int_clt);
    }

    /// Enumerates the available codecs to indentify which pin complexes are
    /// connected to speakers, headphone sockets, microphones, etc.
    /// It then initializes them by setting their power state to D0 (fully on)
    /// and configuring them with an initial gain/mute state, etc.
    /// Configures the speakers and the headphones to use the same DAC
    fn init_codecs(&mut self) -> bool{
        // QEMU Defaults:
        // Node 0 (Root) -> 1 (FG) -> 2 (DAC) -> 3 (Pin Complex / Line Out)
        if let Some(_vid) = self.get_vendor_id(0, 0) {
            dbg!("Codec 0, Node 0 Vendor ID: {:X}\n", _vid);
        }

        // Get the number of nodes under Root
        let node_count;
        let first_node_id;
        if let Some(resp) = self.get_subordinate_node_count(0, 0) {
            node_count = resp & 0xFF;
            first_node_id = (resp >> 16) & 0xFF;
            dbg!("Root node has {} subordinate nodes - starting ID: {}\n",
                node_count & 0xFF, first_node_id);
            
        } else {
            klog!("Failed to get subordinate node count for root node\n");
            return false;
        }

        // Enumerate the nodes and collect information about
        // Audio Function Groups
        for i in 0..node_count {
            let nid = (first_node_id + i) as u8;
            let Some(ntype) = self.get_node_type(0, nid) else {
                klog!("Failed to get node type for node ID: {}\n", nid);
                continue;
            };
            if ntype & 0xFF != 0x1 {
                continue; // Not an Audio Function Group
            }
            dbg!("Found Audio Function Group at Node ID: {}\n", nid);
            // Get the nodes (widgets) under this function group and their types
            //to identify DACs, pin complexes, etc.
            let Some(wcnt_resp) = self.get_subordinate_node_count(0, nid) else {
                klog!("Failed to enumerate nodes under NID: {}\n", nid);
                continue;
            };
            let wcnt = wcnt_resp & 0xFF;
            let wfirst = (wcnt_resp >> 16) & 0xFF;
            dbg!("Audio Function Group NID {} has {} widgets - start wID: {}\n",
                nid, wcnt, wfirst);
            for j in 0..wcnt {
                let w_nid = (wfirst + j) as u8;
                let Some(w_cap) = self.get_widget_capabilities(0, w_nid) else {
                    klog!("Failed to get capabilites of wID: {}\n", w_nid);
                    continue;
                };
                let conn_list = self.get_widget_connection_list(0, w_nid);
                let pin_conf = self.get_pin_configuration(0, w_nid);
                let mut widget = AudioWidget {
                    afg_nid: nid,
                    nid: w_nid,
                    caps: w_cap,
                    pin_caps: None,
                    conn_list: conn_list,
                    pin_conf
                };
                if widget.widget_type() == WidgetType::PinComplex {
                    if let Some(pin_caps) = 
                                self.get_pin_complex_capabilities(0, w_nid) {
                        widget.pin_caps = Some(pin_caps);
                    } else {
                        klog!("Failed to get pin-complex caps of wID: {}\n",
                                w_nid);
                    }
                }
                if widget.widget_type() == WidgetType::AudioOutput ||
                    widget.widget_type() == WidgetType::PinComplex {
                    dbg!("  {:X?}\n", widget);
                }
                
                self.widgets.push(widget);
            }
        }

        // Select the first AudioOutput as the default for speakers
        for (idx, widget) in self.widgets.iter().enumerate() {
            if widget.widget_type() == WidgetType::AudioOutput {
                self.speakers_widx = Some(idx);
                dbg!("Selected default output: AFG Node {}, DAC Node {} \
                       (Output Amp? {})\n",
                        widget.afg_nid, widget.nid,
                        widget.out_amp_present()
                );
                break;
            }
        }

        // Prepare the default output for playback
        if let Some(widx) = self.speakers_widx {
            let afg_nid = self.widgets[widx].afg_nid;
            let dac_nid = self.widgets[widx].nid;

            // AFG Setup
            // Power on the Audio Function Group of the default output
            self.set_power_state(0, afg_nid, 0); // D0 = fully on

            // DAC Setup
            // Power on the Audio Output/DAC
            self.set_power_state(0, dac_nid, 0); // D0
            // Set Amp Gain/Mute for the speakers
            self.apply_output_gain();

            // Pin Complex Setup
            // self.set_power_state(0, pin_nid, 0); // D0
            // Enable Output and Headphone gates of the pin complex
            // self.set_pin_control(0, pin_nid, false, true, true);
            // Set Amp Gain/Mute for pin complex to unmute and max gain (0x7F)
            // self.set_amp_gain(0, pin_nid, 0,
            //                     false,  // Set input amp gain
            //                     true,   // Set output amp gain
            //                     true,   // Set left channel
            //                     true,   // Set right channel
            //                     0x7F,   // Gain value (0-0x7F)
            //                     false   // Mute
            // );
            // Enable the extrenal AMP (EAPD) for the pin complex if supported
            // self.set_eapd_btl(0, pin_nid, true, false, false);

            // Debug
            let Some(_root_pstate) = self.get_power_state(0, 0) else {
                klog!("Failed to get power state of root node\n");
                return false;
            };
            let Some(_afg_pstate) = self.get_power_state(0, afg_nid) else {
                klog!("Failed to get power state of AFG node {}\n", afg_nid);
                return false;
            };
            let Some(_dac_pstate) = self.get_power_state(0, dac_nid) else {
                klog!("Failed to get power state of DAC node {}\n",dac_nid);
                return false;
            };
            dbg!("PStates - Root: {:X}, AFG Node {}: {:X}, DAC Node {}: {:X}\n",
                    _root_pstate, afg_nid, _afg_pstate, dac_nid, _dac_pstate);
        } else {
            klog!("No suitable default output found\n");
            return false;
        }
        
        true
    }

    fn init(&mut self) {
        if self.enabled {
            return;
        }
        self.reset_controller();
        self.init_codecs();
        self.init_streams();
        self.set_output_format(2, 48000, 16); // Default: stereo, 48KHz, 16-bit
        self.enabled = true;
    }

    /// Configures the audio format of all streams
    pub fn set_output_format(&mut self, ch: u8, srate: u32, bps: u8) -> bool {
        if !self.enabled {
            return false;
        }
        // Set the format for both output streams
        if !self.ostream[0].set_audio_format(ch, srate, bps) ||
            !self.ostream[1].set_audio_format(ch, srate, bps) {
            klog!("Failed to set audio format for output streams\n");
            return false;
        }
        // Configure the speakers' DAC with the same format
        let Some(widx) = self.speakers_widx else {
            klog!("No default output configured to set the format.\n");
            return false;
        };
        let fmt = self.ostream[0].audio_format_as_u16();
        let dac_nid = self.widgets[widx].nid;
        self.set_converter_format(0, dac_nid, fmt)
    }

    /// Queues a buffer of PCM (pulse-code modulation) data for playback on
    /// one of the output streams and returns the number of data bytes
    /// successfully queued.
    /// It also starts playing back if necessary.
    /// 
    /// The caller is responsible to make subsequent calls to this function
    /// should the PCM data cannot fit in the stream's buffer all at once.
    /// The caller is also responsible for converting the PCM data to the format
    /// set for the HDA (via set_format).
    /// 
    /// TODO: Add support for appending vs mixing with the currently running
    ///       stream.
    pub fn queue_playback(&mut self, pcm: &[u8]) -> usize {
        if !self.enabled {
            return 0;
        }

        // Write the PCM data into the DMA buffer associated with the next
        // stream to be played.
        let bytes_written = self.ostream[self.next_os as usize].push(pcm);

        // Start playback if not already playing and return
        self.play_audio();
        bytes_written
    }

    pub fn play_audio(&mut self) {
        if self.playing {
            // Should wait for the currently playing stream to either finish
            // or be stopped before switching to the next stream
            return;
        } 
        let Some(widx) = self.speakers_widx else {
            klog!("No default output configured to set the format.\n");
            return;
        };
        if !self.ostream[self.next_os as usize].is_ready(){
            klog!("Stream #{} is not ready to play. PCM WP: {}, LVI: {}, CBL: {}\n",
                self.ostream[self.next_os as usize].number, 
                self.ostream[self.next_os as usize].pcm_wp,
                self.ostream[self.next_os as usize].read_lvi_reg(),
                self.ostream[self.next_os as usize].read_cbl_reg());
            return;
        }
        let dac_nid = self.widgets[widx].nid;
        let next_snum = self.ostream[self.next_os as usize].number;
        self.set_converter_stream_channel_mapping(0, dac_nid, next_snum, 0);
        self.ostream[self.next_os as usize].run(true);
        self.playing = true;
        dbg!("Starting playback on stream #{}\n",
                self.ostream[self.next_os as usize].number);
        self.next_os = (self.next_os + 1) % 2;
    }
    /// Stop playback and idle the controller/stream.
    pub fn stop_audio(&mut self) {
        if !self.playing {
            return;
        }
        // Stop the currently playing stream
        let cur_sindx = (self.next_os + 1) as usize % 2;
        self.ostream[cur_sindx].stop();
        self.playing = false;

        self.next_os = (self.next_os + 1) % 2;
    }

    pub fn apply_output_gain(&mut self) {
        if let Some(widx) = self.speakers_widx {
            let dac_nid = self.widgets[widx].nid;
            self.set_amp_gain(0, dac_nid, 0,
                                false,  // Set input amp gain
                                true,   // Set output amp gain
                                true,   // Set left channel
                                false,   // Set right channel
                                self.output_gain_l ,  // Gain value (0-0x7F)
                                self.output_mute    // Mute
            );
            self.set_amp_gain(0, dac_nid, 0,
                                false,  // Set input amp gain
                                true,   // Set output amp gain
                                false,   // Set left channel
                                true,   // Set right channel
                                self.output_gain_r ,  // Gain value (0-0x7F)
                                self.output_mute    // Mute
            ); 
        }
    }

    fn irq_handler(_irq: u16){
        let mut hda = HDA_DEVICE.lock();
        // let intsts = mmio_read32(hda.mmio_base, Self::REG_INTSTS);
        // Switch to the next stream.
        let next_os = hda.next_os as usize;
        if hda.ostream[next_os].is_ready() {
            let Some(widx) = hda.speakers_widx else {
                return;
            };
            let dac_nid = hda.widgets[widx].nid;
            let next_snum = hda.ostream[next_os].number;
            hda.set_converter_stream_channel_mapping(0, dac_nid, next_snum, 0);
            hda.ostream[next_os].run(true);
            hda.playing = true;
        } else {
            // No stream is ready to play, so just mark as not playing
            hda.playing = false;
        }
        // Clear the INT status for the stream finished playing, and stop it
        //
        let next_os = (hda.next_os + 1) % 2;
        hda.ostream[next_os as usize].stop();
        hda.next_os = next_os;
        //mmio_write32(hda.mmio_base, Self::REG_INTSTS, 0);
        
    }

    //
    // Codec communication via CORB/RIRB
    // Codec Verb Format:
    // [31:28] CAd (Codec Address)
    // [27:20] NId (Node ID)
    // [19:0] Payload (Verb ID[19:8] and parameters[7:0])
    //
    // See section 7.3.3 for details on verb encoding and common verbs.
    //
    const VERBID_GET_PARAM:     u32 = 0xF00;    // Section 7.3.3.1
    const VERBID_GET_CONN_ENT:  u32 = 0xF02;    // Section 7.3.3.3
    const VERBID_GET_AMP_GAIN:  u32 = 0xB;      // Section 7.3.3.7
    const VERBID_SET_AMP_GAIN:  u8  = 0x3;      // Section 7.3.3.7
    const VERBID_GET_FORMAT:    u8  = 0xA;      // Section 7.3.3.8
    const VERBID_SET_FORMAT:    u8  = 0x2;      // Section 7.3.3.8
    const VERBID_GET_PWR_STATE: u32 = 0xF05;    // Section 7.3.3.10
    const VERBID_SET_PWR_STATE: u32 = 0x705;    // Section 7.3.3.10
    const VERBID_GET_CNV_CTRL:  u32 = 0xF06;    // Section 7.3.3.11
    const VERBID_SET_CNV_CTRL:  u32 = 0x706;    // Section 7.3.3.11
    const VERBID_GET_PIN_CTRL:  u32 = 0x707;    // Section 7.3.3.13
    const VERBID_SET_PIN_CTRL:  u32 = 0x707;    // Section 7.3.3.13
    const VERBID_GET_EAPD_BTL:  u32 = 0xF0C;    // Section 7.3.3.16
    const VERBID_SET_EAPD_BTL:  u32 = 0x70C;    // Section 7.3.3.16
    const VERBID_GET_CONF_DEF:  u32 = 0xF1C;    // Section 7.3.3.31 (Pin Widget)
    const VERBID_SET_CONF_B0:   u32 = 0x71C;    // Section 7.3.3.31 (Pin Widget)
    const VERBID_SET_CONF_B1:   u32 = 0x71D;    // Section 7.3.3.31 (Pin Widget)
    const VERBID_SET_CONF_B2:   u32 = 0x71E;    // Section 7.3.3.31 (Pin Widget)
    const VERBID_SET_CONF_B3:   u32 = 0x71F;    // Section 7.3.3.31 (Pin Widget)

    // Parameter IDs for VERBID_GET/SET_PARAM - Section 7.3.4
    const CODEC_PARAM_VENDOR_ID:u32 = 0x00;
    const CODEC_PARAM_NODE_CNT: u32 = 0x04;
    const CODEC_PARAM_FG_TYPE:  u32 = 0x05; // Function Group Type
    const CODEC_PARAM_WIDGET_CAP:u32= 0x09; // Get Audio Widget Capabilities
    const CODEC_PARAM_PIN_CAP:  u32 = 0x0C; // Get Pin Complex Capabilities
    const CODEC_PARAM_INP_AMP_CAP:u32 = 0x0D; // Get Input Amplifier Capabilities
    const CODEC_PARAM_OUT_AMP_CAP:u32 = 0x12; // Get Output Amplifier Capabilities
    const CODEC_PARAM_CONN_LIST_CNT:u32 = 0x0E; // Get Connection List Count


    fn get_vendor_id(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_VENDOR_ID);
        self.send_verb(verb)
    }

    /// Response format:
    /// [23:16] Starting Node Number
    /// [07:00] Total number of nodes
    /// See Section 7.3.4.3 - Subordinate Node Count
    fn get_subordinate_node_count(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_NODE_CNT);
        self.send_verb(verb)
    }

    /// Response format:
    /// [8]: Unsolicited Response Capable
    /// [7:0]: Function Group Type (e.g. 1 = Audio Function Group)
    fn get_node_type(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_FG_TYPE);
        self.send_verb(verb)
    }

    /// Response format:
    /// See Section 7.3.4.6 - Audio Widget Capabilities
    fn get_widget_capabilities(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_WIDGET_CAP);
        self.send_verb(verb)
    }

    /// Response format:
    /// See Section 7.3.4.11 - Connection List Count
    /// Applies to AudioInputConverter, Mixer Widget, Selector Widget,
    /// Pin Widget and Power Widgets.
    fn get_widget_connection_list(&mut self, cad: u8, nid: u8) -> Option<Vec<u16>> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_CONN_LIST_CNT);
        let Some(resp) = self.send_verb(verb) else {
            return None;
        };
        let count = resp & 0x7F;
        let long_form = (resp & 0x8) != 0;
        let mut lst = Vec::new();
        let mut idx = 0;
        while idx < count {
            let verb = Self::build_verb(cad, nid, Self::VERBID_GET_CONN_ENT, idx);
            let Some(conn) = self.send_verb(verb) else {
                break;
            };
            // Long form returns 2 16-bit NIDs short form returns 4 8-bit NIDs
            if long_form {
                lst.push((conn & 0xFFFF) as u16);
                idx += 1;
                if idx == count {
                    break;
                }
                lst.push((conn >> 16) as u16);
                idx += 1;
            } else {
                lst.push((conn & 0xFF) as u16);
                idx += 1;
                if idx == count {
                    break;
                }
                lst.push(((conn >> 8) & 0xFF) as u16);
                idx += 1;
                if idx == count {
                    break;
                }
                lst.push(((conn >> 16) & 0xFF) as u16);
                idx += 1;
                if idx == count {
                    break;
                }
                lst.push(((conn >> 24) & 0xFF) as u16);
                idx += 1;
            }
        }
        Some(lst)
    }

    /// Response format:
    /// See Section 7.3.4.12 - Pin Configuration
    fn get_pin_configuration(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_CONF_DEF, 0);
        self.send_verb(verb)
    }

    /// Response format:
    /// See Section 7.3.4.9 - Pin Capabilities
    fn get_pin_complex_capabilities(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PARAM,
                                        Self::CODEC_PARAM_PIN_CAP);
        self.send_verb(verb)
    }

    fn set_power_state(&mut self, cad: u8, nid: u8, state: u8) -> bool {
        let verb = Self::build_verb(cad, nid, Self::VERBID_SET_PWR_STATE,
                                                                state as u32);
        dbg!("Sending Set Power State, verb: {:X}\n", verb);
        self.send_verb(verb).is_some()
    }

    /// Response format: Table 82 - Section 7.3.3.10 Power State
    /// [31:11] Reserved (zero)
    /// [10]    PS-SettingsReset
    /// [9]     PS-ClkStopOk
    /// [8]     PS-Error
    /// [7:4]   PS-Act: Actual Power State
    /// [3:0]   PS-Set: Set Power State
    fn get_power_state(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_PWR_STATE, 0);
        self.send_verb(verb)
    }

    fn set_amp_gain(&mut self, cad: u8, nid: u8, selector_index: u8,
                    set_inp_amp: bool, set_out_amp: bool,
                    set_left: bool, set_right: bool,
                    gain: u8, mute: bool) -> bool {
        let g = (gain & 0x7F) as u16 |
                (if mute { 0x80 } else { 0 }) |
                (selector_index as u16 & 0xF) << 8|
                (if set_right   { 1 << 12 } else { 0 }) |
                (if set_left    { 1 << 13 } else { 0 }) |
                (if set_inp_amp { 1 << 14 } else { 0 }) |
                (if set_out_amp { 1 << 15 } else { 0 });
        
        let verb = Self::build_verb16(cad, nid, Self::VERBID_SET_AMP_GAIN, g);
        dbg!("Setting amp gain, verb: {:X}\n", verb);
        let ret = self.send_verb(verb).is_some();
        if ret {
            if set_out_amp {
                if set_left {
                    self.output_gain_l = gain;
                    self.output_mute = mute;
                }
                if set_right {
                    self.output_gain_r = gain;
                    self.output_mute = mute;
                }
            }
        }
        ret
    }



    /// Response format: Table 93 - Section 7.3.3.16 EAPD/BTL Enable
    /// [2]: L-R Swap
    /// [1]: EAPD Enable (External Amplifier Power Up = 1)
    /// [0]: BTL (Output is in balanced mode = 1)
    fn get_eapd_btl(&mut self, cad: u8, nid: u8) -> Option<u32> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_EAPD_BTL, 0);
        self.send_verb(verb)
    }

    fn set_eapd_btl(&mut self, cad: u8, nid: u8, eapd_enable: bool, 
                    btl_enable: bool, lr_swap: bool) -> bool {
        let val =   (if lr_swap     { 1 << 2 } else { 0 }) |
                    (if eapd_enable { 1 << 1 } else { 0 }) |
                    (if btl_enable  { 1 << 0 } else { 0 });
        let verb = Self::build_verb(cad, nid, Self::VERBID_SET_EAPD_BTL, val);
        dbg!("Setting EAPD/BTL, verb: {:X}\n", verb);
        self.send_verb(verb).is_some()
    }

    fn set_pin_control(&mut self, cad: u8, nid:u8, inp_enable: bool,
                        out_enable: bool, hpn_enable: bool) -> bool {
        let val =   (if inp_enable  { 1 << 5 } else { 0 }) |
                    (if out_enable  { 1 << 6 } else { 0 }) |
                    (if hpn_enable  { 1 << 7 } else { 0 });
        let verb = Self::build_verb(cad, nid, Self::VERBID_SET_PIN_CTRL, val);
        dbg!("Setting pin control, verb: {:X}\n", verb);
        self.send_verb(verb).is_some()
    }

    fn set_converter_format(&mut self, cad: u8, nid: u8, fmt: u16) -> bool {
        let verb = Self::build_verb16(cad, nid, Self::VERBID_SET_FORMAT, fmt);
        dbg!("Setting converter format, verb: {:X}\n", verb);
        self.send_verb(verb).is_some()
    }

    fn get_converter_format(&mut self, cad: u8, nid: u8) -> Option<u16> {
        let verb = Self::build_verb16(cad, nid, Self::VERBID_GET_FORMAT, 0);
        self.send_verb(verb).map(|resp| resp as u16)
    }

    /// Maps a DAC (AudioOutput) or an ADC (AudioInput) widget to a stream
    /// and channel.
    /// Stream is an integer representing the link stream used by the converter
    /// for data input or output. 0000b is stream 0, 0001b is stream 1, etc.
    /// Although the link is capable of transmitting any stream number,
    /// by convention stream 0 is reserved as unused so that converters whose
    /// stream numbers have been reset to 0 do not unintentionally decode data
    /// not intended for them.
    /// Channel is an integer representing the lowest channel used by the
    /// converter. If the converter is a stereo converter, the converter will
    /// use the channel provided, as well as channel+1, for its data input or
    /// output.
    /// 
    /// nid only applies to Input/Output Converters.
    fn set_converter_stream_channel_mapping(&mut self, cad: u8, nid: u8,
                                        stream_id: u8, channel: u8) -> bool {
        let m = ((channel & 0xF) | ((stream_id & 0xF) << 4)) as u32;
        let verb = Self::build_verb(cad, nid, Self::VERBID_SET_CNV_CTRL, m);
        dbg!("Setting converter stream channel mapping, verb: {:X}\n", verb);
        self.send_verb(verb).is_some()
    }

    /// Response format: Table 85 - Section 7.3.3.11
    /// Returns (stream_id, channel) mapping for the converter widget
    fn get_converter_stream_channel_mapping(&mut self, cad: u8, nid: u8) ->
                                                            Option<(u8, u8)> {
        let verb = Self::build_verb(cad, nid, Self::VERBID_GET_CNV_CTRL, 0);
        self.send_verb(verb).map(|resp| {
            let stream_id = ((resp >> 4) & 0xF) as u8;
            let channel = (resp & 0xF) as u8;
            (stream_id, channel)
        })
    }

    fn build_verb(cad: u8, nid: u8, verb_id: u32, payload: u32) -> u32 {
        ((cad as u32) << 28) | ((nid as u32) << 20) | (verb_id << 8) |
            (payload & 0xFF)
    }

    fn build_verb16(cad: u8, nid: u8, verb_id: u8, payload: u16) -> u32 {
        ((cad as u32) << 28) | ((nid as u32) << 20) |
        (((verb_id & 0xF) as u32) << 16) | (payload as u32)
    }

    /// Uses the Immediate Command interface to send a verb and returns the
    /// response.
    fn send_verb(&mut self, verb: u32) -> Option<u32> {
        self.regs.write::<u32>(Self::REG_ICOI, verb);
        self.regs.write::<u16>(Self::REG_ICS, 1); // send
        // Wait for command to be processed (ICS bit set)
        let mut tries = 10000;
        while self.regs.read::<u16>(Self::REG_ICS) & 1 == 1 && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        // Read response from IRII
        let response = self.regs.read::<u32>(Self::REG_IRII);
        Some(response)
    }

    ///
    /// VFS Interface
    /// audio:
    ///   |---- input (Dir: Default input, e.g., microphone) TODO
    ///   |---- output (Dir: Default output, e.g., speakers)
    ///   |  |--- fmt (File: Read/Write the current audio output format as text
    ///   |  |         e.g., "2ch,48000Hz,16bit")
    ///   |  |--- mute (File: Read/Write the mute state, e.g., "1" or "0")
    ///   |  |--- pcm (File: PCM data can be written here for playback)
    ///   |  |--- vol (File: Read/Write the current volume level as a percentage
    ///   |            e.g., "L:75,R:80")
    ///   |---- codecs (File: Widgets and info) TODO
    ///
    // Fixed handles for special device files
    const DEV_HND_ROOT:             usize = 1;
    const DEV_HND_INPUT:            usize = 2;  
    const DEV_HND_OUTPUT:           usize = 3;
    const DEV_HND_OUTPUT_FMT:       usize = 4;
    const DEV_HND_OUTPUT_MUTE:      usize = 5;
    const DEV_HND_OUTPUT_VOL:       usize = 6;
    const DEV_HND_CODECS:           usize = 7;
    const DEV_HND_PCM_FILE_BASE:    usize = 1000;
    fn fops_handler(op: FileOperation) -> IOCompletion {
        match op {
            FileOperation::Open { path } => {
                let mpath = MountPoint::device_relative_path(path);
                if mpath.eq("/") {
                    return IOCompletion::Successful(Self::DEV_HND_ROOT);
                } else if mpath.eq("/output") || mpath.eq("/output/") {
                    return IOCompletion::Successful(Self::DEV_HND_OUTPUT);
                } else if mpath.eq("/output/fmt") {
                    return IOCompletion::Successful(Self::DEV_HND_OUTPUT_FMT);
                } else if mpath.eq("/output/mute") {
                    return IOCompletion::Successful(Self::DEV_HND_OUTPUT_MUTE);
                } else if mpath.eq("/output/pcm") {
                    let mut hda = HDA_DEVICE.lock();
                    let mut file_list = PCM_FILES.lock();
                    let hnd = hda.next_pcm_hnd;
                    let pcm_fobj = PcmFileObject {
                        dev_handle: hnd,
                        output:     true, 
                        pid:        Task::current_pid(),
                    };
                    file_list.insert(hnd, Arc::new(pcm_fobj));
                    hda.next_pcm_hnd += 1;
                    return IOCompletion::Successful(hnd);
                } else if mpath.eq("/output/vol") {
                    return IOCompletion::Successful(Self::DEV_HND_OUTPUT_VOL);
                } else if mpath.eq("/codecs") {
                    return IOCompletion::Successful(Self::DEV_HND_CODECS);
                } else {
                    return IOCompletion::InvalidPath;
                }
                
            },
            FileOperation::Close { hnd } => {
                if hnd <= Self::DEV_HND_CODECS {
                    return IOCompletion::Successful(0);
                } else {
                    return Self::fclose(hnd);
                }
            },
            FileOperation::Read { hnd, off, buff } => {
                return Self::fread(hnd, off, buff);
            },
            FileOperation::Write { hnd, off, buff } => {
                return Self::fwrite(hnd, off, buff);
            },
            FileOperation::Enum { hnd, out } => {
                return Self::fenum(hnd, out);
            },
             _ => {
                return IOCompletion::InvalidOp;
             }
        }
    }

    fn fenum(hnd: usize, out: &mut Vec<DirectoryEntry>) -> IOCompletion {
        const DIR_FLAGS :   usize = DirectoryEntry::FLG_DIRECTORY |
                                    DirectoryEntry::FLG_SYSTEM |
                                    DirectoryEntry::FLG_PERM_READ |
                                    DirectoryEntry::FLG_DEVICE;
        const ROFILE_FLAGS : usize = DirectoryEntry::FLG_SYSTEM |
                                    DirectoryEntry::FLG_PERM_READ |
                                    DirectoryEntry::FLG_DEVICE;
        const WOFILE_FLAGS : usize = DirectoryEntry::FLG_SYSTEM |
                                    DirectoryEntry::FLG_PERM_WRITE |
                                    DirectoryEntry::FLG_DEVICE;
        const RWFILE_FLAGS : usize = DirectoryEntry::FLG_SYSTEM |
                                    DirectoryEntry::FLG_PERM_READ |
                                    DirectoryEntry::FLG_PERM_WRITE |
                                    DirectoryEntry::FLG_DEVICE;
                                
        if hnd == Self::DEV_HND_ROOT {
            out.push(DirectoryEntry {
                name: String::from("codecs"), size: 0, flags: ROFILE_FLAGS
            });
            out.push(DirectoryEntry {
                name: String::from("input"), size: 0, flags: DIR_FLAGS
            });
            out.push(DirectoryEntry {
                name: String::from("output"), size: 0, flags: DIR_FLAGS
            });
        } else if hnd == Self::DEV_HND_OUTPUT {
            out.push(DirectoryEntry {
                name: String::from("fmt"), size: 0, flags: RWFILE_FLAGS
            });
            out.push(DirectoryEntry {
                name: String::from("mute"), size: 0, flags: RWFILE_FLAGS
            });
            out.push(DirectoryEntry {
                name: String::from("pcm"), size: 0, flags: WOFILE_FLAGS
            });
            out.push(DirectoryEntry {
                name: String::from("vol"), size: 0, flags: RWFILE_FLAGS
            });
        } else {
            return IOCompletion::InvalidHandle;
        }
        return IOCompletion::Successful(out.len());
    }

    fn fread(hnd: usize, off: usize, buff: &mut [u8]) -> IOCompletion {
        if hnd <= Self::DEV_HND_CODECS {
            // Read from a special file
            let hda = HDA_DEVICE.lock();
            return hda.fread_spec(hnd, off, buff);
        }
        IOCompletion::InvalidHandle
    }

    fn fwrite(hnd: usize, off: usize, buff: &[u8]) -> IOCompletion {
        if hnd <= Self::DEV_HND_CODECS {
            // Write to a special file
            let mut hda = HDA_DEVICE.lock();
            return hda.fwrite_spec(hnd, off, buff);
        } else {
            // Write to a PCM file
            let file_list = PCM_FILES.lock();
            if let Some(fobj) = file_list.get(&hnd) {
                if fobj.output {
                    // For simplicity, we only support writing to output PCM files
                    let mut hda = HDA_DEVICE.lock();
                    return IOCompletion::Successful(hda.queue_playback(buff));
                } else {
                    // Cannot write to input PCM files
                    return IOCompletion::InvalidOp;
                }
            }
        }
        IOCompletion::InvalidHandle
    }

    fn fclose(_hnd: usize) -> IOCompletion {
        IOCompletion::InvalidHandle
    }


    fn fread_spec(&self, hnd: usize, off: usize, buff: &mut [u8])
                                                            -> IOCompletion {
        if hnd == Self::DEV_HND_OUTPUT_FMT {
            let str;
            let bytes;
            if self.ostream[0].enabled {
                let ch = self.ostream[0].channels;
                let srate = self.ostream[0].sample_rate;
                let bps = self.ostream[0].bits_per_sample;
                str = format!("{}ch,{}Hz,{}bit", ch, srate, bps);
                bytes = str.as_bytes();
            } else {
                bytes = "No default output configured".as_bytes();
            }
            let len = (bytes.len() - off).min(buff.len());
            buff[..len].copy_from_slice(&bytes[off..off + len]);
            return IOCompletion::Successful(len);
        } else if hnd == Self::DEV_HND_OUTPUT_MUTE {
            let bytes;
            if self.output_mute {
                bytes = "1".as_bytes();
            } else {
                bytes = "0".as_bytes();
            }
            let len = (bytes.len() - off).min(buff.len());
            buff[..len].copy_from_slice(&bytes[off..off + len]);
            return IOCompletion::Successful(len);
        } else if hnd == Self::DEV_HND_OUTPUT_VOL {
            let bytes;
            let l  = (self.output_gain_l as u32 * 100) / Self::MAX_GAIN as u32;
            let r = (self.output_gain_r as u32 * 100) / Self::MAX_GAIN as u32;
            let str = format!("L:{},R:{}", l, r);
            bytes = str.as_bytes();
            let len = (bytes.len() - off).min(buff.len());
            buff[..len].copy_from_slice(&bytes[off..off + len]);
            return IOCompletion::Successful(len);
        }
        IOCompletion::InvalidOp
    }

    fn fwrite_spec(&mut self, hnd: usize, _off: usize, buff: &[u8])
                                                            -> IOCompletion {
        if hnd == Self::DEV_HND_OUTPUT_FMT {
            return IOCompletion::InvalidOp; // TODO
        } else if hnd == Self::DEV_HND_OUTPUT_MUTE {
            let s = core::str::from_utf8(buff).unwrap_or("");
            if s.trim() == "1" {
                self.output_mute = true;
            } else if s.trim() == "0" {
                self.output_mute = false;
            } else {
                return IOCompletion::InvalidOp;
            }
            self.apply_output_gain();
        } else if hnd == Self::DEV_HND_OUTPUT_VOL {
            let s = core::str::from_utf8(buff).unwrap_or("");
            let parts: Vec<&str> = s.trim().split(',').collect();
            if parts.len() != 2 {
                return IOCompletion::InvalidOp;
            }
            let mut l = None;
            let mut r = None;
            for part in parts {
                let kv: Vec<&str> = part.split(':').collect();
                if kv.len() != 2 {
                    return IOCompletion::InvalidOp;
                }
                let key = kv[0].trim();
                let val = kv[1].trim().parse::<u8>();
                if val.is_err() {
                    return IOCompletion::InvalidOp;
                }
                let val = val.unwrap();
                if val > 100 {
                    return IOCompletion::InvalidOp;
                }
                if key == "L" {
                    l = Some((val as u32 * Self::MAX_GAIN as u32) / 100);
                } else if key == "R" {
                    r = Some((val as u32 * Self::MAX_GAIN as u32) / 100);
                } else {
                    return IOCompletion::InvalidOp;
                }
            }
            if let Some(l) = l {
                self.output_gain_l = l as u8;
            }
            if let Some(r) = r {
                self.output_gain_r = r as u8;
            }
            self.apply_output_gain();
        }
        IOCompletion::InvalidOp
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WidgetType {
    AudioOutput     = 0, // Output Converter (DAC or S/PDIF) - Section 7.2.3.1
    AudioInput      = 1, // Input Converter  (ADC or S/PDIF) - Section 7.2.3.2
    AudioMixer      = 2, // Summing Amp
    AudioSelector   = 3, // Multiplexer
    PinComplex      = 4, // External connection for audio    - Section 7.2.3.3
    PowerWidget     = 5,
    VolumeKnob      = 6,
    BeepGenerator   = 7,
    VendorDefined   = 0xF,
    Unknown         = 0xE,
}
struct AudioWidget {
    afg_nid:    u8, // Audio Function Group Node ID this widget belongs to
    nid:        u8, // Node ID
    caps:       u32, // Capabilities from Get Widget Capabilities verb
    pin_caps:   Option<u32>, // 7.3.4.9 Pin Capabilities (if applicable)
    pin_conf:   Option<u32>, // Current configuration if Pin Complex (if applicable)
    conn_list:  Option<Vec<u16>>, // Connection list if applicable
}
impl AudioWidget {
    const PINCAP_HEADPHONE: u32 = 0x8;
    const PINCAP_OUTPUT:    u32 = 0x10;
    const PINCAP_INPUT:     u32 = 0x20;
    
    fn widget_type(&self) -> WidgetType {
        match (self.caps & 0xF00000) >> 20 {
            0 => WidgetType::AudioOutput,
            1 => WidgetType::AudioInput,
            2 => WidgetType::AudioMixer,
            3 => WidgetType::AudioSelector,
            4 => WidgetType::PinComplex,
            5 => WidgetType::PowerWidget,
            6 => WidgetType::VolumeKnob,
            7 => WidgetType::BeepGenerator,
            0xF => WidgetType::VendorDefined,
            _ => WidgetType::Unknown,
        }
    }
    fn channels(&self) -> u8 {
        (((self.caps & 0xE000) >> 12) | (self.caps & 1)) as u8 + 1
    }
    fn in_amp_present(&self) -> bool {
        (self.caps & (1 << 1)) != 0
    }
    fn out_amp_present(&self) -> bool {
        (self.caps & (1 << 2)) != 0
    }
    fn amp_override(&self) -> bool {
        (self.caps & (1 << 3)) != 0
    }
    fn fmt_override(&self) -> bool {
        (self.caps & (1 << 4)) != 0
    }
    fn pin_is_output_capable(&self) -> bool {
        if let Some(pin_caps) = self.pin_caps {
            (pin_caps & Self::PINCAP_OUTPUT) != 0
        } else {
            false
        }
    }
    fn pin_is_headphone_capable(&self) -> bool {
        if let Some(pin_caps) = self.pin_caps {
            (pin_caps & Self::PINCAP_HEADPHONE) != 0
        } else {
            false
        }
    }
    fn pin_is_input_capable(&self) -> bool {
        if let Some(pin_caps) = self.pin_caps {
            (pin_caps & Self::PINCAP_INPUT) != 0
        } else {
            false
        }
    }
    
}
impl Debug for AudioWidget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let wtype = self.widget_type();
        let chans = self.channels();
        if wtype == WidgetType::PinComplex {
            f.debug_struct("PinWidget")
                .field("nid", &self.nid)
                .field("channels", &chans)
                .field("pin_caps", &self.pin_caps)
                .field("conn_list", &self.conn_list)
                .field("pin_conf", &self.pin_conf)
                .finish()
        } else {
            f.debug_struct("AudioWidget")
                .field("nid", &self.nid)
                .field("type", &wtype)
                .field("channels", &chans)
                .field("caps", &self.caps)
                .finish()
        }
    }
}

/// Buffer Descriptor List Entry for Intel HDA streams.
/// Each stream is associated with a Buffer Descriptor List (BDL) that the
/// hardware uses to fetch audio data from memory.
/// There can be up to 256 entries in the BDL, each pointing to a `len` byte
/// buffer in memory.
/// The `flags` field can have bit 0 set to indicate "Interrupt on Completion"
/// (IoC), which tells the hardware to generate an interrupt after processing
/// that buffer.
/// 
/// See section 3.6.2 of the Intel HDA specification for details on the BDL
/// format and usage.
#[repr(C, packed)]
#[derive(Debug)]
struct BufferDescListEntry {
    addr:   u64,
    len:    u32,
    flags:  u32, // Bit 0: IoC
}

///
/// Represents an input/output stream on the Intel HDA controller.
/// Each stream has its own set of registers and a Buffer Descriptor List (BDL)
/// that the hardware uses to fetch/place audio data from/to the memory.
/// 
/// In this implementation, each BDL entry points to a 4KB page of audio data,
/// and the BDL itself is allocated as a 4KB page that can hold up to 256,
/// entries, which is the maximum number of BDL entries the hardware supports.
/// This means that each stream can handle up to 256 * 4KB = 1MB of audio data
/// in its BDL at a time.
/// 
/// The 1MB memory is allocated from the kernel's memory arena upon the
/// initialization of the stream. The IntelHDA driver is responsible for
/// populating the that buffer by copying/mixing various audio sources into it.
/// 
#[derive(Clone, Copy)]
struct HDAStream {
    regs:       MMIORegisterFile,
    number:     u8,     // Stream number used by DAC/ADC widgets (1-based)
    enabled:    bool,   // true if this stream has been initialized and reset
    // Buffer Descriptor List (BDL) information for this stream
    bdl_virt:           usize,  // Base address (virtual no-cache)
    bdl_phys:           usize,  // Base address (physical)
    bdl_entries:        usize,  // Number of entries in the BDL
    pcm_phys:           usize,  // Physical address of the PCM buffer
    pcm_virt:           usize,  // Virtual address of the PCM buffer
    pcm_capacity:       usize,  // Maximum size of the PCM buffer in bytes
    pcm_wp:             usize,  // Index into the PCM buffer where the next
                                // audio samples should be written by the driver
    // Default format: 48KHz, 16-bit, Stereo
    sample_rate:        u32,
    bits_per_sample:    u8,
    channels:           u8,
}
impl HDAStream {
    // Buffer Descriptor List (BDL) constants
    const BDL_ENTRY_SIZE:  usize = 16;
    const BDL_MAX_ENTRIES: usize = PHY_FRAME_SIZE / Self::BDL_ENTRY_SIZE;

    // Stream Descriptor Registers
    const REG_CTL:      usize = 0x00; // Control Register (3 bytes)
    const REG_STS:      usize = 0x03; // Status Register (1 byte)
    const REG_LPIB:     usize = 0x04; // Link Position in Buffer (4 bytes)
    const REG_CBL:      usize = 0x08; // Cyclic Buffer Length (4 bytes)
    const REG_LVI:      usize = 0x0C; // Last Valid Index (1 byte)
    const REG_FIFOS:    usize = 0x10; // FIFO Size (2 byte)
    const REG_FMT:      usize = 0x12; // Format Register (2 bytes)
    const REG_BDLPL:    usize = 0x18; // BDL Base Address Low (4 bytes)
    const REG_BDLPU:    usize = 0x1C; // BDL Base Address High (4 bytes)

    const CTL_SRST: u32 = 1 << 0;   // Stream Reset
    const CTL_RUN:  u32 = 1 << 1;   // Stream Run
    const CTL_IOC:  u32 = 1 << 2;   // Interrupt on Completion

    const fn new() -> Self {
        Self {
            regs:               MMIORegisterFile::new(0, 0x20),
            number:             0,
            enabled:            false,
            bdl_virt:           0,
            bdl_phys:           0,
            bdl_entries:        0,
            pcm_phys:           0,
            pcm_virt:           0,
            pcm_capacity:       0,
            pcm_wp:             0,
            sample_rate:        48000,
            bits_per_sample:    16,
            channels:           2,
        }
    }

    /// Initializes the stream by
    /// - Allocating a 256-entry Buffer Descriptor List (BDL) page if not
    ///   already allocated.
    /// - Programming the BDL base address into the stream's registers.
    /// - Resetting the stream via the stream's control register
    fn init(&mut self, stream_number: u8, mmio_base: usize) -> bool {
        // Allocate a 4KB page for the BDL on the first stream initialization
        if self.bdl_phys == 0 {
            self.number     = stream_number;
            self.regs.base_virt = mmio_base;
            self.bdl_phys   = palloc().expect("out of memory");
            self.bdl_virt   = MMUMapping::dma_from_kernel_phys(self.bdl_phys);
            // Zero out the BDL page
            unsafe {
                ptr::write_bytes(self.bdl_virt as *mut u8, 0, PHY_FRAME_SIZE);
            }
            // Program the BDL base address into the stream's registers
            self.write_bdl_addr(self.bdl_phys); // physical address
            // Set the Last Valid Index (LVI) to 0 for now, will be updated when
            // preparing DMA for audio playback/capture
            self.write_lvi_reg(0);
        }
        dbg!("Stream #{} (mmio_base: {:#X}) initialized: BDL base={:#X}, \
                FIFO size={:#X}\n", self.number, self.mmio_base, self.bdl_base,
                mmio_read16(self.mmio_base, Self::REG_FIFOS));
        // Allocate 1MB of sample buffer and hook it up to the BDL entries
        // 1MB fits 262144 dual-channel samples, i.e., about 5.46 seconds
        // stereo audio at 48KHz
        self.pcm_phys = palloc_continuous(256).expect("out of memory");
        self.pcm_virt = MMUMapping::dma_from_kernel_phys(self.pcm_phys);
        self.pcm_capacity = 256 * PHY_FRAME_SIZE; // 256 entries of 4KB each
        let mut pcm_page_addr = self.pcm_phys;
        // Initialize the BDL entries
        for i in 0..Self::BDL_MAX_ENTRIES {
            let entry_addr = self.bdl_virt + i * Self::BDL_ENTRY_SIZE;
            let entry = entry_addr as *mut BufferDescListEntry;
            let desc = BufferDescListEntry {
                addr: pcm_page_addr as u64,
                len: 0,     // Will be set when samples are inserted
                flags: 0,   // No IoC.
            };
            unsafe { entry.write_volatile(desc); }
            pcm_page_addr += PHY_FRAME_SIZE;
        }

        self.reset()
    }

    /// Resets the stream for a new playback/capture session.
    /// This should be followed by a call to `prepare_dma` to set up the BDL for
    /// the new session.
    /// The stream should be stopped before calling reset
    fn reset(&mut self) -> bool {
        // Bail out if the stream is currently running
        if self.read_ctrl_reg() & Self::CTL_RUN != 0 {
            klog!("Error: Cannot reset stream #{} while it is running\n",
                    self.number);
            return false;
        }
        dbg!("Stream #{} BEFORE setting SRST: CTL: {:X}, STS: {:X}\n",
            self.number, self.read_ctrl_reg(), self.read_status_reg());
        // Reset the stream via the control register (Set SRST)
        // 1) Set the SRST bit and wait for it to be set (or time out)
        self.write_ctrl_reg(Self::CTL_SRST);
        let mut tries = 10000;
        while self.read_ctrl_reg() & Self::CTL_SRST == 0 && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        
        // 2) Clear the SRST bit and wait for it to be cleared
        self.write_ctrl_reg(0);
        tries = 10000;
        while self.read_ctrl_reg() & Self::CTL_SRST != 0 && tries > 0 {
            hint::spin_loop();
            tries -= 1;
        }
        dbg!("Stream #{} after setting SRST: CTL: {:X}, SSTS: {:X}\n",
            self.number, self.read_ctrl_reg(), self.read_status_reg());

        self.enabled = true;
        true
    }

    fn run(&mut self, ioc: bool) {
        // Clear the status register's DESE, FIFOE, BCIS
        self.write_status_reg(0x1C);
        // Set the Run bit in the control register to start the stream
        let mut ctl = self.read_ctrl_reg();
        ctl |= Self::CTL_RUN;
        if ioc {
            ctl |= Self::CTL_IOC;
        }
        // Set the stream number for hardware routing
        let tag: u32 = (self.number as u32) << 20;
        ctl |= tag;

        dbg!("Starting Stream #{}: ctl={:X}, sts={:X}, IoC={}, CBL={}\n",
            self.number, self.read_ctrl_reg(), self.read_status_reg(), ioc,
            self.read_cbl_reg());

        self.write_ctrl_reg(ctl);
        // for i in 0..5 {
        //     klog!("Stream #{} running... Iteration {}, CTL: {:X}, STS: {:X}, LPIB: {}\n",
        //         self.number, i, self.read_ctrl_reg(), self.read_status_reg(),
        //         self.read_lpib_reg());
        //     cpu_busywait(Duration::from_millis(200));
        // }
    }

    fn stop(&mut self) {
        // Clear the Run bit in the control register to stop the stream
        let mut ctl = self.read_ctrl_reg();
        ctl &= !Self::CTL_RUN;
        self.write_ctrl_reg(ctl);
        self.pcm_wp = 0; // Reset the PCM write pointer for the next session
        // Clear any pending interrupt status bits
        self.write_status_reg(0x1C);
        dbg!("Stream #{} stopped: CTL: {:X}, STS: {:X}\n",
            self.number, self.read_ctrl_reg(), self.read_status_reg());
    }

    fn is_running(&self) -> bool {
        (self.read_ctrl_reg() & Self::CTL_RUN) != 0
    }

    /// Returns true if playback/capture can be started on this stream, which
    /// requires the stream to be initialized, not running and have some valid
    /// PCM data in the buffer (pcm_wp > 0)
    fn is_ready(&self) -> bool {
        self.enabled && !self.is_running() && self.pcm_wp > 0
    }

    /// Adds an array of raw PCM samples to the stream's PCM buffer.
    /// The samples are written starting from pcm_wp, and the BDL entries are
    /// updated to reflect the new valid length of audio data in the PCM buffer.
    /// It then updates the Cyclic Buffer Length (CBL) register to indicate the
    /// total length of the PCM buffer (in bytes) and the Last Valid Index (LVI)
    /// register.
    /// 
    /// Returns the number of bytes written to the PCM buffer or 0 if the
    /// stream is not ready (i.e., not initialized or is currently running).
    fn push(&mut self, data: &[u8]) -> usize {
        // Bail out if the stream is currently running.
        if self.read_ctrl_reg() & Self::CTL_RUN != 0 {
            klog!("Error: Cannot push samples to stream #{} while running\n",
                    self.number);
            return 0;
        }
        let sample_size = self.bits_per_sample as usize / 8 * 
                            self.channels as usize;
        // Avoid writing partial samples to the PCM buffer by rounding down the
        // data length to the nearest sample boundary
        let bytes_to_write = (data.len() / sample_size) * sample_size;
        let bytes_written = bytes_to_write.min(self.pcm_capacity - self.pcm_wp);
        if bytes_written == 0 {
            dbg!("Warning: PCM buffer for stream #{} is full, \
                    cannot push more samples\n", self.number);
            return 0;
        }
        let src = data.as_ptr();
        let dst = (self.pcm_virt + self.pcm_wp) as *mut u8;
        unsafe {
            dst.copy_from_nonoverlapping(src, bytes_written);
        }
        // Update the BDL entries to reflect the new valid length of audio data
        // in the PCM buffer. IoC will be disabled for all entries except the
        // last one to avoid
        let first_bdle_index = self.pcm_wp / PHY_FRAME_SIZE;
        let last_bdle_index = (self.pcm_wp + bytes_written - 1) / PHY_FRAME_SIZE;
        for i in first_bdle_index..=last_bdle_index {
            let entry_addr = self.bdl_virt + i * Self::BDL_ENTRY_SIZE;
            let entry = entry_addr as *mut BufferDescListEntry;
            let offset_into_pcm = i * PHY_FRAME_SIZE;
            let valid_bytes_in_entry;
            if offset_into_pcm + PHY_FRAME_SIZE <= self.pcm_wp + bytes_written {
                valid_bytes_in_entry = PHY_FRAME_SIZE;
            } else {
                valid_bytes_in_entry = self.pcm_wp + bytes_written
                                        - offset_into_pcm;
            };
            unsafe {
                let mut desc = entry.read_volatile();
                desc.len = valid_bytes_in_entry as u32;
                // Set IoC for the last entry being written to
                if i == last_bdle_index {
                    desc.flags = 1; // Set IoC bit
                    dbg!("Setting IoC for BDL entry {} (offset {} in PCM \
                          buffer, valid bytes {})\n",
                        i, offset_into_pcm, valid_bytes_in_entry);
                } else {
                    desc.flags = 0;
                }
                entry.write_volatile(desc);
            }
        }
        // Update the Cyclic Buffer Length (CBL) and Last Valid Index (LVI)
        self.write_cbl_reg((self.pcm_wp + bytes_written) as u32);
        // LVI is index of last valid entry
        self.write_lvi_reg(last_bdle_index as u8);
        // Update the pcm_wp index for the next write
        self.pcm_wp += bytes_written;

        dbg!("Pushed {} bytes to stream #{}. PCM WP: {}, LVI: {}, CBL: {}\n",
            bytes_written, self.number, self.pcm_wp, self.read_lvi_reg(),
            self.read_cbl_reg());

        bytes_written
    }

    /// Encode audio format parameters into a hardware-specific format value for
    /// stream configuration.
    /// HDA SDnFMT: u16
    /// [14]    base rate (1 = 44.1k, 0 = 48k)
    /// [13:11] multiplier (mult - 1)
    /// [10:8]  divider (div - 1)
    /// [7:4]   channels - 1
    /// [3:0]   bits per sample code (0=8,1=16,2=20,3=24,4=32)
    ///
    fn audio_format_as_u16(&self) -> u16 {
        let base_44:    u16;
        let mult:       u16;
        let div:        u16;
        let chan:       u16 = (self.channels as u16 - 1) & 0xF; // Max 16
        let bps_code:   u16;
        match self.sample_rate {
            8000  => { base_44 = 0; mult = 1; div = 6; }, // 48k / 6 = 8k
            11025 => { base_44 = 1; mult = 1; div = 4; }, // 44.1k / 4 = 11.025k
            16000 => { base_44 = 0; mult = 1; div = 3; }, // 48k / 3 = 16k
            22050 => { base_44 = 1; mult = 1; div = 2; }, // 44.1k / 2 = 22.05k
            32000 => { base_44 = 0; mult = 1; div = 2; }, // 48k / 2 = 32k
            44100 => { base_44 = 1; mult = 1; div = 1; }, // 44.1k / 1 = 44.1k
            48000 => { base_44 = 0; mult = 1; div = 1; }, // 48k / 1 = 48k
            88200 => { base_44 = 1; mult = 2; div = 1; }, // 44.1k * 2 = 88.2k
            96000 => { base_44 = 0; mult = 2; div = 1; }, // 48k * 2 = 96k
            176400=> { base_44 = 1; mult = 4; div = 1; }, // 44.1k * 4 = 176.4k
            192000=> { base_44 = 0; mult = 4; div = 1; }, // 48k * 4 = 192k
            _ => {
                klog!("Unsupported sample rate: {}\n", self.sample_rate);
                return 0;
            }
        }
        bps_code = match self.bits_per_sample {
            8  => 0,
            16 => 1,
            20 => 2,
            24 => 3,
            32 => 4,
            _  => 1, // Default to 16-bit if unsupported 
        };
        (base_44 << 14) | ((mult - 1) << 11) | ((div - 1) << 8) | (chan << 4) | 
            bps_code
    }

    fn set_audio_format(&mut self, ch: u8, srate: u32, bps: u8) -> bool {
        if !self.enabled {
            return false;
        }
        self.channels = ch;
        self.sample_rate = srate;
        self.bits_per_sample = bps;
        let fmt = self.audio_format_as_u16();
        self.write_format_reg(fmt);
        true
    }

    /// Helper functions
    fn write_ctrl_reg(&mut self, value: u32) {
        // This register is 3-bytes wide, so we need to mask the value to avoid
        // writing to the status register which is the byte after this register
        let lsb = (value & 0xFF) as u8;
        let upper16 = ((value & 0xFFFF00) >> 8) as u16;
        self.regs.write(Self::REG_CTL + 1, upper16);
        self.regs.write(Self::REG_CTL, lsb);
    }

    fn read_ctrl_reg(&self) -> u32 {
        self.regs.read::<u32>(Self::REG_CTL) & 0xFFFFFF // Mask to 3 bytes
    }

    fn read_status_reg(&self) -> u8 {
        self.regs.read::<u8>(Self::REG_STS)
    }

    fn write_status_reg(&mut self, value: u8) {
        self.regs.write(Self::REG_STS, value);
    }

    // The Link Position in Buffer (LPIB) register indicates the number of bytes
    // that have been received off the link.
    fn read_lpib_reg(&self) -> u32 {
        self.regs.read::<u32>(Self::REG_LPIB)
    }

    fn write_lvi_reg(&mut self, value: u8) {
        self.regs.write(Self::REG_LVI, value);
    }

    fn read_lvi_reg(&self) -> u8 {
        self.regs.read::<u8>(Self::REG_LVI)
    }

    // The Cyclic Buffer Length indicates the number of bytes in the complete
    // cyclic buffer. Link Position in Buffer (LPIB) will be reset when it
    // reaches this value.
    // Software may only write to this register after Global Reset, Controller
    // Reset, or Stream Reset has occurred. Once the RUN bit has been set to
    // enable the engine, software must not write to this register until after
    // the next reset is asserted, or undefined events will occur.
    fn write_cbl_reg(&mut self, value: u32) {
        self.regs.write(Self::REG_CBL, value);
    }

    fn read_cbl_reg(&self) -> u32 {
        self.regs.read::<u32>(Self::REG_CBL)
    }

    fn write_format_reg(&mut self, value: u16) {
        self.regs.write(Self::REG_FMT, value);
    }

    fn write_bdl_addr(&mut self, bdl_dma_base: usize) {
        let addr_lower = (bdl_dma_base & 0xFFFF_FFFF) as u32;
        let addr_upper = (bdl_dma_base >> 32) as u32;
        self.regs.write(Self::REG_BDLPL, addr_lower);
        self.regs.write(Self::REG_BDLPU, addr_upper);
    }

}

struct PcmFileObject {
    dev_handle: usize,
    output:     bool,   // true for output audio stream, false for input
    pid:        usize,
}

pub fn self_test(freq: u32, seconds: f32) {
    // Test tone: Square wave @ 48KHz, 16-bit, Stereo for 0.5 seconds
    const SAMPLE_RATE: u32 = 48000;
    const BYTES_PER_SAMPLE: usize = 2; // 16-bit
    const CHANNELS: usize = 2; // Stereo
    let num_samples = (SAMPLE_RATE as f32 * seconds) as usize;
    let buffer_size: usize = num_samples * CHANNELS * BYTES_PER_SAMPLE;

    let num_pages = div_round_up!(buffer_size, PHY_FRAME_SIZE);
    let pcm_phys = palloc_continuous(num_pages).expect("out of memory");
    let pcm_virt = MMUMapping::dma_from_kernel_phys(pcm_phys);

    // Generate square wave samples with amplitude of 5000 and frequency
    // determined by the input parameter
    let period_samples = (SAMPLE_RATE / freq) as usize;
    for i in 0..num_samples {
        let val;
        if i % period_samples < period_samples / 2 {
            val = 5000i16;
        } else {
            val = -5000i16;
        };
        let offset = i * BYTES_PER_SAMPLE * CHANNELS;
        // Write same sample to both left and right channels
        let l_sample = (pcm_virt + offset) as *mut i16;
        let r_sample = (pcm_virt + offset + BYTES_PER_SAMPLE) as *mut i16;
        unsafe {
            l_sample.write_volatile(val);
            r_sample.write_volatile(val);
        }
    }

    // Prepare the DMA buffer list and start playback
    let pcm_data = unsafe {
        core::slice::from_raw_parts(pcm_virt as *const u8, buffer_size)
    };

    {
        let mut hda = HDA_DEVICE.lock();
        hda.queue_playback(pcm_data);
    }
        
    // free the buffer
    pfree_continuous(pcm_phys, num_pages);
}


