mod display;
mod ids;
mod model;

pub use ids::{BlockId, FieldId, FunctionId, LocalId, TypeId};
pub use model::{
    AssertKind, BinaryOp, CheckedBinaryOp, Constant, LocalKind, OmBlock, OmField, OmFunction,
    OmLocal, OmProgram, OmStruct, OmType, Operand, Rvalue, Statement, SwitchValue, Terminator,
    UnaryOp, UnwindAction,
};
