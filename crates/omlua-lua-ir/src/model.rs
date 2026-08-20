use crate::{LirBlockId, LirFunctionId, LirLocalId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LirProgram {
    pub entry: LirFunctionId,
    pub requirements: BackendRequirements,
    pub helpers: Vec<RuntimeHelper>,
    pub functions: Vec<LirFunction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendRequirements {
    pub minimum_integer_bits: u8,
    pub label_jumps: bool,
    pub native_bitwise: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeHelper {
    I32DivTrunc,
    I32Rem,
    DeepCopy,
    RefGet,
    RefSet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LirFunction {
    pub id: LirFunctionId,
    pub entry: LirBlockId,
    pub parameters: Vec<LirLocalId>,
    pub return_local: Option<LirLocalId>,
    pub locals: Vec<LirLocal>,
    pub blocks: Vec<LirBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LirLocal {
    pub id: LirLocalId,
    pub kind: LirValueKind,
    pub parameter: bool,
    pub addressable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LirRefKind {
    Shared,
    Mutable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirValueKind {
    Bool,
    Integer,
    Table(Vec<LirValueKind>),
    Enum(Vec<Vec<LirValueKind>>),
    Reference {
        kind: LirRefKind,
        pointee: Box<LirValueKind>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LirBlock {
    pub id: LirBlockId,
    pub statements: Vec<LirStatement>,
    pub terminator: LirTerminator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirStatement {
    Assign {
        destination: LirLocalId,
        value: LirExpression,
    },
    Store {
        destination: LirPlace,
        value: LirExpression,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirPlace {
    Local(LirLocalId),
    TableField {
        table: Box<LirExpression>,
        index: u32,
        result: LirValueKind,
    },
    EnumField {
        value: Box<LirExpression>,
        variant: u32,
        field: u32,
        result: LirValueKind,
    },
    Deref {
        reference: Box<LirExpression>,
        result: LirValueKind,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirExpression {
    Value(LirValue),
    Unary {
        op: LirUnaryOp,
        operand: Box<LirExpression>,
    },
    Binary {
        op: LirBinaryOp,
        left: Box<LirExpression>,
        right: Box<LirExpression>,
    },
    RuntimeCall {
        helper: RuntimeHelper,
        arguments: Vec<LirExpression>,
    },
    Table {
        fields: Vec<LirExpression>,
    },
    TableGet {
        table: Box<LirExpression>,
        index: u32,
        result: LirValueKind,
    },
    Enum {
        shapes: Vec<Vec<LirValueKind>>,
        tag: u32,
        fields: Vec<LirExpression>,
    },
    EnumTag {
        value: Box<LirExpression>,
    },
    EnumField {
        value: Box<LirExpression>,
        variant: u32,
        field: u32,
        result: LirValueKind,
    },
    Reference {
        kind: LirRefKind,
        place: Box<LirPlace>,
    },
    DerefGet {
        reference: Box<LirExpression>,
        result: LirValueKind,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirValue {
    Local(LirLocalId),
    Bool(bool),
    Integer(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LirUnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LirBinaryOp {
    And,
    Or,
    Add,
    Sub,
    Mul,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LirTerminator {
    Jump {
        target: LirBlockId,
    },
    Switch {
        discriminant: LirExpression,
        targets: Vec<(i64, LirBlockId)>,
        otherwise: LirBlockId,
    },
    Call {
        callee: LirFunctionId,
        arguments: Vec<LirExpression>,
        destination: Option<LirPlace>,
        target: LirBlockId,
    },
    Branch {
        condition: LirExpression,
        if_true: LirBlockId,
        if_false: LirBlockId,
    },
    Return {
        value: Option<LirExpression>,
    },
    Raise {
        message: String,
    },
    Unreachable,
}
