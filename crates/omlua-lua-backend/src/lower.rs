use std::collections::BTreeSet;

use omlua_ir::{
    AssertKind, BinaryOp, CheckedBinaryOp, Constant, OmFunction, OmProgram, OmType, Operand,
    Rvalue, Statement, Terminator, UnaryOp, UnwindAction,
};
use omlua_lua_ir::{
    BackendRequirements, LirBinaryOp, LirBlock, LirBlockId, LirExpression, LirFunction,
    LirFunctionId, LirLocal, LirLocalId, LirProgram, LirStatement, LirTerminator, LirUnaryOp,
    LirValue, LirValueKind, RuntimeHelper,
};

use crate::{LowerError, LuaBackendProfile, LuaDialect};

pub fn lower_program(
    program: &OmProgram,
    profile: &LuaBackendProfile,
) -> Result<LirProgram, LowerError> {
    if profile.dialect() != LuaDialect::Lua54 {
        return Err(LowerError::program("unsupported Lua backend dialect"));
    }
    if profile.numeric().integer_bits < 64 {
        return Err(LowerError::program(
            "backend integer width is insufficient for checked i32 arithmetic",
        ));
    }
    if !profile.control_flow().label_jumps {
        return Err(LowerError::program(
            "backend does not support label-based control flow",
        ));
    }

    let mut helpers = BTreeSet::new();
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        functions.push(FunctionLowerer::new(function, &mut helpers)?.lower()?);
    }

    Ok(LirProgram {
        entry: LirFunctionId::new(program.entry.index()),
        requirements: BackendRequirements {
            minimum_integer_bits: 64,
            label_jumps: true,
            native_bitwise: false,
        },
        helpers: helpers.into_iter().collect(),
        functions,
    })
}

struct FunctionLowerer<'a> {
    function: &'a OmFunction,
    helpers: &'a mut BTreeSet<RuntimeHelper>,
    next_block: u32,
    error_blocks: Vec<LirBlock>,
}

impl<'a> FunctionLowerer<'a> {
    fn new(
        function: &'a OmFunction,
        helpers: &'a mut BTreeSet<RuntimeHelper>,
    ) -> Result<Self, LowerError> {
        let next_block = function
            .blocks
            .iter()
            .map(|block| block.id.index())
            .max()
            .ok_or_else(|| LowerError::function(&function.name, "function has no basic blocks"))?
            .checked_add(1)
            .ok_or_else(|| LowerError::function(&function.name, "basic block ID overflow"))?;
        Ok(Self {
            function,
            helpers,
            next_block,
            error_blocks: Vec::new(),
        })
    }

    fn lower(mut self) -> Result<LirFunction, LowerError> {
        let mut blocks = Vec::with_capacity(self.function.blocks.len());
        for block in &self.function.blocks {
            let mut statements = Vec::new();
            for statement in &block.statements {
                self.lower_statement(block.id.index(), statement, &mut statements)?;
            }
            let terminator = self.lower_terminator(block.id.index(), &block.terminator)?;
            blocks.push(LirBlock {
                id: LirBlockId::new(block.id.index()),
                statements,
                terminator,
            });
        }
        blocks.append(&mut self.error_blocks);

        let mut locals = Vec::new();
        for local in &self.function.locals {
            let Some(kind) = lower_type(local.ty) else {
                continue;
            };
            locals.push(LirLocal {
                id: LirLocalId::new(local.id.index()),
                kind,
                parameter: self.function.parameters.contains(&local.id),
            });
        }

        let mut parameters = Vec::new();
        for id in &self.function.parameters {
            if self.local_type(id.index())? != OmType::Unit {
                parameters.push(LirLocalId::new(id.index()));
            }
        }

        Ok(LirFunction {
            id: LirFunctionId::new(self.function.id.index()),
            entry: LirBlockId::new(self.function.blocks[0].id.index()),
            parameters,
            return_local: lower_type(self.function.return_type).map(|_| LirLocalId::new(0)),
            locals,
            blocks,
        })
    }

    fn lower_statement(
        &mut self,
        block: u32,
        statement: &Statement,
        output: &mut Vec<LirStatement>,
    ) -> Result<(), LowerError> {
        match statement {
            Statement::Assign { destination, value } => {
                if self.local_type(destination.index())? == OmType::Unit {
                    if matches!(value, Rvalue::Use(Operand::Constant(Constant::Unit))) {
                        return Ok(());
                    }
                    return Err(self.block_error(block, "non-unit value assigned to a unit local"));
                }
                output.push(LirStatement::Assign {
                    destination: LirLocalId::new(destination.index()),
                    value: self.lower_rvalue(block, value)?,
                });
            }
            Statement::CheckedBinary {
                value,
                overflow,
                op,
                left,
                right,
            } => {
                let native_op = match op {
                    CheckedBinaryOp::Add => LirBinaryOp::Add,
                    CheckedBinaryOp::Sub => LirBinaryOp::Sub,
                    CheckedBinaryOp::Mul => LirBinaryOp::Mul,
                };
                output.push(LirStatement::Assign {
                    destination: LirLocalId::new(value.index()),
                    value: binary(
                        native_op,
                        self.lower_operand(block, left)?,
                        self.lower_operand(block, right)?,
                    ),
                });
                let result = local(value.index());
                output.push(LirStatement::Assign {
                    destination: LirLocalId::new(overflow.index()),
                    value: binary(
                        LirBinaryOp::Or,
                        binary(LirBinaryOp::Lt, result.clone(), integer(i32::MIN.into())),
                        binary(LirBinaryOp::Gt, result, integer(i32::MAX.into())),
                    ),
                });
            }
        }
        Ok(())
    }

    fn lower_rvalue(&mut self, block: u32, value: &Rvalue) -> Result<LirExpression, LowerError> {
        match value {
            Rvalue::Use(operand) => self.lower_operand(block, operand),
            Rvalue::Unary { op, operand } => Ok(LirExpression::Unary {
                op: match op {
                    UnaryOp::Neg => LirUnaryOp::Neg,
                    UnaryOp::Not => LirUnaryOp::Not,
                },
                operand: Box::new(self.lower_operand(block, operand)?),
            }),
            Rvalue::Binary { op, left, right } => {
                let left = self.lower_operand(block, left)?;
                let right = self.lower_operand(block, right)?;
                match op {
                    BinaryOp::Div => {
                        self.helpers.insert(RuntimeHelper::I32DivTrunc);
                        Ok(runtime(RuntimeHelper::I32DivTrunc, vec![left, right]))
                    }
                    BinaryOp::Rem => {
                        self.helpers.insert(RuntimeHelper::I32DivTrunc);
                        self.helpers.insert(RuntimeHelper::I32Rem);
                        Ok(runtime(RuntimeHelper::I32Rem, vec![left, right]))
                    }
                    _ => Ok(binary(lower_binary(*op), left, right)),
                }
            }
        }
    }

    fn lower_operand(&self, block: u32, operand: &Operand) -> Result<LirExpression, LowerError> {
        match operand {
            Operand::Copy(local_id) | Operand::Move(local_id) => {
                if self.local_type(local_id.index())? == OmType::Unit {
                    return Err(self.block_error(block, "unit local used as a scalar value"));
                }
                Ok(local(local_id.index()))
            }
            Operand::Constant(Constant::Unit) => {
                Err(self.block_error(block, "unit constant used as a scalar value"))
            }
            Operand::Constant(Constant::Bool(value)) => {
                Ok(LirExpression::Value(LirValue::Bool(*value)))
            }
            Operand::Constant(Constant::I32(value)) => Ok(integer((*value).into())),
        }
    }

    fn lower_terminator(
        &mut self,
        block: u32,
        terminator: &Terminator,
    ) -> Result<LirTerminator, LowerError> {
        match terminator {
            Terminator::Goto { target } => Ok(LirTerminator::Jump {
                target: LirBlockId::new(target.index()),
            }),
            Terminator::SwitchInt {
                discriminant,
                targets,
                otherwise,
            } => Ok(LirTerminator::Switch {
                discriminant: self.lower_operand(block, discriminant)?,
                targets: targets
                    .iter()
                    .map(|(value, target)| {
                        i64::try_from(value.0)
                            .map(|value| (value, LirBlockId::new(target.index())))
                            .map_err(|_| {
                                self.block_error(block, "switch value does not fit Lua integer")
                            })
                    })
                    .collect::<Result<_, _>>()?,
                otherwise: LirBlockId::new(otherwise.index()),
            }),
            Terminator::Call {
                callee,
                arguments,
                destination,
                target,
                unwind,
            } => {
                self.validate_unwind(block, *unwind)?;
                let destination = (self.local_type(destination.index())? != OmType::Unit)
                    .then(|| LirLocalId::new(destination.index()));
                let mut lowered_arguments = Vec::new();
                for argument in arguments {
                    if !self.operand_is_unit(argument)? {
                        lowered_arguments.push(self.lower_operand(block, argument)?);
                    }
                }
                Ok(LirTerminator::Call {
                    callee: LirFunctionId::new(callee.index()),
                    arguments: lowered_arguments,
                    destination,
                    target: LirBlockId::new(target.index()),
                })
            }
            Terminator::Assert {
                condition,
                expected,
                kind,
                target,
                unwind,
            } => {
                self.validate_unwind(block, *unwind)?;
                let message =
                    assert_message(kind).map_err(|detail| self.block_error(block, detail))?;
                let error = self.push_error_block(block, message)?;
                let normal = LirBlockId::new(target.index());
                Ok(LirTerminator::Branch {
                    condition: self.lower_operand(block, condition)?,
                    if_true: if *expected { normal } else { error },
                    if_false: if *expected { error } else { normal },
                })
            }
            Terminator::Return => Ok(LirTerminator::Return {
                value: lower_type(self.function.return_type).map(|_| local(0)),
            }),
            Terminator::Unreachable => Ok(LirTerminator::Unreachable),
        }
    }

    fn validate_unwind(&self, block: u32, unwind: UnwindAction) -> Result<(), LowerError> {
        let detail = match unwind {
            UnwindAction::Continue => return Ok(()),
            UnwindAction::Unreachable => "unreachable unwind actions",
            UnwindAction::TerminateAbi => "ABI-terminate unwind actions",
            UnwindAction::TerminateInCleanup => "cleanup-terminate unwind actions",
            UnwindAction::Cleanup(_) => "cleanup unwind edges",
        };
        Err(self.block_error(
            block,
            format!("{detail} are not supported by backend `lua54`"),
        ))
    }

    fn push_error_block(
        &mut self,
        source_block: u32,
        message: &'static str,
    ) -> Result<LirBlockId, LowerError> {
        let id = LirBlockId::new(self.next_block);
        self.next_block = self
            .next_block
            .checked_add(1)
            .ok_or_else(|| self.block_error(source_block, "basic block ID overflow"))?;
        self.error_blocks.push(LirBlock {
            id,
            statements: Vec::new(),
            terminator: LirTerminator::Raise {
                message: message.to_owned(),
            },
        });
        Ok(id)
    }

    fn local_type(&self, index: u32) -> Result<OmType, LowerError> {
        self.function
            .locals
            .iter()
            .find(|local| local.id.index() == index)
            .map(|local| local.ty)
            .ok_or_else(|| {
                LowerError::function(
                    &self.function.name,
                    format!("local %{index} does not exist"),
                )
            })
    }

    fn operand_is_unit(&self, operand: &Operand) -> Result<bool, LowerError> {
        match operand {
            Operand::Copy(local) | Operand::Move(local) => {
                Ok(self.local_type(local.index())? == OmType::Unit)
            }
            Operand::Constant(Constant::Unit) => Ok(true),
            Operand::Constant(Constant::Bool(_) | Constant::I32(_)) => Ok(false),
        }
    }

    fn block_error(&self, block: u32, detail: impl Into<String>) -> LowerError {
        LowerError::block(&self.function.name, block, detail)
    }
}

fn lower_type(ty: OmType) -> Option<LirValueKind> {
    match ty {
        OmType::Unit => None,
        OmType::Bool => Some(LirValueKind::Bool),
        OmType::I32 => Some(LirValueKind::Integer),
    }
}

fn lower_binary(op: BinaryOp) -> LirBinaryOp {
    match op {
        BinaryOp::And => LirBinaryOp::And,
        BinaryOp::Add => LirBinaryOp::Add,
        BinaryOp::Sub => LirBinaryOp::Sub,
        BinaryOp::Mul => LirBinaryOp::Mul,
        BinaryOp::Eq => LirBinaryOp::Eq,
        BinaryOp::Ne => LirBinaryOp::Ne,
        BinaryOp::Lt => LirBinaryOp::Lt,
        BinaryOp::Le => LirBinaryOp::Le,
        BinaryOp::Gt => LirBinaryOp::Gt,
        BinaryOp::Ge => LirBinaryOp::Ge,
        BinaryOp::Div | BinaryOp::Rem => unreachable!("division and remainder use helpers"),
    }
}

fn assert_message(kind: &AssertKind) -> Result<&'static str, &'static str> {
    match kind {
        AssertKind::Overflow {
            op: BinaryOp::Add, ..
        } => Ok("attempt to add with overflow"),
        AssertKind::Overflow {
            op: BinaryOp::Sub, ..
        } => Ok("attempt to subtract with overflow"),
        AssertKind::Overflow {
            op: BinaryOp::Mul, ..
        } => Ok("attempt to multiply with overflow"),
        AssertKind::Overflow {
            op: BinaryOp::Div, ..
        } => Ok("attempt to divide with overflow"),
        AssertKind::Overflow {
            op: BinaryOp::Rem, ..
        } => Ok("attempt to calculate the remainder with overflow"),
        AssertKind::Overflow { .. } => {
            Err("unsupported arithmetic operation in overflow assertion")
        }
        AssertKind::OverflowNeg(_) => Ok("attempt to negate with overflow"),
        AssertKind::DivisionByZero(_) => Ok("attempt to divide by zero"),
        AssertKind::RemainderByZero(_) => {
            Ok("attempt to calculate the remainder with a divisor of zero")
        }
    }
}

fn local(index: u32) -> LirExpression {
    LirExpression::Value(LirValue::Local(LirLocalId::new(index)))
}

fn integer(value: i64) -> LirExpression {
    LirExpression::Value(LirValue::Integer(value))
}

fn binary(op: LirBinaryOp, left: LirExpression, right: LirExpression) -> LirExpression {
    LirExpression::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn runtime(helper: RuntimeHelper, arguments: Vec<LirExpression>) -> LirExpression {
    LirExpression::RuntimeCall { helper, arguments }
}
