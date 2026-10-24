//! NCS bytecode reader and writer.
//!
//! Layout (V1.0): `"NCS "`, `"V1.0"`, a magic byte `0x42`, then a big-endian
//! u32 file size, then a stream of (opcode, qualifier, operands). Multi-byte
//! operands are big-endian. Jump offsets are relative to the start of their
//! instruction.
//!
//! This is the structured equivalent of DeNCS `Decoder.java`. That class
//! emits a token string for a generated parser; here the same bytes become
//! [`Instruction`] values. Opcode `0x42` in DeNCS is the header magic (its
//! `"T"` command), not a real instruction — the size field is consumed as
//! header, not as bytecode.

use kotor_ncs_isa::Operands;

const HEADER_SIZE: usize = 13;
const MAGIC: u8 = 0x42;

/// One decoded instruction, located by its file offset.
#[derive(Clone, Debug)]
pub struct Instruction {
    pub offset: u32,
    pub op: &'static str,
    pub args: Vec<Arg>,
    /// Set for ACTION: the engine-function routine id.
    pub routine: Option<u16>,
    /// Set for ACTION by the caller when it has a name; the reader leaves it `None`.
    pub routine_name: Option<&'static str>,
    /// Set for ACTION: how many arguments the call consumes.
    pub argc: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Str(String),
    /// Absolute file offset of a jump target.
    Jump(u32),
}

#[derive(Clone, Debug)]
pub struct Ncs {
    pub declared_size: u32,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err(message: impl Into<String>) -> Error {
    Error {
        message: message.into(),
    }
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"NCS ")
}

pub fn read(data: &[u8]) -> Result<Ncs> {
    if data.len() < HEADER_SIZE {
        return Err(err(format!("truncated NCS header ({} bytes)", data.len())));
    }
    if !data.starts_with(b"NCS V1.0") {
        return Err(err("the data file is not an NCS V1.0 file"));
    }
    let magic = data[8];
    if magic != MAGIC {
        return Err(err(format!(
            "invalid NCS header magic: expected 0x{MAGIC:02X}, got 0x{magic:02X}"
        )));
    }
    let declared_size = u32::from_be_bytes(data[9..13].try_into().unwrap());
    if declared_size as usize > data.len() {
        return Err(err(format!(
            "NCS size field ({declared_size}) is larger than the file ({})",
            data.len()
        )));
    }
    if declared_size as usize <= HEADER_SIZE {
        return Ok(Ncs {
            declared_size,
            instructions: Vec::new(),
        });
    }

    let end = (declared_size as usize).min(data.len());
    let mut pos = HEADER_SIZE;
    let mut instructions = Vec::new();
    while pos < end {
        if pos + 2 > end {
            break;
        }
        if data[pos..end].iter().all(|&b| b == 0) {
            break;
        }
        match read_instruction(data, pos, end) {
            Ok((ins, next)) => {
                pos = next;
                instructions.push(ins);
            }
            Err(e) => {
                if data[pos..end].iter().all(|&b| b == 0) {
                    break;
                }
                return Err(e);
            }
        }
    }
    Ok(Ncs {
        declared_size,
        instructions,
    })
}

/// Encode `ncs` as NCS V1.0 bytes. The size field is the encoded length.
pub fn write(ncs: &Ncs) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for ins in &ncs.instructions {
        write_instruction(&mut body, ins)?;
    }
    let declared_size = (HEADER_SIZE + body.len()) as u32;
    let mut out = Vec::with_capacity(declared_size as usize);
    out.extend_from_slice(b"NCS V1.0");
    out.push(MAGIC);
    out.extend_from_slice(&declared_size.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

fn read_instruction(data: &[u8], offset: usize, end: usize) -> Result<(Instruction, usize)> {
    let mut pos = offset;
    let opcode = take_u8(data, &mut pos, end)?;
    let qualifier = take_u8(data, &mut pos, end)?;
    let decoded = kotor_ncs_isa::lookup(opcode, qualifier).ok_or_else(|| {
        err(format!(
            "unknown NCS instruction 0x{opcode:02X}/0x{qualifier:02X} at offset {offset}"
        ))
    })?;
    let op = decoded.mnemonic;
    let kind = decoded.operands;

    let mut ins = Instruction {
        offset: offset as u32,
        op,
        args: Vec::new(),
        routine: None,
        routine_name: None,
        argc: None,
    };

    match kind {
        Operands::None => {}
        Operands::Copy => {
            ins.args
                .push(Arg::Int(take_i32(data, &mut pos, end)? as i64));
            ins.args
                .push(Arg::Int(take_u16(data, &mut pos, end)? as i64));
        }
        Operands::ConstInt => ins
            .args
            .push(Arg::Int(take_i32(data, &mut pos, end)? as i64)),
        Operands::ConstFloat => ins
            .args
            .push(Arg::Float(take_f32(data, &mut pos, end)? as f64)),
        Operands::ConstString => {
            let len = take_u16(data, &mut pos, end)? as usize;
            let bytes = take_bytes(data, &mut pos, end, len)?;
            ins.args
                .push(Arg::Str(String::from_utf8_lossy(bytes).into_owned()));
        }
        Operands::ConstObject => ins
            .args
            .push(Arg::Int(take_i32(data, &mut pos, end)? as i64)),
        Operands::Action => {
            let routine = take_u16(data, &mut pos, end)?;
            let argc = take_u8(data, &mut pos, end)?;
            ins.routine = Some(routine);
            ins.argc = Some(argc);
            ins.args.push(Arg::Int(routine as i64));
            ins.args.push(Arg::Int(argc as i64));
        }
        Operands::Offset => ins
            .args
            .push(Arg::Int(take_i32(data, &mut pos, end)? as i64)),
        Operands::Jump => {
            let rel = take_i32(data, &mut pos, end)?;
            let target = offset as i64 + rel as i64;
            if target < 0 || target as usize > end {
                return Err(err(format!(
                    "jump at {offset} lands outside the file (rel {rel}, target {target})"
                )));
            }
            ins.args.push(Arg::Jump(target as u32));
        }
        Operands::Destruct => {
            ins.args
                .push(Arg::Int(take_u16(data, &mut pos, end)? as i64));
            ins.args
                .push(Arg::Int(take_i16(data, &mut pos, end)? as i64));
            ins.args
                .push(Arg::Int(take_u16(data, &mut pos, end)? as i64));
        }
        Operands::Increment => ins
            .args
            .push(Arg::Int(take_i32(data, &mut pos, end)? as i64)),
        Operands::StoreState => {
            ins.args
                .push(Arg::Int(take_u32(data, &mut pos, end)? as i64));
            ins.args
                .push(Arg::Int(take_u32(data, &mut pos, end)? as i64));
        }
        Operands::StructCompare => ins
            .args
            .push(Arg::Int(take_u16(data, &mut pos, end)? as i64)),
    }
    Ok((ins, pos))
}

fn write_instruction(out: &mut Vec<u8>, ins: &Instruction) -> Result<()> {
    let spec = kotor_ncs_isa::lookup_mnemonic(ins.op)
        .ok_or_else(|| err(format!("unknown mnemonic {}", ins.op)))?;
    out.push(spec.opcode);
    out.push(spec.qualifier);
    match spec.operands {
        Operands::None => {}
        Operands::Copy => {
            let off = int_arg(ins, 0)? as i32;
            let size = int_arg(ins, 1)? as u16;
            out.extend_from_slice(&off.to_be_bytes());
            out.extend_from_slice(&size.to_be_bytes());
        }
        Operands::ConstInt | Operands::ConstObject | Operands::Offset | Operands::Increment => {
            let v = int_arg(ins, 0)? as i32;
            out.extend_from_slice(&v.to_be_bytes());
        }
        Operands::ConstFloat => {
            let Arg::Float(v) = ins.args.first().ok_or_else(|| err("missing float"))? else {
                return Err(err("expected float operand"));
            };
            out.extend_from_slice(&(*v as f32).to_be_bytes());
        }
        Operands::ConstString => {
            let Arg::Str(s) = ins.args.first().ok_or_else(|| err("missing string"))? else {
                return Err(err("expected string operand"));
            };
            let bytes = s.as_bytes();
            let len = u16::try_from(bytes.len()).map_err(|_| err("CONSTS longer than u16"))?;
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(bytes);
        }
        Operands::Action => {
            let routine = ins.routine.unwrap_or(int_arg(ins, 0)? as u16);
            let argc = ins.argc.unwrap_or(int_arg(ins, 1)? as u8);
            out.extend_from_slice(&routine.to_be_bytes());
            out.push(argc);
        }
        Operands::Jump => {
            let Arg::Jump(target) = ins.args.first().ok_or_else(|| err("missing jump"))? else {
                return Err(err("expected jump operand"));
            };
            let rel = *target as i32 - ins.offset as i32;
            out.extend_from_slice(&rel.to_be_bytes());
        }
        Operands::Destruct => {
            let a = int_arg(ins, 0)? as u16;
            let b = int_arg(ins, 1)? as i16;
            let c = int_arg(ins, 2)? as u16;
            out.extend_from_slice(&a.to_be_bytes());
            out.extend_from_slice(&b.to_be_bytes());
            out.extend_from_slice(&c.to_be_bytes());
        }
        Operands::StoreState => {
            let a = int_arg(ins, 0)? as u32;
            let b = int_arg(ins, 1)? as u32;
            out.extend_from_slice(&a.to_be_bytes());
            out.extend_from_slice(&b.to_be_bytes());
        }
        Operands::StructCompare => {
            let v = int_arg(ins, 0)? as u16;
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    Ok(())
}

fn int_arg(ins: &Instruction, i: usize) -> Result<i64> {
    match ins.args.get(i) {
        Some(Arg::Int(v)) => Ok(*v),
        _ => Err(err(format!("expected int operand {i} on {}", ins.op))),
    }
}

fn take_u8(data: &[u8], pos: &mut usize, end: usize) -> Result<u8> {
    if *pos >= end {
        return Err(err("unexpected EOF"));
    }
    let b = data[*pos];
    *pos += 1;
    Ok(b)
}

fn take_bytes<'a>(data: &'a [u8], pos: &mut usize, end: usize, n: usize) -> Result<&'a [u8]> {
    if *pos + n > end {
        return Err(err("unexpected EOF"));
    }
    let s = &data[*pos..*pos + n];
    *pos += n;
    Ok(s)
}

fn take_u16(data: &[u8], pos: &mut usize, end: usize) -> Result<u16> {
    let b = take_bytes(data, pos, end, 2)?;
    Ok(u16::from_be_bytes(b.try_into().unwrap()))
}

fn take_i16(data: &[u8], pos: &mut usize, end: usize) -> Result<i16> {
    let b = take_bytes(data, pos, end, 2)?;
    Ok(i16::from_be_bytes(b.try_into().unwrap()))
}

fn take_u32(data: &[u8], pos: &mut usize, end: usize) -> Result<u32> {
    let b = take_bytes(data, pos, end, 4)?;
    Ok(u32::from_be_bytes(b.try_into().unwrap()))
}

fn take_i32(data: &[u8], pos: &mut usize, end: usize) -> Result<i32> {
    let b = take_bytes(data, pos, end, 4)?;
    Ok(i32::from_be_bytes(b.try_into().unwrap()))
}

fn take_f32(data: &[u8], pos: &mut usize, end: usize) -> Result<f32> {
    Ok(f32::from_bits(take_u32(data, pos, end)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(size: u32) -> Vec<u8> {
        let mut b = b"NCS V1.0".to_vec();
        b.push(0x42);
        b.extend_from_slice(&size.to_be_bytes());
        b
    }

    #[test]
    fn empty_script_is_valid() {
        let data = header(13);
        let n = read(&data).unwrap();
        assert!(n.instructions.is_empty());
    }

    #[test]
    fn retn_only() {
        let mut data = header(15);
        data.extend_from_slice(&[0x20, 0x00]);
        let n = read(&data).unwrap();
        assert_eq!(n.instructions.len(), 1);
        assert_eq!(n.instructions[0].op, "RETN");
        assert_eq!(n.instructions[0].offset, 13);
    }

    #[test]
    fn consts_and_action() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x04, 0x05]);
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(b"hi");
        body.extend_from_slice(&[0x05, 0x00]);
        body.extend_from_slice(&200u16.to_be_bytes());
        body.push(2);
        body.extend_from_slice(&[0x20, 0x00]);
        let mut data = header(13 + body.len() as u32);
        data.extend_from_slice(&body);
        let n = read(&data).unwrap();
        assert_eq!(n.instructions[0].op, "CONSTS");
        match &n.instructions[0].args[0] {
            Arg::Str(s) => assert_eq!(s, "hi"),
            other => panic!("{other:?}"),
        }
        assert_eq!(n.instructions[1].op, "ACTION");
        assert_eq!(n.instructions[1].routine_name, None);
        assert_eq!(n.instructions[1].argc, Some(2));
    }

    #[test]
    fn jump_is_absolute() {
        let mut data = header(21);
        data.extend_from_slice(&[0x1D, 0x00]);
        data.extend_from_slice(&6i32.to_be_bytes());
        data.extend_from_slice(&[0x20, 0x00]);
        let n = read(&data).unwrap();
        match n.instructions[0].args[0] {
            Arg::Jump(t) => assert_eq!(t, 19),
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn trailing_zero_padding_is_not_an_error() {
        let mut data = header(20);
        data.extend_from_slice(&[0x20, 0x00]);
        data.extend_from_slice(&[0, 0, 0, 0, 0]);
        let n = read(&data).unwrap();
        assert_eq!(n.instructions.len(), 1);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut data = b"NCS V1.0".to_vec();
        data.push(0x00);
        data.extend_from_slice(&13u32.to_be_bytes());
        assert!(read(&data).is_err());
    }

    #[test]
    fn increment_offset_is_signed() {
        let mut data = header(19);
        data.extend_from_slice(&[0x24, 0x03]);
        data.extend_from_slice(&(-12i32).to_be_bytes());
        let n = read(&data).unwrap();
        assert_eq!(n.instructions.last().unwrap().args[0], Arg::Int(-12));
    }

    #[test]
    fn write_round_trips_retn() {
        let mut data = header(15);
        data.extend_from_slice(&[0x20, 0x00]);
        let n = read(&data).unwrap();
        let out = write(&n).unwrap();
        assert_eq!(out, data);
    }
}
