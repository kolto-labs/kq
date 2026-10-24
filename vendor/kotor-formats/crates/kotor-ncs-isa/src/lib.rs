//! The NCS instruction set, as data.
//!
//! An NCS instruction is identified by two bytes: an opcode and a type
//! qualifier. Together they name the operation and say what operand bytes
//! follow. That mapping is a fact about the bytecode format, not about any
//! particular program that reads or writes it, so it lives here on its own:
//! a disassembler consults it to decode, a compiler backend consults it to
//! emit, and neither has to depend on the other to agree.
//!
//! This crate holds no reader and no writer, and has no dependencies.

#![forbid(unsafe_code)]

/// The operand bytes that follow an instruction's opcode and qualifier.
///
/// Every variant describes a fixed layout except [`Operands::ConstString`],
/// whose length is carried in the instruction itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operands {
    /// No operand bytes.
    None,
    /// Stack offset and size: `i32`, `u16`.
    Copy,
    /// One `i32`.
    ConstInt,
    /// One `f32`.
    ConstFloat,
    /// A `u16` length followed by that many bytes of text.
    ConstString,
    /// One `u32` object id.
    ConstObject,
    /// Engine routine id (`u16`) and argument count (`u8`).
    Action,
    /// One `i32` stack offset.
    Offset,
    /// One `i32` relative jump.
    Jump,
    /// Bytes to remove, offset to keep, size to keep: `u16`, `i16`, `u16`.
    Destruct,
    /// One `u32` stack offset.
    Increment,
    /// Two `u32` sizes.
    StoreState,
    /// One `u16` struct size.
    StructCompare,
}

impl Operands {
    /// Byte width of the operands, when it is fixed.
    ///
    /// [`Operands::ConstString`] returns `None`: its width depends on the
    /// length prefix, so it can only be known while reading or writing.
    pub const fn byte_len(self) -> Option<usize> {
        Some(match self {
            Operands::None => 0,
            Operands::Copy => 6,
            Operands::ConstInt => 4,
            Operands::ConstFloat => 4,
            Operands::ConstString => return None,
            Operands::ConstObject => 4,
            Operands::Action => 3,
            Operands::Offset => 4,
            Operands::Jump => 4,
            Operands::Destruct => 6,
            Operands::Increment => 4,
            Operands::StoreState => 8,
            Operands::StructCompare => 2,
        })
    }
}

/// One instruction's identity: its two selector bytes, name, and operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: u8,
    pub qualifier: u8,
    pub mnemonic: &'static str,
    pub operands: Operands,
}

/// Opcode `0x00` is padding, and is accepted with any qualifier.
pub const RESERVED_OPCODE: u8 = 0x00;

/// The instruction named by opcode `0x00`.
pub const RESERVED: Instruction = Instruction {
    opcode: RESERVED_OPCODE,
    qualifier: 0x00,
    mnemonic: "RESERVED",
    operands: Operands::None,
};

/// Every instruction the games' bytecode uses, in opcode order.
pub static INSTRUCTIONS: &[Instruction] = &{
    use Operands::*;
    macro_rules! ins {
        ($( ($op:expr, $qual:expr, $name:literal, $ops:expr) ),* $(,)?) => {
            [ $( Instruction {
                opcode: $op,
                qualifier: $qual,
                mnemonic: $name,
                operands: $ops,
            } ),* ]
        };
    }

    ins![
        (0x01, 0x01, "CPDOWNSP", Copy),
        (0x02, 0x03, "RSADDI", None),
        (0x02, 0x04, "RSADDF", None),
        (0x02, 0x05, "RSADDS", None),
        (0x02, 0x06, "RSADDO", None),
        (0x02, 0x10, "RSADDEFF", None),
        (0x02, 0x11, "RSADDEVT", None),
        (0x02, 0x12, "RSADDLOC", None),
        (0x02, 0x13, "RSADDTAL", None),
        (0x03, 0x01, "CPTOPSP", Copy),
        (0x04, 0x03, "CONSTI", ConstInt),
        (0x04, 0x04, "CONSTF", ConstFloat),
        (0x04, 0x05, "CONSTS", ConstString),
        (0x04, 0x06, "CONSTO", ConstObject),
        (0x05, 0x00, "ACTION", Action),
        (0x06, 0x20, "LOGANDII", None),
        (0x07, 0x20, "LOGORII", None),
        (0x08, 0x20, "INCORII", None),
        (0x09, 0x20, "EXCORII", None),
        (0x0A, 0x20, "BOOLANDII", None),
        (0x0B, 0x20, "EQUALII", None),
        (0x0B, 0x21, "EQUALFF", None),
        (0x0B, 0x22, "EQUALOO", None),
        (0x0B, 0x23, "EQUALSS", None),
        (0x0B, 0x24, "EQUALTT", StructCompare),
        (0x0B, 0x30, "EQUALEFFEFF", None),
        (0x0B, 0x31, "EQUALEVTEVT", None),
        (0x0B, 0x32, "EQUALLOCLOC", None),
        (0x0B, 0x33, "EQUALTALTAL", None),
        (0x0C, 0x20, "NEQUALII", None),
        (0x0C, 0x21, "NEQUALFF", None),
        (0x0C, 0x22, "NEQUALOO", None),
        (0x0C, 0x23, "NEQUALSS", None),
        (0x0C, 0x24, "NEQUALTT", StructCompare),
        (0x0C, 0x30, "NEQUALEFFEFF", None),
        (0x0C, 0x31, "NEQUALEVTEVT", None),
        (0x0C, 0x32, "NEQUALLOCLOC", None),
        (0x0C, 0x33, "NEQUALTALTAL", None),
        (0x0D, 0x20, "GEQII", None),
        (0x0D, 0x21, "GEQFF", None),
        (0x0E, 0x20, "GTII", None),
        (0x0E, 0x21, "GTFF", None),
        (0x0F, 0x20, "LTII", None),
        (0x0F, 0x21, "LTFF", None),
        (0x10, 0x20, "LEQII", None),
        (0x10, 0x21, "LEQFF", None),
        (0x11, 0x20, "SHLEFTII", None),
        (0x12, 0x20, "SHRIGHTII", None),
        (0x13, 0x20, "USHRIGHTII", None),
        (0x14, 0x20, "ADDII", None),
        (0x14, 0x21, "ADDFF", None),
        (0x14, 0x23, "ADDSS", None),
        (0x14, 0x25, "ADDIF", None),
        (0x14, 0x26, "ADDFI", None),
        (0x14, 0x3A, "ADDVV", None),
        (0x15, 0x20, "SUBII", None),
        (0x15, 0x21, "SUBFF", None),
        (0x15, 0x25, "SUBIF", None),
        (0x15, 0x26, "SUBFI", None),
        (0x15, 0x3A, "SUBVV", None),
        (0x16, 0x20, "MULII", None),
        (0x16, 0x21, "MULFF", None),
        (0x16, 0x25, "MULIF", None),
        (0x16, 0x26, "MULFI", None),
        (0x16, 0x3B, "MULVF", None),
        (0x16, 0x3C, "MULFV", None),
        (0x17, 0x20, "DIVII", None),
        (0x17, 0x21, "DIVFF", None),
        (0x17, 0x25, "DIVIF", None),
        (0x17, 0x26, "DIVFI", None),
        (0x17, 0x3B, "DIVVF", None),
        (0x17, 0x3C, "DIVFV", None),
        (0x18, 0x20, "MODII", None),
        (0x19, 0x03, "NEGI", None),
        (0x19, 0x04, "NEGF", None),
        (0x1A, 0x03, "COMPI", None),
        (0x1B, 0x00, "MOVSP", Offset),
        (0x1D, 0x00, "JMP", Jump),
        (0x1E, 0x00, "JSR", Jump),
        (0x1F, 0x00, "JZ", Jump),
        (0x20, 0x00, "RETN", None),
        (0x21, 0x01, "DESTRUCT", Destruct),
        (0x22, 0x03, "NOTI", None),
        (0x23, 0x03, "DECxSP", Increment),
        (0x24, 0x03, "INCxSP", Increment),
        (0x25, 0x00, "JNZ", Jump),
        (0x26, 0x01, "CPDOWNBP", Copy),
        (0x27, 0x01, "CPTOPBP", Copy),
        (0x28, 0x03, "DECxBP", Increment),
        (0x29, 0x03, "INCxBP", Increment),
        (0x2A, 0x00, "SAVEBP", None),
        (0x2B, 0x00, "RESTOREBP", None),
        (0x2C, 0x10, "STORE_STATE", StoreState),
        (0x2D, 0x00, "NOP", None),
    ]
};

/// Look an instruction up by the two bytes that select it.
///
/// Opcode `0x00` is padding and matches whatever qualifier follows it.
pub fn lookup(opcode: u8, qualifier: u8) -> Option<Instruction> {
    if opcode == RESERVED_OPCODE {
        return Some(RESERVED);
    }
    INSTRUCTIONS
        .iter()
        .copied()
        .find(|ins| ins.opcode == opcode && ins.qualifier == qualifier)
}

/// Look an instruction up by mnemonic, for emitting one by name.
pub fn lookup_mnemonic(mnemonic: &str) -> Option<Instruction> {
    INSTRUCTIONS
        .iter()
        .copied()
        .find(|ins| ins.mnemonic == mnemonic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_bytes_are_unique() {
        for (i, ins) in INSTRUCTIONS.iter().enumerate() {
            let duplicate = INSTRUCTIONS[..i]
                .iter()
                .any(|other| other.opcode == ins.opcode && other.qualifier == ins.qualifier);
            assert!(!duplicate, "duplicate selector for {}", ins.mnemonic);
        }
    }

    #[test]
    fn mnemonics_are_unique() {
        for (i, ins) in INSTRUCTIONS.iter().enumerate() {
            let duplicate = INSTRUCTIONS[..i]
                .iter()
                .any(|other| other.mnemonic == ins.mnemonic);
            assert!(!duplicate, "duplicate mnemonic {}", ins.mnemonic);
        }
    }

    #[test]
    fn lookup_round_trips_through_both_directions() {
        for ins in INSTRUCTIONS {
            let found = lookup(ins.opcode, ins.qualifier).expect("selector lookup");
            assert_eq!(&found, ins);

            let by_name = lookup_mnemonic(ins.mnemonic).expect("mnemonic lookup");
            assert_eq!(&by_name, ins);
        }
    }

    #[test]
    fn padding_matches_any_qualifier() {
        assert_eq!(lookup(0x00, 0x00), Some(RESERVED));
        assert_eq!(lookup(0x00, 0xFF), Some(RESERVED));
    }

    #[test]
    fn unknown_selectors_are_rejected() {
        // 0x1C sits in a gap between MOVSP and JMP.
        assert_eq!(lookup(0x1C, 0x00), None);
        // A real opcode with a qualifier it never carries.
        assert_eq!(lookup(0x04, 0x99), None);
        assert_eq!(lookup_mnemonic("NOT_AN_INSTRUCTION"), None);
    }

    #[test]
    fn only_the_length_prefixed_string_has_no_fixed_width() {
        for ins in INSTRUCTIONS {
            if ins.operands == Operands::ConstString {
                assert_eq!(ins.operands.byte_len(), None);
            } else {
                assert!(
                    ins.operands.byte_len().is_some(),
                    "{} should have a fixed operand width",
                    ins.mnemonic
                );
            }
        }
    }
}
