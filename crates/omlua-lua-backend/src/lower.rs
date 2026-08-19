use std::collections::{BTreeMap, BTreeSet};

use omlua_ir::{
    AssertKind, BinaryOp, CheckedBinaryOp, Constant, OmEnum, OmFunction, OmProgram, OmStruct,
    OmType, Operand, ProjectElem, Rvalue, Statement, Terminator, TypeId, UnaryOp, UnwindAction,
    VariantId,
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

    let definitions = index_definitions(program)?;
    for function in &program.functions {
        validate_nominal_type(function.return_type, &definitions, &function.name)?;
        for local in &function.locals {
            validate_nominal_type(local.ty, &definitions, &function.name)?;
        }
    }

    let mut helpers = BTreeSet::new();
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        functions.push(FunctionLowerer::new(function, &definitions, &mut helpers)?.lower()?);
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
    definitions: &'a Definitions<'a>,
    helpers: &'a mut BTreeSet<RuntimeHelper>,
    next_block: u32,
    error_blocks: Vec<LirBlock>,
}

impl<'a> FunctionLowerer<'a> {
    fn new(
        function: &'a OmFunction,
        definitions: &'a Definitions<'a>,
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
            definitions,
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
            let Some(kind) = lower_type(local.ty, self.definitions)? else {
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
            return_local: lower_type(self.function.return_type, self.definitions)?
                .map(|_| LirLocalId::new(0)),
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
                let destination_type = self.local_type(destination.index())?;
                let value_type = self.rvalue_type(block, value)?;
                if destination_type != value_type {
                    return Err(self.block_error(
                        block,
                        format!(
                            "assignment type mismatch: destination is {destination_type:?}, value is {value_type:?}"
                        ),
                    ));
                }
                if destination_type == OmType::Unit {
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
            Rvalue::Struct { fields, .. } => Ok(LirExpression::Table {
                fields: fields
                    .iter()
                    .map(|field| self.lower_operand(block, field))
                    .collect::<Result<_, _>>()?,
            }),
            Rvalue::SharedBorrow { source } => self.lower_operand(block, source),
            Rvalue::Variant {
                ty,
                variant,
                fields,
            } => {
                let definition = self.definitions.enums.get(ty).ok_or_else(|| {
                    self.block_error(block, format!("enum type @{ty} does not exist"))
                })?;
                let variant_definition =
                    self.enum_variant_definition(definition, block, *variant)?;
                if fields.len() != variant_definition.fields.len() {
                    return Err(self.block_error(
                        block,
                        format!(
                            "variant v{variant} of enum @{ty} expects {} fields, got {}",
                            variant_definition.fields.len(),
                            fields.len()
                        ),
                    ));
                }
                for (operand, field) in fields.iter().zip(&variant_definition.fields) {
                    let actual = self.operand_type(operand)?;
                    if actual != field.ty {
                        return Err(self.block_error(
                            block,
                            format!("field .{} has the wrong type", field.id),
                        ));
                    }
                }
                Ok(LirExpression::Enum {
                    shapes: enum_shapes(definition, self.definitions)?,
                    tag: variant.index(),
                    fields: fields
                        .iter()
                        .map(|field| self.lower_operand(block, field))
                        .collect::<Result<_, _>>()?,
                })
            }
            Rvalue::Discriminant { source } => {
                if !matches!(self.operand_type(source)?, OmType::Enum(_)) {
                    return Err(self.block_error(block, "discriminant source is not an enum value"));
                }
                Ok(LirExpression::EnumTag {
                    value: Box::new(self.lower_operand(block, source)?),
                })
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
            Operand::Project { base, path, .. } => {
                if path.is_empty() {
                    return Err(self.block_error(block, "field projection is empty"));
                }
                let mut value = local(base.index());
                let mut ty = self.local_type(base.index())?;
                let mut downcast: Option<VariantId> = None;
                for element in path {
                    match element {
                        ProjectElem::Deref => {
                            let OmType::SharedRef(id) = ty else {
                                return Err(self.block_error(
                                    block,
                                    "dereference source is not a shared structure reference",
                                ));
                            };
                            ty = OmType::Struct(id);
                        }
                        ProjectElem::Downcast(variant) => {
                            let OmType::Enum(id) = ty else {
                                return Err(
                                    self.block_error(block, "downcast crosses a non-enum value")
                                );
                            };
                            self.validate_variant(block, id, *variant)?;
                            downcast = Some(*variant);
                        }
                        ProjectElem::Field(field_id) => match (ty, downcast.take()) {
                            (OmType::Struct(struct_id), None) => {
                                let field = self.field(struct_id, field_id.index(), block)?;
                                let result =
                                    lower_type(field.ty, self.definitions)?.ok_or_else(|| {
                                        self.block_error(
                                            block,
                                            "unit structure fields are not supported",
                                        )
                                    })?;
                                value = LirExpression::TableGet {
                                    table: Box::new(value),
                                    index: field_id.index().checked_add(1).ok_or_else(|| {
                                        self.block_error(block, "Lua table field index overflow")
                                    })?,
                                    result,
                                };
                                ty = field.ty;
                            }
                            (OmType::Enum(enum_id), Some(variant)) => {
                                let field =
                                    self.enum_field(block, enum_id, variant, field_id.index())?;
                                let result =
                                    lower_type(field.ty, self.definitions)?.ok_or_else(|| {
                                        self.block_error(
                                            block,
                                            "unit enum fields are not supported",
                                        )
                                    })?;
                                value = LirExpression::EnumField {
                                    value: Box::new(value),
                                    variant: variant.index(),
                                    field: field_id.index(),
                                    result,
                                };
                                ty = field.ty;
                            }
                            (OmType::Enum(_), None) => {
                                return Err(self.block_error(
                                    block,
                                    "enum field access requires a variant downcast",
                                ));
                            }
                            _ => {
                                return Err(self.block_error(
                                    block,
                                    "field projection starts from a non-struct value",
                                ));
                            }
                        },
                    }
                }
                if downcast.is_some() {
                    return Err(self.block_error(
                        block,
                        "projection ends with a downcast without a field read",
                    ));
                }
                Ok(value)
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
            } => {
                let discriminant_type = self.operand_type(discriminant)?;
                let discriminant = self.lower_operand(block, discriminant)?;
                if discriminant_type == OmType::Bool {
                    let mut if_false = LirBlockId::new(otherwise.index());
                    let mut if_true = LirBlockId::new(otherwise.index());
                    let mut seen_false = false;
                    let mut seen_true = false;
                    for (value, target) in targets {
                        match value.0 {
                            0 if !seen_false => {
                                seen_false = true;
                                if_false = LirBlockId::new(target.index());
                            }
                            1 if !seen_true => {
                                seen_true = true;
                                if_true = LirBlockId::new(target.index());
                            }
                            0 | 1 => {
                                return Err(self.block_error(
                                    block,
                                    format!("boolean switch contains duplicate value {}", value.0),
                                ));
                            }
                            value => {
                                return Err(self.block_error(
                                    block,
                                    format!("boolean switch contains invalid value {value}"),
                                ));
                            }
                        }
                    }
                    Ok(LirTerminator::Branch {
                        condition: discriminant,
                        if_true,
                        if_false,
                    })
                } else {
                    Ok(LirTerminator::Switch {
                        discriminant,
                        targets: targets
                            .iter()
                            .map(|(value, target)| {
                                self.decode_i32_switch_value(block, value.0)
                                    .map(|value| (value, LirBlockId::new(target.index())))
                            })
                            .collect::<Result<_, _>>()?,
                        otherwise: LirBlockId::new(otherwise.index()),
                    })
                }
            }
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
                value: lower_type(self.function.return_type, self.definitions)?.map(|_| local(0)),
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
        Ok(self.operand_type(operand)? == OmType::Unit)
    }

    fn operand_type(&self, operand: &Operand) -> Result<OmType, LowerError> {
        match operand {
            Operand::Copy(local) | Operand::Move(local) => self.local_type(local.index()),
            Operand::Project { base, path, .. } => {
                if path.is_empty() {
                    return Err(LowerError::function(
                        &self.function.name,
                        "field projection is empty",
                    ));
                }
                let mut ty = self.local_type(base.index())?;
                let mut downcast: Option<VariantId> = None;
                for element in path {
                    match element {
                        ProjectElem::Deref => {
                            let OmType::SharedRef(id) = ty else {
                                return Err(LowerError::function(
                                    &self.function.name,
                                    "dereference source is not a shared structure reference",
                                ));
                            };
                            ty = OmType::Struct(id);
                        }
                        ProjectElem::Downcast(variant) => {
                            let OmType::Enum(_) = ty else {
                                return Err(LowerError::function(
                                    &self.function.name,
                                    "downcast crosses a non-enum value",
                                ));
                            };
                            downcast = Some(*variant);
                        }
                        ProjectElem::Field(field_id) => {
                            ty = match (ty, downcast.take()) {
                                (OmType::Struct(struct_id), None) => {
                                    self.field_for_function(struct_id, field_id.index())?.ty
                                }
                                (OmType::Enum(enum_id), Some(variant)) => {
                                    self.enum_field_for_function(
                                        enum_id,
                                        variant,
                                        field_id.index(),
                                    )?
                                    .ty
                                }
                                (OmType::Enum(_), None) => {
                                    return Err(LowerError::function(
                                        &self.function.name,
                                        "enum field access requires a variant downcast",
                                    ));
                                }
                                _ => {
                                    return Err(LowerError::function(
                                        &self.function.name,
                                        "field projection crosses a non-struct value",
                                    ));
                                }
                            };
                        }
                    }
                }
                if downcast.is_some() {
                    return Err(LowerError::function(
                        &self.function.name,
                        "projection ends with a downcast without a field read",
                    ));
                }
                Ok(ty)
            }
            Operand::Constant(Constant::Unit) => Ok(OmType::Unit),
            Operand::Constant(Constant::Bool(_)) => Ok(OmType::Bool),
            Operand::Constant(Constant::I32(_)) => Ok(OmType::I32),
        }
    }

    fn rvalue_type(&self, block: u32, value: &Rvalue) -> Result<OmType, LowerError> {
        match value {
            Rvalue::Use(operand) => self.operand_type(operand),
            Rvalue::Unary { op, .. } => Ok(match op {
                UnaryOp::Neg => OmType::I32,
                UnaryOp::Not => OmType::Bool,
            }),
            Rvalue::Binary { op, .. } => Ok(match op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => OmType::Bool,
                BinaryOp::And => OmType::Bool,
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                    OmType::I32
                }
            }),
            Rvalue::Struct { ty, fields } => {
                let definition = self.definitions.structs.get(ty).ok_or_else(|| {
                    self.block_error(block, format!("structure type @{ty} does not exist"))
                })?;
                if fields.len() != definition.fields.len() {
                    return Err(self.block_error(
                        block,
                        format!(
                            "structure @{} expects {} fields, got {}",
                            ty,
                            definition.fields.len(),
                            fields.len()
                        ),
                    ));
                }
                for (operand, field) in fields.iter().zip(&definition.fields) {
                    let actual = self.operand_type(operand)?;
                    if actual != field.ty {
                        return Err(self.block_error(
                            block,
                            format!("field .{} has the wrong type", field.id),
                        ));
                    }
                }
                Ok(OmType::Struct(*ty))
            }
            Rvalue::SharedBorrow { source } => match self.operand_type(source)? {
                OmType::Struct(id) => Ok(OmType::SharedRef(id)),
                _ => Err(self.block_error(block, "shared borrow source is not an owned structure")),
            },
            Rvalue::Variant {
                ty,
                variant,
                fields,
            } => {
                let definition = self.definitions.enums.get(ty).ok_or_else(|| {
                    self.block_error(block, format!("enum type @{ty} does not exist"))
                })?;
                let variant_definition =
                    self.enum_variant_definition(definition, block, *variant)?;
                if fields.len() != variant_definition.fields.len() {
                    return Err(self.block_error(
                        block,
                        format!(
                            "variant v{variant} of enum @{ty} expects {} fields, got {}",
                            variant_definition.fields.len(),
                            fields.len()
                        ),
                    ));
                }
                for (operand, field) in fields.iter().zip(&variant_definition.fields) {
                    let actual = self.operand_type(operand)?;
                    if actual != field.ty {
                        return Err(self.block_error(
                            block,
                            format!("field .{} has the wrong type", field.id),
                        ));
                    }
                }
                Ok(OmType::Enum(*ty))
            }
            Rvalue::Discriminant { source } => match self.operand_type(source)? {
                OmType::Enum(_) => Ok(OmType::I32),
                _ => Err(self.block_error(block, "discriminant source is not an enum value")),
            },
        }
    }

    fn field(
        &self,
        struct_id: TypeId,
        field_index: u32,
        block: u32,
    ) -> Result<&'a omlua_ir::OmField, LowerError> {
        self.field_for_function(struct_id, field_index)
            .map_err(|error| self.block_error(block, error.detail()))
    }

    fn field_for_function(
        &self,
        struct_id: TypeId,
        field_index: u32,
    ) -> Result<&'a omlua_ir::OmField, LowerError> {
        let definition = self.definitions.structs.get(&struct_id).ok_or_else(|| {
            LowerError::function(
                &self.function.name,
                format!("structure type @{struct_id} does not exist"),
            )
        })?;
        definition
            .fields
            .get(field_index as usize)
            .filter(|field| field.id.index() == field_index)
            .ok_or_else(|| {
                LowerError::function(
                    &self.function.name,
                    format!("field .{field_index} does not exist in structure @{struct_id}"),
                )
            })
    }

    fn validate_variant(
        &self,
        block: u32,
        enum_id: TypeId,
        variant: VariantId,
    ) -> Result<(), LowerError> {
        let definition = self.definitions.enums.get(&enum_id).ok_or_else(|| {
            self.block_error(block, format!("enum type @{enum_id} does not exist"))
        })?;
        self.enum_variant_definition(definition, block, variant)
            .map(|_| ())
    }

    fn enum_variant_definition(
        &self,
        definition: &'a OmEnum,
        block: u32,
        variant: VariantId,
    ) -> Result<&'a omlua_ir::OmVariant, LowerError> {
        definition
            .variants
            .get(variant.index() as usize)
            .filter(|variant_def| variant_def.id == variant)
            .ok_or_else(|| {
                self.block_error(
                    block,
                    format!(
                        "variant v{variant} does not exist in enum @{}",
                        definition.id
                    ),
                )
            })
    }

    fn enum_field(
        &self,
        block: u32,
        enum_id: TypeId,
        variant: VariantId,
        field_index: u32,
    ) -> Result<&'a omlua_ir::OmField, LowerError> {
        self.enum_field_for_function(enum_id, variant, field_index)
            .map_err(|error| self.block_error(block, error.detail()))
    }

    fn enum_field_for_function(
        &self,
        enum_id: TypeId,
        variant: VariantId,
        field_index: u32,
    ) -> Result<&'a omlua_ir::OmField, LowerError> {
        let definition = self.definitions.enums.get(&enum_id).ok_or_else(|| {
            LowerError::function(
                &self.function.name,
                format!("enum type @{enum_id} does not exist"),
            )
        })?;
        definition
            .variants
            .get(variant.index() as usize)
            .filter(|variant_def| variant_def.id == variant)
            .ok_or_else(|| {
                LowerError::function(
                    &self.function.name,
                    format!("variant v{variant} does not exist in enum @{enum_id}"),
                )
            })?
            .fields
            .get(field_index as usize)
            .filter(|field| field.id.index() == field_index)
            .ok_or_else(|| {
                LowerError::function(
                    &self.function.name,
                    format!(
                        "field .{field_index} does not exist in variant v{variant} of enum @{enum_id}"
                    ),
                )
            })
    }

    fn decode_i32_switch_value(&self, block: u32, value: u128) -> Result<i64, LowerError> {
        let bits = u32::try_from(value)
            .map_err(|_| self.block_error(block, "i32 switch value contains more than 32 bits"))?;
        Ok(i64::from(bits as i32))
    }

    fn block_error(&self, block: u32, detail: impl Into<String>) -> LowerError {
        LowerError::block(&self.function.name, block, detail)
    }
}

fn lower_type(
    ty: OmType,
    definitions: &Definitions<'_>,
) -> Result<Option<LirValueKind>, LowerError> {
    match ty {
        OmType::Unit => Ok(None),
        OmType::Bool => Ok(Some(LirValueKind::Bool)),
        OmType::I32 => Ok(Some(LirValueKind::Integer)),
        OmType::Enum(id) => {
            let definition = definitions
                .enums
                .get(&id)
                .ok_or_else(|| LowerError::program(format!("enum type @{id} does not exist")))?;
            Ok(Some(LirValueKind::Enum(enum_shapes(
                definition,
                definitions,
            )?)))
        }
        OmType::Struct(id) | OmType::SharedRef(id) => {
            let definition = definitions.structs.get(&id).ok_or_else(|| {
                LowerError::program(format!("structure type @{id} does not exist"))
            })?;
            let fields = definition
                .fields
                .iter()
                .map(|field| {
                    lower_type(field.ty, definitions)?.ok_or_else(|| {
                        LowerError::program(format!(
                            "field .{} of structure @{id} has unit type",
                            field.id
                        ))
                    })
                })
                .collect::<Result<_, _>>()?;
            Ok(Some(LirValueKind::Table(fields)))
        }
    }
}

fn enum_shapes(
    definition: &OmEnum,
    definitions: &Definitions<'_>,
) -> Result<Vec<Vec<LirValueKind>>, LowerError> {
    definition
        .variants
        .iter()
        .map(|variant| {
            variant
                .fields
                .iter()
                .map(|field| {
                    lower_type(field.ty, definitions)?.ok_or_else(|| {
                        LowerError::program(format!(
                            "field .{} of variant v{} of enum @{} has unit type",
                            field.id, variant.id, definition.id
                        ))
                    })
                })
                .collect()
        })
        .collect()
}

struct Definitions<'a> {
    structs: BTreeMap<TypeId, &'a OmStruct>,
    enums: BTreeMap<TypeId, &'a OmEnum>,
}

fn index_definitions(program: &OmProgram) -> Result<Definitions<'_>, LowerError> {
    let mut definitions = Definitions {
        structs: BTreeMap::new(),
        enums: BTreeMap::new(),
    };
    for definition in &program.structs {
        if definitions
            .structs
            .insert(definition.id, definition)
            .is_some()
        {
            return Err(LowerError::program(format!(
                "structure type @{} is defined twice",
                definition.id
            )));
        }
        if definitions.enums.contains_key(&definition.id) {
            return Err(LowerError::program(format!(
                "type @{} is both a structure and an enum",
                definition.id
            )));
        }
        for (index, field) in definition.fields.iter().enumerate() {
            if field.id.index() as usize != index {
                return Err(LowerError::program(format!(
                    "structure @{} has non-contiguous field identifier .{}",
                    definition.id, field.id
                )));
            }
        }
    }
    for definition in &program.enums {
        if definitions
            .enums
            .insert(definition.id, definition)
            .is_some()
        {
            return Err(LowerError::program(format!(
                "enum type @{} is defined twice",
                definition.id
            )));
        }
        if definitions.structs.contains_key(&definition.id) {
            return Err(LowerError::program(format!(
                "type @{} is both a structure and an enum",
                definition.id
            )));
        }
        for (variant_index, variant) in definition.variants.iter().enumerate() {
            if variant.id.index() as usize != variant_index {
                return Err(LowerError::program(format!(
                    "enum @{} has non-contiguous variant identifier v{}",
                    definition.id, variant.id
                )));
            }
            for (field_index, field) in variant.fields.iter().enumerate() {
                if field.id.index() as usize != field_index {
                    return Err(LowerError::program(format!(
                        "variant v{} of enum @{} has non-contiguous field identifier .{}",
                        variant.id, definition.id, field.id
                    )));
                }
            }
        }
    }
    for definition in &program.structs {
        for field in &definition.fields {
            validate_nominal_field(
                &definitions,
                format!("field .{} of structure @{}", field.id, definition.id),
                field.ty,
            )?;
        }
    }
    for definition in &program.enums {
        for variant in &definition.variants {
            for field in &variant.fields {
                validate_nominal_field(
                    &definitions,
                    format!(
                        "field .{} of variant v{} of enum @{}",
                        field.id, variant.id, definition.id
                    ),
                    field.ty,
                )?;
            }
        }
    }
    validate_acyclic_definitions(&definitions)?;
    Ok(definitions)
}

fn validate_nominal_field(
    definitions: &Definitions<'_>,
    description: impl std::fmt::Display,
    ty: OmType,
) -> Result<(), LowerError> {
    if let OmType::Struct(id) | OmType::SharedRef(id) | OmType::Enum(id) = ty {
        let known = definitions.structs.contains_key(&id) || definitions.enums.contains_key(&id);
        if !known {
            return Err(LowerError::program(format!(
                "{description} references missing type @{id}"
            )));
        }
    }
    if matches!(ty, OmType::Unit | OmType::SharedRef(_)) {
        return Err(LowerError::program(format!(
            "{description} has an unsupported type"
        )));
    }
    Ok(())
}

fn validate_acyclic_definitions(definitions: &Definitions<'_>) -> Result<(), LowerError> {
    fn visit(
        id: TypeId,
        definitions: &Definitions<'_>,
        visiting: &mut BTreeSet<TypeId>,
        complete: &mut BTreeSet<TypeId>,
    ) -> Result<(), LowerError> {
        if complete.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(LowerError::program(format!(
                "type definitions contain a by-value cycle through @{id}"
            )));
        }
        if let Some(definition) = definitions.structs.get(&id) {
            for field in &definition.fields {
                if let OmType::Struct(field_type) | OmType::Enum(field_type) = field.ty {
                    visit(field_type, definitions, visiting, complete)?;
                }
            }
        } else if let Some(definition) = definitions.enums.get(&id) {
            for variant in &definition.variants {
                for field in &variant.fields {
                    if let OmType::Struct(field_type) | OmType::Enum(field_type) = field.ty {
                        visit(field_type, definitions, visiting, complete)?;
                    }
                }
            }
        } else {
            unreachable!("type existence was validated before cycle detection");
        }
        visiting.remove(&id);
        complete.insert(id);
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for id in definitions
        .structs
        .keys()
        .chain(definitions.enums.keys())
        .copied()
    {
        visit(id, definitions, &mut visiting, &mut complete)?;
    }
    Ok(())
}

fn validate_nominal_type(
    ty: OmType,
    definitions: &Definitions<'_>,
    function: &str,
) -> Result<(), LowerError> {
    if let OmType::Struct(id) | OmType::SharedRef(id) = ty
        && !definitions.structs.contains_key(&id)
    {
        return Err(LowerError::function(
            function,
            format!("local type references missing structure @{id}"),
        ));
    }
    if let OmType::Enum(id) = ty
        && !definitions.enums.contains_key(&id)
    {
        return Err(LowerError::function(
            function,
            format!("local type references missing enum @{id}"),
        ));
    }
    Ok(())
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
