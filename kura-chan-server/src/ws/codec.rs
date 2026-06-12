use bytes::{Buf, BufMut, BytesMut};

pub const AUDIO_INPUT: u8 = 0x01;
pub const AUDIO_OUTPUT: u8 = 0x02;

pub const FLAG_START: u8 = 0x01;
pub const FLAG_END: u8 = 0x02;
pub const FLAG_INTERRUPT: u8 = 0x04;

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub frame_type: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl AudioFrame {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        let mut buf = &data[..];
        let frame_type = buf.get_u8();
        let flags = buf.get_u8();
        let payload_len = buf.get_u16() as usize;
        if buf.len() < payload_len {
            return None;
        }
        Some(Self {
            frame_type,
            flags,
            payload: buf[..payload_len].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(4 + self.payload.len());
        buf.put_u8(self.frame_type);
        buf.put_u8(self.flags);
        buf.put_u16(self.payload.len() as u16);
        buf.put_slice(&self.payload);
        buf.to_vec()
    }
}
