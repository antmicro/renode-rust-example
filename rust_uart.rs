use lazy_static::lazy_static;
use std::sync::Mutex;
use std::io::Cursor;
use std::collections::VecDeque;
use bitstream_io::{LittleEndian, BitReader, BitRead, BitWriter, BitWrite};

#[link(wasm_import_module = "uart")]
extern { // imported from host
    #[link_name = "SetIRQ"]
    fn set_irq_inner(value: i32);
    #[link_name = "InvokeCharReceived"]
    fn invoke_char_received_inner(character: i32);
}

unsafe fn set_irq(value: bool) {
    set_irq_inner(value.into());
}

unsafe fn invoke_char_received(value: u8) {
    invoke_char_received_inner(value.into());
}

static mut TRANSMIT_DATA: u32 = 0;
static mut RECEIVE_DATA: u32 = 0;
static mut TRANSMIT_CONTROL: u32 = 0;
static mut RECEIVE_CONTROL: u32 = 0;
static mut INTERRUPT_ENABLE: u32 = 0;
static mut INTERRUPT_PENDING: u32 = 0;
static mut BAUDRATE_DIVISOR: u32 = 0xFFFF;

static mut TRANSMIT_ENABLE: bool = true; //name: "TXEN"
static mut RECEIVE_ENABLE: bool = true; //name: "RXEN"
static mut NUMBER_OF_STOP_BITS: bool = false; //name: "NSTOP"
static mut BAUD_RATE_DIVISOR: i32 = 0; //name: "DIV"
static mut TRANSMIT_WATERMARK_LEVEL: i32 = 0; //name: "TXCNT"
static mut RECEIVE_WATERMARK_LEVEL: i32 = 0; //name: "RXCNT"
static mut TRANSMIT_WATERMARK_INTERRUPT_PENDING: bool = false; //name: "TXWM"
static mut RECEIVE_WATERMARK_INTERRUPT_PENDING: bool = false; //name: "RXWM"
static mut TRANSMIT_WATERMARK_INTERRUPT_ENABLE: bool = false; //name: "TXWM"
static mut RECEIVE_WATERMARK_INTERRUPT_ENABLE: bool = false; //name: "RXWM"

const TRANSMIT_DATA_OFFSET: i64 = 0x0;
const RECEIVE_DATA_OFFSET: i64 = 0x04;
const TRANSMIT_CONTROL_OFFSET: i64 = 0x08;
const RECEIVE_CONTROL_OFFSET: i64 = 0x0C;
const INTERRUPT_ENABLE_OFFSET: i64 = 0x10;
const INTERRUPT_PENDING_OFFSET: i64 = 0x14;
const BAUDRATE_DIVISOR_OFFSET: i64 = 0x18;

static mut QUEUE_COUNT: i32 = 0;

lazy_static! {  
    static ref QUEUE: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
}

#[export_name = "Reset"] // exported to wasm
pub unsafe extern fn reset() {
    clear_buffer();
    reset_registers();
    update_interrupts();
}

#[export_name = "WriteChar"] // exported to wasm
pub unsafe extern fn write_char(value: u32) {
    {
        let queue = &mut QUEUE.lock().unwrap();
        queue.push_back(value.to_le_bytes()[0]);
        QUEUE_COUNT += 1;
    }
    char_written();
}

#[export_name = "ReadDoubleWord"] // exported to wasm
pub unsafe extern fn read_double_word(offset: i64) -> u32 {
    return match offset {
        TRANSMIT_DATA_OFFSET => {
            TRANSMIT_DATA &= ! (1 << 31); // FULL => false
            return TRANSMIT_DATA;
        },
        RECEIVE_DATA_OFFSET => {
            if QUEUE_COUNT == 0 { //EMPTY
                RECEIVE_DATA |= 1 << 31;
            } else {
                RECEIVE_DATA &= ! (1 << 31);
            }

            let mut bytes: [u8; 4] = RECEIVE_DATA.to_le_bytes();
            let output: &mut [u8] = &mut bytes; 
            {
                let mut writer = BitWriter::endian(output, LittleEndian);
                
                let (success, character) = try_get_character();
                let byts: [u8; 1] = [character];
                if success {
                    writer.write_bytes(&byts).unwrap();
                }
            }

            RECEIVE_DATA = u32::from_le_bytes(bytes);

            return RECEIVE_DATA;
        },
        TRANSMIT_CONTROL_OFFSET => {
            return TRANSMIT_CONTROL;
        },
        RECEIVE_CONTROL_OFFSET => {
            return RECEIVE_CONTROL;
        },
        INTERRUPT_ENABLE_OFFSET => {
            return INTERRUPT_ENABLE;
        },
        INTERRUPT_PENDING_OFFSET => {
            if TRANSMIT_WATERMARK_INTERRUPT_PENDING {
                INTERRUPT_PENDING |= 1<<0;
            } else {
                INTERRUPT_PENDING &= ! 1<<0;
            }

            if RECEIVE_WATERMARK_INTERRUPT_PENDING {
                INTERRUPT_PENDING |= 1<<1;
            } else {
                INTERRUPT_PENDING &= ! 1<<1;
            }

            return INTERRUPT_PENDING;
        },
        BAUDRATE_DIVISOR_OFFSET => {
            return BAUDRATE_DIVISOR;
        },
        _    => 0,
    };
}

#[export_name = "WriteDoubleWord"] // exported to wasm
pub unsafe extern fn write_double_word(offset: i64, value: u32) {
    match offset {
        TRANSMIT_DATA_OFFSET => {
            TRANSMIT_DATA = value;
            let bytes = value.to_le_bytes();
            let mut cursor = Cursor::new(&bytes);
            {
                let mut reader = BitReader::endian(&mut cursor, LittleEndian);
                let character = reader.read(8).unwrap();
                if TRANSMIT_ENABLE {
                    transmit_character(character);
                    update_interrupts();
                }
            }
        },
        RECEIVE_DATA_OFFSET => {
            // write does nothing
            // RECEIVE_DATA = value;
        },
        TRANSMIT_CONTROL_OFFSET => {
            TRANSMIT_CONTROL = value;
            let bytes = value.to_le_bytes();
            let mut cursor = Cursor::new(&bytes);
            {
                let mut reader = BitReader::endian(&mut cursor, LittleEndian);
                TRANSMIT_ENABLE = reader.read_bit().unwrap();
                NUMBER_OF_STOP_BITS = reader.read_bit().unwrap();
                reader.skip(14).unwrap();
                let transmit_watermark_level: i32 = reader.read(3).unwrap();

                if transmit_watermark_level != TRANSMIT_WATERMARK_LEVEL {
                    TRANSMIT_WATERMARK_LEVEL = transmit_watermark_level;
                    update_interrupts();
                }
            }
        },
        RECEIVE_CONTROL_OFFSET => {
            RECEIVE_CONTROL = value;
            let bytes = value.to_le_bytes();
            let mut cursor = Cursor::new(&bytes);
            {
                let mut reader = BitReader::endian(&mut cursor, LittleEndian);
                let receive_enable: bool = reader.read_bit().unwrap();

                if RECEIVE_ENABLE != receive_enable {
                    RECEIVE_ENABLE = receive_enable;
                    if !receive_enable {
                        clear_buffer()
                    }
                }

                reader.skip(15).unwrap();
                let receive_watermark_level: i32 = reader.read(3).unwrap();

                if receive_watermark_level != RECEIVE_WATERMARK_LEVEL {
                    RECEIVE_WATERMARK_LEVEL = receive_watermark_level;
                    update_interrupts();
                }
            }
        },
        INTERRUPT_ENABLE_OFFSET => {
            INTERRUPT_ENABLE = value;
            let bytes = value.to_le_bytes();
            let mut cursor = Cursor::new(&bytes);
            {
                let mut reader = BitReader::endian(&mut cursor, LittleEndian);
                let transmit_watermark_interrupt_enable: bool = reader.read_bit().unwrap();
                if transmit_watermark_interrupt_enable != TRANSMIT_WATERMARK_INTERRUPT_ENABLE {
                    TRANSMIT_WATERMARK_INTERRUPT_ENABLE = transmit_watermark_interrupt_enable;
                    update_interrupts();
                }

                let receive_watermark_interrupt_enable: bool = reader.read_bit().unwrap();
                if receive_watermark_interrupt_enable != RECEIVE_WATERMARK_INTERRUPT_ENABLE {
                    RECEIVE_WATERMARK_INTERRUPT_ENABLE = receive_watermark_interrupt_enable;
                    update_interrupts();
                }
            }
        },
        INTERRUPT_PENDING_OFFSET => {
            // FieldMode.Read "TXWM" - bit 0, "TXWM"  - bit 1
            // Cannot be written to, are set by peripheral itself
            // INTERRUPT_PENDING = value;
        },
        BAUDRATE_DIVISOR_OFFSET => {
            BAUDRATE_DIVISOR = value;
            let bytes = value.to_le_bytes();
            let mut cursor = Cursor::new(&bytes);
            {
                let mut reader = BitReader::endian(&mut cursor, LittleEndian);
                BAUD_RATE_DIVISOR = reader.read(16).unwrap();
            }
        },
        _    => println!("Unhandled write to unimplemented register!"),
    };
}

unsafe fn update_interrupts() {
    TRANSMIT_WATERMARK_INTERRUPT_PENDING = TRANSMIT_WATERMARK_LEVEL > 0;
    RECEIVE_WATERMARK_INTERRUPT_PENDING = QUEUE_COUNT > RECEIVE_WATERMARK_LEVEL;

    set_irq(TRANSMIT_WATERMARK_INTERRUPT_ENABLE && TRANSMIT_WATERMARK_INTERRUPT_PENDING 
    || RECEIVE_WATERMARK_INTERRUPT_ENABLE && RECEIVE_WATERMARK_INTERRUPT_PENDING);
}

unsafe fn char_written() {
    if ! RECEIVE_ENABLE {
        clear_buffer();
    } else {
        update_interrupts();
    }
}

unsafe fn queue_emptied() {
    if RECEIVE_ENABLE {
        update_interrupts();
    }
}

unsafe fn try_get_character() -> (bool, u8) {
    let queue = &mut QUEUE.lock().unwrap();

    if QUEUE_COUNT == 0 {
        return (false, 0x00);
    }

    let character = queue.pop_front().unwrap_or(u8::default());
    QUEUE_COUNT -= 1;
    
    if QUEUE_COUNT == 0 {
        queue_emptied();
    }
    return (true, character);
}

unsafe fn transmit_character(character: u8) {
    invoke_char_received(character);
}

unsafe fn clear_buffer() {
    {
        let queue = &mut QUEUE.lock().unwrap();
        queue.clear();
    }
    queue_emptied();
}

unsafe fn reset_registers() {
        TRANSMIT_DATA = 0;
        RECEIVE_DATA = 0;
        TRANSMIT_CONTROL = 0;
        RECEIVE_CONTROL = 0;
        INTERRUPT_ENABLE = 0;
        INTERRUPT_PENDING = 0;
        BAUDRATE_DIVISOR = 0xFFFF;  
}
