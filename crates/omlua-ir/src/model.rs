use crate::{BlockId, FunctionId, LocalId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmProgram {
    pub entry: FunctionId,
    pub functions: Vec<OmFunction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmFunction {
    pub id: FunctionId,
    pub name: String,
    pub return_type: OmType,
    pub parameters: Vec<LocalId>,
    pub locals: Vec<OmLocal>,
    pub blocks: Vec<OmBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmLocal {
    pub id: LocalId,
    pub ty: OmType,
    pub kind: LocalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Return,
    Parameter,
    Temporary,
    CheckedValue,
    CheckedOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmType {
    Unit,
    Bool,
    I32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Assign {
        destination: LocalId,
        value: Rvalue,
    },
    CheckedBinary {
        value: LocalId,
        overflow: LocalId,
        op: CheckedBinaryOp,
        left: Operand,
        right: Operand,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rvalue {
    Use(Operand),
    Unary {
        op: UnaryOp,
        operand: Operand,
    },
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Copy(LocalId),
    Move(LocalId),
    Constant(Constant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constant {
    Unit,
    Bool(bool),
    I32(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    And,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedBinaryOp {
    Add,
    Sub,
    Mul,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    Goto {
        target: BlockId,
    },
    SwitchInt {
        discriminant: Operand,
        targets: Vec<(SwitchValue, BlockId)>,
        otherwise: BlockId,
    },
    Call {
        callee: FunctionId,
        arguments: Vec<Operand>,
        destination: LocalId,
        target: BlockId,
        unwind: UnwindAction,
    },
    Assert {
        condition: Operand,
        expected: bool,
        kind: AssertKind,
        target: BlockId,
        unwind: UnwindAction,
    },
    Return,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchValue(pub u128);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssertKind {
    Overflow {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    OverflowNeg(Operand),
    DivisionByZero(Operand),
    RemainderByZero(Operand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindAction {
    Continue,
    Unreachable,
    TerminateAbi,
    TerminateInCleanup,
    Cleanup(BlockId),
}
