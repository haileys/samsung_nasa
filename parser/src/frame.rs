use core::num::NonZeroUsize;

use thiserror::Error;

/// Streaming frame parser
#[derive(Default)]
pub struct FrameParser {
    state: State,
    buffer: FrameData,
}

pub const MAX_FRAME_SIZE: usize = 1024;
pub type FrameData = heapless::Vec<u8, MAX_FRAME_SIZE>;

const FRAME_START: u8 = 0x32;
const FRAME_END: u8 = 0x34;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame too short: {size} bytes")]
    FrameTooShort { size: u16 },
    #[error("frame too long: {size} bytes exceeds max {MAX_FRAME_SIZE}")]
    FrameTooLong { size: u16 },
    #[error("bad CRC: received {received:04x?}, expected {expected:04x?}")]
    BadCrc { received: u16, expected: u16 },
    #[error("bad frame end marker: received {received:x?}, expected {FRAME_END}")]
    BadFrameEnd { received: u8 },
}

#[derive(Default)]
enum State {
    #[default] Start,
    SizeHi,
    SizeLo { size_hi: u8 },
    Data { remain: NonZeroUsize },
    CrcHi,
    CrcLo { crc_hi: u8 },
    End,
}

impl FrameParser {
    /// Alias for `FrameParser::default`
    pub fn new() -> Self {
        FrameParser::default()
    }

    pub fn feed(&mut self, byte: u8) -> Result<Option<&FrameData>, FrameError> {
        let state = core::mem::take(&mut self.state);
        match feed_byte(state, &mut self.buffer, byte) {
            Transition::Next(state) => {
                self.state = state;
                Ok(None)
            }
            Transition::Complete => {
                self.state = State::Start;
                Ok(Some(&self.buffer))
            }
            Transition::Error(err) => {
                self.state = State::Start;
                Err(err)
            }
        }
    }
}

enum Transition {
    Next(State),
    Complete,
    Error(FrameError),
}

fn feed_byte(state: State, buffer: &mut FrameData, byte: u8) -> Transition {
    use Transition::{Next, Complete, Error};

    match state {
        State::Start if byte == FRAME_START => Next(State::SizeHi),
        State::Start => Next(State::Start),
        State::SizeHi => Next(State::SizeLo { size_hi: byte }),
        State::SizeLo { size_hi } => {
            let size = u16::from_be_bytes([size_hi, byte]);

            // frame size on the wire includes the 4 bytes for the CRC at
            // the end and this size. subtract and return error on underflow,
            // leaving number of bytes expected for data
            let Some(remain) = usize::from(size).checked_sub(4) else {
                return Error(FrameError::FrameTooShort { size });
            };

            // validate that data length is not greater than size of buffer
            if remain > MAX_FRAME_SIZE {
                return Error(FrameError::FrameTooLong { size });
            }

            buffer.clear();
            Next(next_data_state(remain))
        }
        State::Data { remain } => {
            buffer.push(byte).expect("no capacity left in buffer, this should never happen");

            Next(next_data_state(remain.get() - 1))
        }
        State::CrcHi => Next(State::CrcLo { crc_hi: byte }),
        State::CrcLo { crc_hi } => {
            // validate crc:
            let received = u16::from_be_bytes([crc_hi, byte]);
            let expected = crc16(&buffer);
            if received != expected {
                return Error(FrameError::BadCrc { received, expected });
            }
            Next(State::End)
        }
        State::End if byte == FRAME_END => Complete,
        State::End => Error(FrameError::BadFrameEnd { received: byte }),
    }
}

fn next_data_state(remain: usize) -> State {
    match NonZeroUsize::new(remain) {
        Some(remain) => State::Data { remain },
        None => State::CrcHi,
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;

    for byte in data {
        crc = crc ^ (u16::from(*byte) << 8);

        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }

    crc
}
