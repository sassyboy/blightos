//
// BlightOS kernel
//
// embedded MultiMedia Card (eMMC) Interface
//
//
#![allow(dead_code)]
use core::time::Duration;
use crate::arch::*;
use crate::drivers::storage::*;
use crate::util::*;

#[cfg(feature="debug_emmc")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[eMMC] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_emmc"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

pub static SDHC: Spinlock<BCM2835SDHost> = Spinlock::new(BCM2835SDHost::new());

pub struct BCM2835SDHost {
    mmio_base:      usize,
    gpio_base:      usize,
    sd_hv:          u32, // HOST_SPEC_V[1..3]
    sd_scr:         [u32; 2],
    sd_rca:         u32,
    sd_err:         u32,
}

impl BCM2835SDHost {
    // Registers
    const REG_ARG2:             usize = 0x00;
    const REG_BLKSIZECNT:       usize = 0x04;
    const REG_ARG1:             usize = 0x08;
    const REG_CMDTM:            usize = 0x0C;
    const REG_RESP0:            usize = 0x10;
    const REG_RESP1:            usize = 0x14;
    const REG_RESP2:            usize = 0x18;
    const REG_RESP3:            usize = 0x1C;
    const REG_DATA:             usize = 0x20;
    const REG_STATUS:           usize = 0x24;
    const REG_CONTROL0:         usize = 0x28;
    const REG_CONTROL1:         usize = 0x2C;
    const REG_INTERRUPT:        usize = 0x30;
    const REG_INT_MASK:         usize = 0x34;
    const REG_INT_EN:           usize = 0x38;
    const REG_CONTROL2:         usize = 0x3C;
    const REG_SLOTISR_VER:      usize = 0xFC;

    // Commands
    const CMD_GO_IDLE:          u32 = 0x00000000;
    const CMD_ALL_SEND_CID:     u32 = 0x02010000;
    const CMD_SEND_REL_ADDR:    u32 = 0x03020000;
    const CMD_CARD_SELECT:      u32 = 0x07030000;
    const CMD_SEND_IF_COND:     u32 = 0x08020000;
    const CMD_STOP_TRANS:       u32 = 0x0C030000;
    const CMD_READ_SINGLE:      u32 = 0x11220010;
    const CMD_READ_MULTI:       u32 = 0x12220032;
    const CMD_SET_BLOCKCNT:     u32 = 0x17020000;
    const CMD_APP_CMD:          u32 = 0x37000000;
    const CMD_SET_BUS_WIDTH:    u32 = 0x06020000 | Self::CMD_NEED_APP;
    const CMD_SEND_OP_COND:     u32 = 0x29020000 | Self::CMD_NEED_APP;
    const CMD_SEND_SCR:         u32 = 0x33220010 | Self::CMD_NEED_APP;

    // Command flags
    const CMD_NEED_APP:         u32 = 0x80000000;
    const CMD_RSPNS_48:         u32 = 0x00020000;
    const CMD_ERRORS_MASK:      u32 = 0xfff9c004;
    const CMD_RCA_MASK:         u32 = 0xffff0000;

    // Status register
    const SR_READ_AVAILABLE:    u32 = 0x00000800;
    const SR_DAT_INHIBIT:       u32 = 0x00000002;
    const SR_CMD_INHIBIT:       u32 = 0x00000001;
    const SR_APP_CMD:           u32 = 0x00000020;

    // Interrupt register
    const INT_DATA_TIMEOUT:     u32 = 0x00100000;
    const INT_CMD_TIMEOUT:      u32 = 0x00010000;
    const INT_READ_RDY:         u32 = 0x00000020;
    const INT_CMD_DONE:         u32 = 0x00000001;
    const INT_ERROR_MASK:       u32 = 0x017E8000;

    // Control register
    const C0_SPI_MODE_EN:       u32 = 0x00100000;
    const C0_HCTL_HS_EN:        u32 = 0x00000004;
    const C0_HCTL_DWITDH:       u32 = 0x00000002;

    const C1_SRST_DATA:         u32 = 0x04000000;
    const C1_SRST_CMD:          u32 = 0x02000000;
    const C1_SRST_HC:           u32 = 0x01000000;
    const C1_TOUNIT_DIS:        u32 = 0x000f0000;
    const C1_TOUNIT_MAX:        u32 = 0x000e0000;
    const C1_CLK_GENSEL:        u32 = 0x00000020;
    const C1_CLK_EN:            u32 = 0x00000004;
    const C1_CLK_STABLE:        u32 = 0x00000002;
    const C1_CLK_INTLEN:        u32 = 0x00000001;

    // SLOTISR_VER values
    const HOST_SPEC_NUM:        u32 = 0x00ff0000;
    const HOST_SPEC_NUM_SHIFT:  u32 = 16;
    const HOST_SPEC_V3:         u32 = 2;
    const HOST_SPEC_V2:         u32 = 1;
    const HOST_SPEC_V1:         u32 = 0;

    // SCR flags
    const SCR_SD_BUS_WIDTH_4:   u32 = 0x00000400;
    const SCR_SUPP_SET_BLKCNT:  u32 = 0x02000000;
    // added by my driver
    const SCR_SUPP_CCS:         u32 = 0x00000001;

    const ACMD41_VOLTAGE:       u32 = 0x00ff8000;
    const ACMD41_CMD_COMPLETE:  u32 = 0x80000000;
    const ACMD41_CMD_CCS:       u32 = 0x40000000;
    const ACMD41_ARG_HC:        u32 = 0x51ff8000;

    // GPIO Controler Registers
    const GPIO_FSEL4:           usize = 0x10;
    const GPIO_FSEL5:           usize = 0x14;
    const GPIO_HEN0:            usize = 0x64;
    const GPIO_HEN1:            usize = 0x68;
    const GPIO_PUD:             usize = 0x94;
    const GPIO_PUDCLK0:         usize = 0x98;
    const GPIO_PUDCLK1:         usize = 0x9C;

    const SD_OK:                u32 = 0;
    const SD_ERROR:             u32 = 1;
    const SD_TIMEOUT:           u32 = 2;

    pub const fn new() -> Self {
        Self {
            mmio_base:  0,
            gpio_base:  0,
            sd_hv:      0,
            sd_scr:     [0; 2],
            sd_rca:     0,
            sd_err:     0
        }
    }

    pub fn enumerate() -> usize {
        // TODO - walk the device tree and find a device like:
        // path: "mmc@7e300000"
        // pname:compatible val:brcm,bcm2835-mmcbrcm,bcm2835-sdhci 
        //
        SDHC.lock().init(0x3F300000, 0x3F200000);
        1
    }

    pub fn post_enum() {
        
    }

    pub fn release( _device: usize) {
    }

    ///
    pub fn init(&mut self, mmio_addr: usize, gpio_base: usize) {
        self.mmio_base = mmio_addr;
        self.gpio_base = gpio_base;

        // Set up the GPIO - TODO read pinctrl-names from device tree
        // GPIO_CD
        self.gpio_write(Self::GPIO_FSEL4, 
                        self.gpio_read(Self::GPIO_FSEL4) & !(7<<(7*3) as u32));
        self.gpio_write(Self::GPIO_PUD, 2);
        cpu_busywait(Duration::from_millis(15));
        self.gpio_write(Self::GPIO_PUDCLK1, 1<<15);
        cpu_busywait(Duration::from_millis(15));
        self.gpio_write(Self::GPIO_PUD, 0);
        self.gpio_write(Self::GPIO_PUDCLK1, 0);
        self.gpio_write(Self::GPIO_HEN1,
                        self.gpio_read(Self::GPIO_HEN1) | (1 << 15 as u32));
        // GPIO_CLK, GPIO_CMD
        self.gpio_write(Self::GPIO_FSEL4, 
                        self.gpio_read(Self::GPIO_FSEL4) |
                            (7<<(8*3) as u32) | (7<<(9*3) as u32));
        self.gpio_write(Self::GPIO_PUD, 2);
        cpu_busywait(Duration::from_millis(15));
        self.gpio_write(Self::GPIO_PUDCLK1, (1<<16) | (1<<17));
        self.gpio_write(Self::GPIO_PUD, 0);
        self.gpio_write(Self::GPIO_PUDCLK1, 0);
        // GPIO_DAT0, GPIO_DAT1, GPIO_DAT2, GPIO_DAT3
        self.gpio_write(Self::GPIO_FSEL5, 
                        self.gpio_read(Self::GPIO_FSEL5) |
                            (7<<(0*3) as u32) | (7<<(1*3) as u32) |
                            (7<<(2*3) as u32) | (7<<(3*3) as u32) );
        self.gpio_write(Self::GPIO_PUD, 2);
        crate::arch::cpu_busywait(Duration::from_millis(15));
        self.gpio_write(Self::GPIO_PUDCLK1,
                            (1<<18) | (1<<19) | (1<<20) | (1<<21));
        self.gpio_write(Self::GPIO_PUD, 0);
        self.gpio_write(Self::GPIO_PUDCLK1, 0);
        ////////
        self.sd_hv = (self.read_reg(Self::REG_SLOTISR_VER) & 
                    Self::HOST_SPEC_NUM) >> Self::HOST_SPEC_NUM_SHIFT;
        dbg!("eMMC: GPIO Initialized - SD Version: {:X}\n", self.sd_hv);
        // Reset the card
        self.sd_reset();

        // Register the SD card as a Disk in the storage module
        let disk: Disk = Disk {
            bus: BusType::EMMC,
            bus_id: 0, // TODO support mutiple EMMC Controllers
            drive_id: 0,
            part_id: 0,
            sector_size: 512,
            sector_count: 2097152, // TODO: Get this from the SD card
            issue_io: Self::issue_io
        };
        {
            let mut dlst = DISK_LIST.lock();
            dlst.push(disk);
        }
    }

    fn gpio_read(&self, reg: usize) -> u32 {
        unsafe {
            ((self.gpio_base + reg) as usize as *const u32).read_volatile()
        }
    }
    fn gpio_write(&mut self, reg: usize, val: u32) {
        unsafe {
            ((self.gpio_base + reg) as usize as *mut u32).write_volatile(val);
        }
    }

    fn read_reg(&self, reg: usize) -> u32 {
        unsafe {
            ((self.mmio_base + reg) as usize as *const u32).read_volatile()
        }
    }

    fn write_reg(&mut self, reg: usize, val: u32) {
        unsafe {
            ((self.mmio_base + reg) as usize as *mut u32).write_volatile(val);
        }
    }

    fn sd_set_clk(&mut self, f: u32) -> bool{
        let c = 41666666/f;
        let mut s=32;
        let mut h=0;
    
        // Wait for the inhibit flags to clear
        let inhibit_flags = Self::SR_CMD_INHIBIT | Self::SR_DAT_INHIBIT;
        for _i in 0..100_000 {
            if self.read_reg(Self::REG_STATUS) & inhibit_flags == 0 {
                break;
            }
            cpu_busywait(Duration::from_millis(1));
        }
        if self.read_reg(Self::REG_STATUS) & inhibit_flags != 0 {
            klog!("ERROR: timeout waiting for inhibit flag\n");
            return false;
        }
        // Disable the clock and set the frequency
        self.write_reg(Self::REG_CONTROL1, self.read_reg(Self::REG_CONTROL1) &
                                                            !Self::C1_CLK_EN);
        cpu_busywait(Duration::from_millis(10));
        let mut x = c - 1;
        let mut d;
        if x == 0 {
            s = 0;
        } else {
            if x & 0xffff0000 == 0 { x <<= 16; s -= 16; }
            if x & 0xff000000 == 0 { x <<= 8;  s -= 8; }
            if x & 0xf0000000 == 0 { x <<= 4;  s -= 4; }
            if x & 0xc0000000 == 0 { x <<= 2;  s -= 2; }
            if x & 0x80000000 == 0 { /*x <<= 1;*/  s -= 1; }
            if s > 0 { s -= 1; }
            if s > 7 { s = 7; }
        }
        if self.sd_hv > Self::HOST_SPEC_V2 {
            d = c;
        } else {
            d = 1 << s;
        }
        if d <= 2 {
            d = 2;
            s = 0;
        }
        dbg!("sd_clk divisor: {:X}, shift: {:X}\n", d, s);
        if self.sd_hv > Self::HOST_SPEC_V2{
            h = (d & 0x300) >> 2;
        }
        d = ((d & 0x0ff) << 8) | h;
        self.write_reg(Self::REG_CONTROL1, d |
                            (self.read_reg(Self::REG_CONTROL1) & 0xffff003f));
        cpu_busywait(Duration::from_millis(10));
        // Enable the clock
        self.write_reg(Self::REG_CONTROL1, Self::C1_CLK_EN |
                                            self.read_reg(Self::REG_CONTROL1));
        cpu_busywait(Duration::from_millis(10));
        for _i in 0..10_000 {
            if self.read_reg(Self::REG_CONTROL1) & Self::C1_CLK_STABLE > 0 {
                return true;
            }
            cpu_busywait(Duration::from_millis(10));
        }
        klog!("ERROR: failed to get stable clock\n");
        false
    }

    fn sd_reset(&mut self) {
        self.write_reg(Self::REG_CONTROL0, 0);
        self.write_reg(Self::REG_CONTROL1, Self::C1_SRST_HC |
                                        self.read_reg(Self::REG_CONTROL1));
        for _i in 0..10000 {
            cpu_busywait(Duration::from_millis(10));
            if self.read_reg(Self::REG_CONTROL1) & Self::C1_SRST_HC == 0 {
                break;
            }
        }
        if self.read_reg(Self::REG_CONTROL1) & Self::C1_SRST_HC > 0 {
            klog!("Failed to reset eMMC - ctrl1={:X}\n",
                self.read_reg(Self::REG_CONTROL1));
        }
        self.write_reg(Self::REG_CONTROL1, Self::C1_CLK_INTLEN |
                Self::C1_TOUNIT_MAX |  self.read_reg(Self::REG_CONTROL1));
        cpu_busywait(Duration::from_millis(10));
        // Set the clock frequency
        let r = self.sd_set_clk(400000);
        if r == false {
            klog!("sd_reset failed (freq: 400000)\n");
            return;
        }
        self.write_reg(Self::REG_INT_MASK, 0xffffffff);
        // self.write_reg(Self::REG_INT_EN, 0xffffffff); TODO - Enable IRQ Generation
        
        // CMD_GO_IDLE
        self.sd_cmd(Self::CMD_GO_IDLE,0);
        if self.sd_err != 0 {
            klog!("CMD_GO_IDLE failed. sd_err={}\n", self.sd_err);
            return;
        }
        // CMD_SEND_IF_COND
        self.sd_cmd(Self::CMD_SEND_IF_COND, 0x000001AA);
        if self.sd_err != 0 {
            klog!("CMD_SEND_IF_COND failed. sd_err={}\n", self.sd_err);
            return;
        }
        // CMD_SEND_OP_COND
        let mut timedout = true;
        let mut cmd_ret = 0;
        for _i in 0..6 {
            cpu_busywait(Duration::from_micros(4)); // at least 400 cpu cycles
            cmd_ret = self.sd_cmd(Self::CMD_SEND_OP_COND, Self::ACMD41_ARG_HC);
            dbg!("eMMC CMD_SEND_OP_COND returned: 0x{:X} ", cmd_ret);
            if cmd_ret & Self::ACMD41_CMD_COMPLETE != 0 {
                dbg!("COMPLETE ");
            }
            if cmd_ret & Self::ACMD41_VOLTAGE != 0 {
                dbg!("VOLTAGE ");
            }
            if cmd_ret & Self::ACMD41_CMD_CCS != 0 {
                dbg!("CSS ");
            }
            dbg!("\n");
            if cmd_ret & Self::ACMD41_CMD_COMPLETE != 0 {
                timedout = false;
                break;
            }
        }
        if timedout {
            klog!("sd_reset failed: CMD_SEND_OP_COND Timed out!\n");
            return;
        }
        if cmd_ret & Self::ACMD41_CMD_COMPLETE == 0 ||
            cmd_ret & Self::ACMD41_VOLTAGE == 0 {
            klog!("sd_reset failed: CMD_SEND_OP_COND\n");
            return;
        }
        let mut ccs = 0;
        if cmd_ret & Self::ACMD41_CMD_CCS != 0 {
            ccs = Self::SCR_SUPP_CCS;
        }
        // CMD_ALL_SEND_CID
        self.sd_cmd(Self::CMD_ALL_SEND_CID,0);
        // CMD_SEND_REL_ADDR
        self.sd_rca = self.sd_cmd(Self::CMD_SEND_REL_ADDR, 0);
        dbg!("eMMC: CMD_SEND_REL_ADDR returned 0x{:X}\n", self.sd_rca);
        if self.sd_err != 0 {
            klog!("sd_reset failed: CMD_SEND_REL_ADDR - sd_err={}\n", self.sd_err);
            return;
        }

        // Set the clock frequency to 25_000_000
        let r = self.sd_set_clk(25000000);
        if r == false {
            klog!("sd_reset failed (freq: 25000000)\n");
            return;
        }

        // CMD_CARD_SELECT
        self.sd_cmd(Self::CMD_CARD_SELECT, self.sd_rca);
        if self.sd_err !=0 {
            klog!("sd_reset failed: CMD_CARD_SELECT - sd_err={}\n", self.sd_err);
            return;
        }

        if self.sd_status(Self::SR_DAT_INHIBIT) == false {
            klog!("sd_reset failed: SR_DAT_INHIBIT still set.\n");
            return;
        }

        self.write_reg(Self::REG_BLKSIZECNT, (1<<16) | 8);
        self.sd_cmd(Self::CMD_SEND_SCR, 0);
        if self.sd_err != 0 {
           klog!("sd_reset failed: CMD_SEND_SCR - sd_err={}\n", self.sd_err); 
        }

        if self.sd_int(Self::INT_READ_RDY) == false {
            klog!("sd_reset failed: INT_READ_RDY timed out\n");
            return;
        } 

        let mut r = 0;
        for _i in 0..100_000 {
            if r == 2 {
                break;
            }
            if self.read_reg(Self::REG_STATUS) & Self::SR_READ_AVAILABLE != 0 {
                self.sd_scr[r] = self.read_reg(Self::REG_DATA);
                r += 1;
            } else {
                cpu_busywait(Duration::from_millis(1));
            }
        }
        if r != 2 {
            klog!("sd_reset failed to read scr\n");
            return;
        }

        if self.sd_scr[0] & Self::SCR_SD_BUS_WIDTH_4 != 0 {
            self.sd_cmd(Self::CMD_SET_BUS_WIDTH, self.sd_rca | 2);
            if self.sd_err != 0 {
                klog!("sd_reset failed to set the bus width to 4\n");
                return;
            }
            self.write_reg(Self::REG_CONTROL0, Self::C0_HCTL_DWITDH | 
                                            self.read_reg(Self::REG_CONTROL0));
        }
        
        // add software flag
        dbg!("eMMC: supports ");
        if self.sd_scr[0] & Self::SCR_SUPP_SET_BLKCNT != 0 {
            dbg!("SET_BLKCNT ");
        }
        if ccs != 0 {
            dbg!("CCS ");
        }
        dbg!("\n");
        self.sd_scr[0] &= !Self::SCR_SUPP_CCS;
        self.sd_scr[0] |= ccs;
        dbg!("eMMC sd_reset successful!\n");
    }

    fn sd_cmd(&mut self, cmd_code: u32, arg: u32) -> u32 {
        let mut code = cmd_code;
        if code & Self::CMD_NEED_APP != 0 {
            let mut cmd = Self::CMD_APP_CMD;
            if self.sd_rca != 0 {
                cmd |= Self::CMD_RSPNS_48;
            }
            let r = self.sd_cmd(cmd, self.sd_rca);
            if self.sd_rca != 0 && r == 0 {
                klog!("ERROR: failed to send SD APP command\n");
                self.sd_err= Self::SD_ERROR;
                return 0;
            }
            code &= !Self::CMD_NEED_APP;
        }

        if self.sd_status(Self::SR_CMD_INHIBIT) == false {
            klog!("ERROR: EMMC busy\n");
            self.sd_err= Self::SD_TIMEOUT;
            return 0;
        }
        dbg!("EMMC: Sending command {:X} with arg {:X}\n", code, arg);
        self.write_reg(Self::REG_INTERRUPT, self.read_reg(Self::REG_INTERRUPT));
        self.write_reg(Self::REG_ARG1, arg);
        self.write_reg(Self::REG_CMDTM, code);
        match code {
            Self::CMD_SEND_OP_COND  => {
                cpu_busywait(Duration::from_millis(1000));
            },
            Self::CMD_SEND_IF_COND | Self::CMD_APP_CMD => {
                cpu_busywait(Duration::from_millis(100));
            },
            _ =>{}
        }
        if self.sd_int(Self::INT_CMD_DONE) == false {
            klog!("sd_cmd failed to send EMMC command\n");
            self.sd_err = Self::SD_ERROR;
            return 0;
        }
    
        let mut r = self.read_reg(Self::REG_RESP0);
        if code == Self::CMD_GO_IDLE | Self::CMD_APP_CMD {
            return 0;
        } else if code == Self::CMD_APP_CMD | Self::CMD_RSPNS_48 {
            return r & Self::SR_APP_CMD;
        } else if code == Self::CMD_SEND_OP_COND {
            return r;
        } else if code == Self::CMD_SEND_IF_COND {
            if r == arg {
                return Self::SD_OK;
            } else {
                return Self::SD_ERROR;
            }
        } else if code == Self::CMD_ALL_SEND_CID {
            r |= self.read_reg(Self::REG_RESP3);
            r |= self.read_reg(Self::REG_RESP2);
            r |= self.read_reg(Self::REG_RESP1);
            return r;
        } else if code == Self::CMD_SEND_REL_ADDR {
            self.sd_err = ( (r&0x1fff) | ((r&0x2000)<<6) | ((r&0x4000)<<8) | 
                                ((r&0x8000)<<8) ) & Self::CMD_ERRORS_MASK;
            return r & Self::CMD_RCA_MASK
        }
        r & Self::CMD_ERRORS_MASK
    }
    
    // Wait for data or command ready
    fn sd_status(&self, mask: u32) -> bool {
        for _i in 0..10_000 {
            if self.read_reg(Self::REG_STATUS) & mask != 0 && 
                self.read_reg(Self::REG_INTERRUPT) & Self::INT_ERROR_MASK == 0 {
                cpu_busywait(Duration::from_millis(1));
            } else {
                return true;
            }
        }
        false
    }

    // Wait for interrupt
    fn sd_int(&mut self, mask: u32) -> bool {
        let m = mask | Self::INT_ERROR_MASK;
        let mut hit = false;
        for _i in 0..1000_000 {
            if self.read_reg(Self::REG_INTERRUPT) & m != 0 {
                hit = true;
                break;
            }
            cpu_busywait(Duration::from_millis(1));
        }
        let v = self.read_reg(Self::REG_INTERRUPT);
        if hit == false || v & Self::INT_CMD_TIMEOUT != 0 ||
                            v & Self::INT_DATA_TIMEOUT != 0 {
            klog!("sd_int: Timed out\n");
            self.write_reg(Self::REG_INTERRUPT, v);
            return false; // Timed out
        } else if v & Self::INT_ERROR_MASK != 0 {
            klog!("sd_int: Error detected\n");
            self.write_reg(Self::REG_INTERRUPT, v);
            return false;
        }
        self.write_reg(Self::REG_INTERRUPT, mask);
        true
    }

    pub fn sd_readblock(&mut self, lba: u32, buffer: usize, cnt: u32) -> u32 {
        let num;
        if cnt < 1 {
            num = 1;
        } else {
            num = cnt;
        }
        dbg!("sd_readblock lba {} count {}\n", lba, num);
        if self.sd_status(Self::SR_DAT_INHIBIT) == false {
            self.sd_err= Self::SD_TIMEOUT;
            return 0;
        }
        // unsigned int *buf=(unsigned int *)buffer;
        if self.sd_scr[0] & Self::SCR_SUPP_CCS != 0 {
            if num > 1 && (self.sd_scr[0] & Self::SCR_SUPP_SET_BLKCNT != 0) {
                self.sd_cmd(Self::CMD_SET_BLOCKCNT, num);
                if self.sd_err != 0 {
                    return 0;
                }
            }
            self.write_reg(Self::REG_BLKSIZECNT, (num << 16) | 512);
            if num == 1 {
                self.sd_cmd(Self::CMD_READ_SINGLE, lba);
            } else {
                self.sd_cmd(Self::CMD_READ_MULTI, lba);
            }
            if self.sd_err != 0 {
                return 0;
            } 
        } else {
            self.write_reg(Self::REG_BLKSIZECNT, (1 << 16) | 512);
        }

        let mut blk_cnt=0;
        let mut bufptr = buffer as *mut u32;
        while blk_cnt < num {
            if self.sd_scr[0] & Self::SCR_SUPP_CCS == 0 {
                self.sd_cmd(Self::CMD_READ_SINGLE,(lba + blk_cnt)*512);
                if self.sd_err != 0 {
                    return 0;
                }
            }
            if self.sd_int(Self::INT_READ_RDY) == false {
                klog!("\rERROR: Timeout waiting for ready to read\n");
                return 0;
            }
            for _i in 0..128 {
                unsafe {
                    bufptr.write(self.read_reg(Self::REG_DATA));
                    bufptr = bufptr.add(1);
                }
            }
            blk_cnt += 1;
        }
        
        if (num > 1) && (self.sd_scr[0] & Self::SCR_SUPP_SET_BLKCNT == 0) && 
            (self.sd_scr[0] & Self::SCR_SUPP_CCS) != 0 {
            self.sd_cmd(Self::CMD_STOP_TRANS, 0);
        } 
        if self.sd_err != Self::SD_OK || blk_cnt != num {
            return 0;
        }
        num * 512
    }

    fn issue_io(dsk: &Disk, iorq: &IORequest) {
        // TEST - READ and DUMP LBA2
        // let buf = crate::mem::phys::palloc().unwrap();
        // let ret = self.sd_readblock(2, buf, 1);
        // klog!("Read {} bytes from lba 2:\n", ret);
        // if ret > 0 {
        //     crate::util::dump_memory_columns(buf, 60, 5);
        // }
        // Make a copy of the caller's request for the driver's internal use
        let mut cpyrq = iorq.clone();
        cpyrq.ts_issued = SystemTimer::current_timestamp();
        if dsk.bus != BusType::EMMC || dsk.bus_id != 0 {
            cpyrq.completion = Err(error!(ErrorCode::InvalidBus));
            (iorq.completion_cb)(cpyrq);
            return;
        }
        if iorq.lba + iorq.sectors as u64 > dsk.sector_count {
            cpyrq.completion = Err(error!(ErrorCode::OutOfBoundIO));
            (iorq.completion_cb)(cpyrq);
            return;
            
        }
        if dsk.drive_id as usize >= 1 { // Only 1 SD card per eMMC
            cpyrq.completion = Err(error!(ErrorCode::InvalidDrive));
            (iorq.completion_cb)(cpyrq);
            return;
        }
        // TODO implement non-blocking
        let mut sdhc = SDHC.lock();
        if iorq.op == IOOperation::Read {
            cpyrq.ts_submitted = SystemTimer::current_timestamp();
            let ret = sdhc.sd_readblock(iorq.lba as u32, iorq.buffer, 
                                                    iorq.sectors as u32);
            cpyrq.ts_completed = SystemTimer::current_timestamp();
            if ret > 0 {
                cpyrq.completion = Ok(ret as usize);
            } else {
                cpyrq.completion = Err(error!(ErrorCode::IOError));
            }
        } else {
            // TODO: support write
            cpyrq.completion = Err(error!(ErrorCode::InvalidOp));
        }
        drop(sdhc);
        (cpyrq.completion_cb)(cpyrq);
    }

}