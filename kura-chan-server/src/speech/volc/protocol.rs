//! Volcengine (豆包语音) custom binary WebSocket framing.
//!
//! Every WS binary frame is: 4-byte header + (optional 4-byte sequence) +
//! 4-byte big-endian payload size + payload. Integers are big-endian.

// Message types (high nibble of byte 1)
pub const MSG_FULL_CLIENT_REQUEST: u8 = 0b0001; // client: params JSON
pub const MSG_AUDIO_ONLY_REQUEST: u8 = 0b0010; // client: audio bytes
pub const MSG_FULL_SERVER_RESPONSE: u8 = 0b1001; // server: result JSON
pub const MSG_SERVER_ERROR: u8 = 0b1111; // server: error

// Message-type-specific flags (low nibble of byte 1)
pub const FLAG_NONE: u8 = 0b0000; // no sequence in header
pub const FLAG_POS_SEQ: u8 = 0b0001; // 4-byte positive sequence follows header
pub const FLAG_LAST_NO_SEQ: u8 = 0b0010; // last (negative) packet, no sequence
pub const FLAG_LAST_NEG_SEQ: u8 = 0b0011; // last packet, negative sequence follows

// Serialization (high nibble of byte 2)
pub const SER_RAW: u8 = 0b0000;
pub const SER_JSON: u8 = 0b0001;

// Compression (low nibble of byte 2)
pub const COMP_NONE: u8 = 0b0000;
pub const COMP_GZIP: u8 = 0b0001;

const PROTOCOL_VERSION: u8 = 0b0001;
const HEADER_SIZE_WORDS: u8 = 0b0001; // 1 * 4 = 4 bytes

/// Build a message frame (no sequence field; uncompressed).
pub fn build(msg_type: u8, flags: u8, serialization: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push((PROTOCOL_VERSION << 4) | HEADER_SIZE_WORDS);
    out.push((msg_type << 4) | flags);
    out.push((serialization << 4) | COMP_NONE);
    out.push(0); // reserved
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug)]
pub struct ServerFrame {
    pub msg_type: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
    /// Present for error frames (MSG_SERVER_ERROR): the 4-byte error code.
    pub error_code: Option<u32>,
}

impl ServerFrame {
    pub fn is_last(&self) -> bool {
        self.flags == FLAG_LAST_NO_SEQ || self.flags == FLAG_LAST_NEG_SEQ
    }
}

/// Parse a server frame. Returns None on malformed input.
pub fn parse(data: &[u8]) -> Option<ServerFrame> {
    if data.len() < 4 {
        return None;
    }
    let header_size = ((data[0] & 0x0f) as usize) * 4;
    let msg_type = data[1] >> 4;
    let flags = data[1] & 0x0f;
    if data.len() < header_size {
        return None;
    }
    let mut idx = header_size;

    let mut error_code = None;
    if msg_type == MSG_SERVER_ERROR {
        // error frame: 4-byte code, then 4-byte size, then message payload
        if data.len() < idx + 4 {
            return None;
        }
        error_code = Some(u32::from_be_bytes([
            data[idx], data[idx + 1], data[idx + 2], data[idx + 3],
        ]));
        idx += 4;
    } else if flags == FLAG_POS_SEQ || flags == FLAG_LAST_NEG_SEQ {
        // sequence number present
        idx += 4;
    }

    if data.len() < idx + 4 {
        return None;
    }
    let size = u32::from_be_bytes([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]) as usize;
    idx += 4;
    let end = (idx + size).min(data.len());
    let payload = data[idx..end].to_vec();

    Some(ServerFrame {
        msg_type,
        flags,
        payload,
        error_code,
    })
}


// === TTS V3 event protocol ===
// Downlink frames carry an event number (flags == FLAG_EVENT) followed by a
// length-prefixed session_id, then the length-prefixed payload.
pub const FLAG_EVENT: u8 = 0b0100;

pub const EV_SESSION_FINISHED: u32 = 152;
pub const EV_TTS_SENTENCE_START: u32 = 350;
pub const EV_TTS_SENTENCE_END: u32 = 351;
pub const EV_TTS_RESPONSE: u32 = 352; // carries audio bytes

#[derive(Debug)]
pub struct V3Frame {
    pub msg_type: u8,
    pub event: Option<u32>,
    pub payload: Vec<u8>,
    pub error_code: Option<u32>,
}

/// Parse a TTS V3 downlink frame:
/// header(4) [+ event(4) + sid_len(4) + sid(N)] + payload_len(4) + payload
pub fn parse_v3(data: &[u8]) -> Option<V3Frame> {
    if data.len() < 4 {
        return None;
    }
    let header_size = ((data[0] & 0x0f) as usize) * 4;
    let msg_type = data[1] >> 4;
    let flags = data[1] & 0x0f;
    if data.len() < header_size {
        return None;
    }
    let mut idx = header_size;
    let mut event = None;
    let mut error_code = None;

    let rd_u32 = |d: &[u8], i: usize| -> Option<u32> {
        if d.len() < i + 4 {
            None
        } else {
            Some(u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]))
        }
    };

    if msg_type == MSG_SERVER_ERROR {
        error_code = Some(rd_u32(data, idx)?);
        idx += 4;
    } else if flags == FLAG_EVENT {
        event = Some(rd_u32(data, idx)?);
        idx += 4;
        let sid_len = rd_u32(data, idx)? as usize;
        idx += 4 + sid_len;
    }

    let plen = rd_u32(data, idx)? as usize;
    idx += 4;
    let end = (idx + plen).min(data.len());
    Some(V3Frame {
        msg_type,
        event,
        payload: data[idx..end].to_vec(),
        error_code,
    })
}
