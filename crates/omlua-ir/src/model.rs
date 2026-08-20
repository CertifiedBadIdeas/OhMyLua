use crate::{BlockId, FieldId, FunctionId, LocalId, TypeId, VariantId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmProgram {
    pub entry: FunctionId,
    pub structs: Vec<OmStruct>,
    pub enums: Vec<OmEnum>,
    pub functions: Vec<OmFunction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmStruct {
    pub id: TypeId,
    pub name: String,
    pub fields: Vec<OmField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmEnum {
    pub id: TypeId,
    pub name: String,
    pub variants: Vec<OmVariant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmVariant {
    pub id: VariantId,
    pub name: String,
    pub fields: Vec<OmField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmField {
    pub id: FieldId,
    pub name: String,
    pub ty: OmType,
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
    Discriminant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Shared,
    Mutable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefTarget {
    Unit,
    Bool,
    I32,
    Struct(TypeId),
    Enum(TypeId),
}

impl RefTarget {
    pub fn as_type(self) -> OmType {
        match self {
            Self::Unit => OmType::Unit,
            Self::Bool => OmType::Bool,
            Self::I32 => OmType::I32,
            Self::Struct(id) => OmType::Struct(id),
            Self::Enum(id) => OmType::Enum(id),
        }
    }
}

impl OmType {
    pub fn as_ref_target(self) -> Option<RefTarget> {
        match self {
            Self::Unit => Some(RefTarget::Unit),
            Self::Bool => Some(RefTarget::Bool),
            Self::I32 => Some(RefTarget::I32),
            Self::Struct(id) => Some(RefTarget::Struct(id)),
            Self::Enum(id) => Some(RefTarget::Enum(id)),
            Self::Ref { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmType {
    Unit,
    Bool,
    I32,
    Struct(TypeId),
    Enum(TypeId),
    Ref {
        kind: RefKind,
        target: RefTarget,
    },
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
    Store {
        destination: Place,
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
    Struct {
        ty: TypeId,
        fields: Vec<Operand>,
    },
    Variant {
        ty: TypeId,
        variant: VariantId,
        fields: Vec<Operand>,
    },
    Discriminant {
        source: Operand,
    },
    Borrow {
        kind: RefKind,
        source: Place,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub base: LocalId,
    pub path: Vec<ProjectElem>,
}

impl Place {
    pub fn local(base: LocalId) -> Self {
        Self {
            base,
            path: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Copy(LocalId),
    Move(LocalId),
    Project {
        base: LocalId,
        path: Vec<ProjectElem>,
        moved: bool,
    },
    Constant(Constant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectElem {
    Deref,
    Downcast(VariantId),
    Field(FieldId),
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
    BitNot,
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
        destination: Place,
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
